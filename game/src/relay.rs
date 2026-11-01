//! The game's side of the relay server (DESIGN.md sections 6 and 8).
//!
//! A raw connection to the server (R6's WebSocket transport, or anything
//! else that implements rewind's `Transport`) carries `ClientMsg` and
//! `ServerMsg`. Two stages:
//!
//! 1. **Lobby** (`RelayClient`): send `CreateRoom` or `JoinRoom`, then wait
//!    for `RoomCreated` and `MatchStart { player_index, seed }`.
//! 2. **Match** (`RelayTransport`): wrap every rollback message in
//!    `ClientMsg::Relay` and unwrap `ServerMsg::Relay`. It is itself a
//!    `Transport`, so `P2PSession` uses it without knowing about the relay.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use rewind::Transport;
use serde::{Deserialize, Serialize};

// A copy of `server/src/protocol.rs` (FROZEN: both authors agreed on it).
// The game can't depend on the server crate (it would pull tokio and axum
// into the browser build), so the two copies are kept identical by the
// test `copies_match_the_server_byte_for_byte`, which compares the encoded
// bytes of every variant with the server's own types.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMsg {
    CreateRoom,
    JoinRoom { code: String },
    Relay(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMsg {
    RoomCreated { code: String },
    MatchStart { player_index: u8, seed: u64 },
    Relay(Vec<u8>),
    PeerLeft,
}

/// What the lobby tells the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyEvent {
    /// Our room exists; show the code so the other player can join.
    RoomCreated { code: String },
    /// Both players are in. Start an `OnlineMatch` with these.
    MatchStart { player_index: u8, seed: u64 },
    /// The other player left (or the room expired) before the match began.
    PeerLeft,
}

fn send_msg<T: Transport>(socket: &mut T, msg: &ClientMsg) {
    // Serializing a String or bytes into a Vec<u8> can't fail.
    let bytes = bincode::serialize(msg).expect("ClientMsg always serializes");
    socket.send(&bytes);
}

/// Stage 1: the lobby.
pub struct RelayClient<T: Transport> {
    socket: T,
    /// Relay bytes that arrived in the same poll as (or after) `MatchStart`.
    /// The peer may start sending inputs immediately, so these belong to the
    /// match and are handed to the `RelayTransport`.
    early_relay: VecDeque<Vec<u8>>,
}

impl<T: Transport> RelayClient<T> {
    pub fn new(socket: T) -> RelayClient<T> {
        RelayClient {
            socket,
            early_relay: VecDeque::new(),
        }
    }

    pub fn create_room(&mut self) {
        send_msg(&mut self.socket, &ClientMsg::CreateRoom);
    }

    pub fn join_room(&mut self, code: &str) {
        let code = code.to_owned();
        send_msg(&mut self.socket, &ClientMsg::JoinRoom { code });
    }

    /// Everything the server said since the last poll.
    pub fn poll(&mut self) -> Vec<LobbyEvent> {
        let mut events = Vec::new();
        for bytes in self.socket.poll() {
            // The server is ours; a message we can't decode is a bug or a
            // version mismatch, and skipping it is the safest reaction.
            let Ok(msg) = bincode::deserialize::<ServerMsg>(&bytes) else {
                continue;
            };
            match msg {
                ServerMsg::RoomCreated { code } => events.push(LobbyEvent::RoomCreated { code }),
                ServerMsg::MatchStart { player_index, seed } => {
                    events.push(LobbyEvent::MatchStart { player_index, seed })
                }
                ServerMsg::Relay(payload) => self.early_relay.push_back(payload),
                ServerMsg::PeerLeft => events.push(LobbyEvent::PeerLeft),
            }
        }
        events
    }

    /// Stage 2: after `MatchStart`, turn the connection into the session's
    /// transport. Also returns a flag that turns true when the server says
    /// the other player left (`P2PSession` owns the transport, so the game
    /// watches the flag instead).
    pub fn into_match(self) -> (RelayTransport<T>, PeerLeft) {
        let peer_left = PeerLeft(Rc::new(Cell::new(false)));
        let transport = RelayTransport {
            socket: self.socket,
            inbox: self.early_relay,
            peer_left: peer_left.clone(),
        };
        (transport, peer_left)
    }
}

/// Shared "the other player left" flag. `Rc<Cell<bool>>`: shared between the
/// transport (which sets it) and the game (which reads it), single-threaded.
#[derive(Clone)]
pub struct PeerLeft(Rc<Cell<bool>>);

impl PeerLeft {
    pub fn get(&self) -> bool {
        self.0.get()
    }
}

/// Stage 2: rollback messages wrapped in the relay's envelope.
pub struct RelayTransport<T: Transport> {
    socket: T,
    inbox: VecDeque<Vec<u8>>,
    peer_left: PeerLeft,
}

impl<T: Transport> Transport for RelayTransport<T> {
    fn send(&mut self, bytes: &[u8]) {
        send_msg(&mut self.socket, &ClientMsg::Relay(bytes.to_vec()));
    }

    fn poll(&mut self) -> Vec<Vec<u8>> {
        for bytes in self.socket.poll() {
            match bincode::deserialize::<ServerMsg>(&bytes) {
                Ok(ServerMsg::Relay(payload)) => self.inbox.push_back(payload),
                Ok(ServerMsg::PeerLeft) => self.peer_left.0.set(true),
                // Lobby messages don't come after MatchStart; ignore strays.
                _ => {}
            }
        }
        self.inbox.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::Bot;
    use crate::online::OnlineMatch;
    use rewind::{memory_pair, MemoryTransport};
    use riposte_server::protocol as server;
    use riposte_server::rooms::{ConnId, Rooms};
    use std::time::Instant;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    #[test]
    fn copies_match_the_server_byte_for_byte() {
        let pairs = [
            (ClientMsg::CreateRoom, server::ClientMsg::CreateRoom),
            (
                ClientMsg::JoinRoom {
                    code: "ABCD".into(),
                },
                server::ClientMsg::JoinRoom {
                    code: "ABCD".into(),
                },
            ),
            (
                ClientMsg::Relay(vec![1, 2, 3]),
                server::ClientMsg::Relay(vec![1, 2, 3]),
            ),
        ];
        for (ours, theirs) in pairs {
            assert_eq!(
                bincode::serialize(&ours).unwrap(),
                bincode::serialize(&theirs).unwrap()
            );
        }
        let pairs = [
            (
                ServerMsg::RoomCreated {
                    code: "WXYZ".into(),
                },
                server::ServerMsg::RoomCreated {
                    code: "WXYZ".into(),
                },
            ),
            (
                ServerMsg::MatchStart {
                    player_index: 1,
                    seed: 0xDEAD_BEEF,
                },
                server::ServerMsg::MatchStart {
                    player_index: 1,
                    seed: 0xDEAD_BEEF,
                },
            ),
            (
                ServerMsg::Relay(vec![9, 8]),
                server::ServerMsg::Relay(vec![9, 8]),
            ),
            (ServerMsg::PeerLeft, server::ServerMsg::PeerLeft),
        ];
        for (ours, theirs) in pairs {
            assert_eq!(
                bincode::serialize(&ours).unwrap(),
                bincode::serialize(&theirs).unwrap()
            );
        }
    }

    /// The real server's room logic (`riposte_server::rooms::Rooms`) run
    /// in-process: the same code the deployed relay runs, minus the sockets.
    /// Each client is one end of an in-memory pair; the relay holds the
    /// other end and moves messages between the wire and `Rooms`.
    struct InProcessRelay {
        rooms: Rooms,
        clients: Vec<(
            ConnId,
            MemoryTransport,
            UnboundedReceiver<server::ServerMsg>,
        )>,
    }

    impl InProcessRelay {
        fn new() -> InProcessRelay {
            InProcessRelay {
                rooms: Rooms::with_seed(12345),
                clients: Vec::new(),
            }
        }

        /// A new connection: returns the client's end of the wire.
        fn connect(&mut self) -> MemoryTransport {
            let (client_end, relay_end) = memory_pair();
            let (_, outbox_rx) = unbounded_channel::<server::ServerMsg>();
            let conn = self.clients.len() as ConnId;
            self.clients.push((conn, relay_end, outbox_rx));
            client_end
        }

        /// Handle everything the clients sent, then deliver what the rooms
        /// want to send back. What the WebSocket handler does, once.
        fn pump(&mut self) {
            let now = Instant::now();
            for i in 0..self.clients.len() {
                let conn = self.clients[i].0;
                for bytes in self.clients[i].1.poll() {
                    let msg: server::ClientMsg = bincode::deserialize(&bytes).unwrap();
                    match msg {
                        // Entering a room hands the room a sender for this
                        // connection's outbox; we keep the receiving end.
                        server::ClientMsg::CreateRoom => {
                            let (tx, rx) = unbounded_channel();
                            self.clients[i].2 = rx;
                            self.rooms.create_room(conn, tx, now).unwrap();
                        }
                        server::ClientMsg::JoinRoom { code } => {
                            let (tx, rx) = unbounded_channel();
                            self.clients[i].2 = rx;
                            self.rooms.join_room(conn, tx, &code, now).unwrap();
                        }
                        server::ClientMsg::Relay(payload) => {
                            self.rooms.relay(conn, payload, now);
                        }
                    }
                }
            }
            for (_, wire, outbox) in &mut self.clients {
                while let Ok(msg) = outbox.try_recv() {
                    wire.send(&bincode::serialize(&msg).unwrap());
                }
            }
        }

        fn disconnect(&mut self, conn: ConnId) {
            self.rooms.leave(conn);
            self.pump();
        }
    }

    /// Create a room on one client and join it from another, through the
    /// real room logic. Returns both clients and their MatchStart values.
    fn lobby(relay: &mut InProcessRelay) -> [(RelayClient<MemoryTransport>, u8, u64); 2] {
        let mut host = RelayClient::new(relay.connect());
        let mut guest = RelayClient::new(relay.connect());

        host.create_room();
        relay.pump();
        let code = match host.poll().as_slice() {
            [LobbyEvent::RoomCreated { code }] => code.clone(),
            other => panic!("expected RoomCreated, got {other:?}"),
        };

        // Typed in lowercase, as a person might.
        guest.join_room(&code.to_lowercase());
        relay.pump();
        let start = |client: &mut RelayClient<MemoryTransport>| match client.poll().as_slice() {
            [LobbyEvent::MatchStart { player_index, seed }] => (*player_index, *seed),
            other => panic!("expected MatchStart, got {other:?}"),
        };
        let (hi, hs) = start(&mut host);
        let (gi, gs) = start(&mut guest);
        [(host, hi, hs), (guest, gi, gs)]
    }

    #[test]
    fn create_and_join_through_the_real_room_logic() {
        let mut relay = InProcessRelay::new();
        let [(_, host_index, host_seed), (_, guest_index, guest_seed)] = lobby(&mut relay);
        assert_eq!((host_index, guest_index), (0, 1));
        assert_eq!(host_seed, guest_seed, "both players must get the same seed");
    }

    #[test]
    fn full_online_match_over_the_relay_stays_in_sync() {
        // Lobby, then two OnlineMatches whose rollback messages travel
        // through the real room logic, bot vs bot for 20 s.
        let mut relay = InProcessRelay::new();
        let [(host, hi, seed), (guest, gi, _)] = lobby(&mut relay);
        let (host_wire, _) = host.into_match();
        let (guest_wire, _) = guest.into_match();
        let mut a = OnlineMatch::new(hi as usize, seed, host_wire);
        let mut b = OnlineMatch::new(gi as usize, seed, guest_wire);
        let mut bots = [Bot::new(1), Bot::new(2)];

        for _ in 0..1200 {
            let ia = bots[0].input(0, &a.game);
            let ib = bots[1].input(1, &b.game);
            a.tick(ia);
            b.tick(ib);
            relay.pump();
        }
        // Idle until every input has crossed the relay.
        for _ in 0..30 {
            a.tick(0);
            b.tick(0);
            relay.pump();
        }
        assert_eq!(a.game, b.game);
        assert_eq!(a.game.frame, 1230);
    }

    #[test]
    fn peer_leaving_mid_match_is_reported() {
        let mut relay = InProcessRelay::new();
        let [(host, hi, seed), (guest, _, _)] = lobby(&mut relay);
        let (host_wire, host_saw_leave) = host.into_match();
        let mut a = OnlineMatch::new(hi as usize, seed, host_wire);
        drop(guest);
        a.tick(0);
        assert!(!host_saw_leave.get());

        relay.disconnect(1); // the guest's connection id
        a.tick(0);
        assert!(host_saw_leave.get());
    }

    #[test]
    fn inputs_sent_right_after_match_start_are_not_lost() {
        // The host learns about the match first and sends a rollback
        // message before the guest has even polled its MatchStart. That
        // message arrives in the same poll as MatchStart and must reach the
        // guest's session.
        let mut relay = InProcessRelay::new();
        let mut host = RelayClient::new(relay.connect());
        let mut guest = RelayClient::new(relay.connect());
        host.create_room();
        relay.pump();
        let LobbyEvent::RoomCreated { code } = host.poll().remove(0) else {
            panic!("no room");
        };
        guest.join_room(&code);
        relay.pump();
        host.poll();
        let (mut host_wire, _) = host.into_match();
        host_wire.send(b"first input");
        relay.pump();

        assert!(matches!(guest.poll()[..], [LobbyEvent::MatchStart { .. }]));
        let (mut guest_wire, _) = guest.into_match();
        assert_eq!(guest_wire.poll(), vec![b"first input".to_vec()]);
    }
}
