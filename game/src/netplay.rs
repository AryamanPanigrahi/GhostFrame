//! Online play (I1): from "Create room" / "Join room" in the menu to a
//! running `OnlineMatch` over Ram's `WebSocketTransport` (R6).
//!
//! Two stages:
//! - `Lobby`: connected to the relay, waiting for the room code and then for
//!   `MatchStart { player_index, seed }`.
//! - `OnlineScreen`: the match itself, plus what the session reports (R5's
//!   stalls and desyncs) and the relay's `PeerLeft`.
//!
//! Both are generic over the socket (`ByteSocket`) so tests can use a fake
//! one; the real game uses the default (tungstenite on desktop, quad-net in
//! the browser).

use rewind::{ByteSocket, Event, Frame, LobbyEvent, WebSocketTransport};

use crate::driver::TickDriver;
use crate::online::OnlineMatch;
use crate::sim::Input;

/// Where the relay runs if `RIPOSTE_SERVER` isn't set: the same machine,
/// `cargo run -p riposte-server`'s default port.
const DEFAULT_SERVER: &str = "ws://127.0.0.1:8080/ws";

/// The relay's address. On desktop, `RIPOSTE_SERVER` overrides it, e.g.
/// `ws://192.168.1.20:8080/ws` to reach a server on another laptop.
#[cfg(not(target_arch = "wasm32"))]
pub fn server_url() -> String {
    std::env::var("RIPOSTE_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.to_owned())
}

/// In the browser there are no environment variables; the page decides (see
/// `relayUrl` in `web/index.html`: `?server=` override, else the page's own
/// address) and hands the bytes over through two functions it adds to the
/// module's imports.
#[cfg(target_arch = "wasm32")]
pub fn server_url() -> String {
    extern "C" {
        fn riposte_server_url_len() -> u32;
        fn riposte_server_url_copy(ptr: *mut u8, capacity: u32) -> u32;
    }
    // SAFETY: both functions are provided by the page before the game
    // starts, and `copy` writes at most `capacity` bytes into our buffer.
    let (len, bytes) = unsafe {
        let len = riposte_server_url_len() as usize;
        let mut buf = vec![0u8; len];
        let copied = riposte_server_url_copy(buf.as_mut_ptr(), len as u32) as usize;
        buf.truncate(copied);
        (len, buf)
    };
    match String::from_utf8(bytes) {
        Ok(url) if len > 0 => url,
        _ => DEFAULT_SERVER.to_owned(),
    }
}

/// How long to wait for `MatchStart` after joining. The relay doesn't answer
/// a wrong or full room code (it closes the connection, since the frozen
/// `ServerMsg` has no error variant), so silence is the only signal.
const JOIN_TIMEOUT_MICROS: u64 = 5_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LobbyStatus {
    /// Asked for a room; waiting for its code.
    CreatingRoom,
    /// Room exists; waiting for the other player to join with this code.
    WaitingForOpponent { code: String },
    /// Asked to join this room; waiting for `MatchStart`.
    Joining { code: String },
    /// Nothing more will happen; Esc goes back to the menu.
    Failed { reason: String },
}

pub struct Lobby<S: ByteSocket> {
    /// `None` once it has been handed to the match.
    transport: Option<WebSocketTransport<S>>,
    pub status: LobbyStatus,
    waited_micros: u64,
}

impl<S: ByteSocket> Lobby<S> {
    pub fn create(mut transport: WebSocketTransport<S>) -> Lobby<S> {
        transport.create_room();
        Lobby {
            transport: Some(transport),
            status: LobbyStatus::CreatingRoom,
            waited_micros: 0,
        }
    }

    pub fn join(mut transport: WebSocketTransport<S>, code: &str) -> Lobby<S> {
        transport.join_room(code);
        Lobby {
            transport: Some(transport),
            status: LobbyStatus::Joining {
                code: code.to_owned(),
            },
            waited_micros: 0,
        }
    }

    /// Call every frame. Returns the match as soon as both players are in.
    pub fn update(&mut self, elapsed_micros: u64) -> Option<OnlineScreen<S>> {
        let transport = self.transport.as_mut()?;
        for event in transport.lobby_events() {
            match event {
                LobbyEvent::RoomCreated { code } => {
                    self.status = LobbyStatus::WaitingForOpponent { code };
                }
                LobbyEvent::MatchStart { player_index, seed } => {
                    let transport = self.transport.take()?;
                    return Some(OnlineScreen::new(player_index as usize, seed, transport));
                }
                LobbyEvent::PeerLeft => {
                    self.fail("The other player left before the match started.");
                }
                LobbyEvent::ConnectionLost => {
                    // A wrong or full code makes the relay close the
                    // connection, so while joining this means "no such room".
                    let reason = match &self.status {
                        LobbyStatus::Joining { code } => no_room(code),
                        _ => "Lost the connection to the server.".to_owned(),
                    };
                    self.fail(&reason);
                    return None;
                }
            }
        }

        if let LobbyStatus::Joining { code } = &self.status {
            self.waited_micros += elapsed_micros;
            if self.waited_micros > JOIN_TIMEOUT_MICROS {
                let reason = no_room(code);
                self.fail(&reason);
            }
        }
        None
    }

    fn fail(&mut self, reason: &str) {
        self.status = LobbyStatus::Failed {
            reason: reason.to_owned(),
        };
        // Close the connection: nothing more can happen in this lobby.
        self.transport = None;
    }

    /// Text for the placeholder lobby screen.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = match &self.status {
            LobbyStatus::CreatingRoom => vec!["Creating a room...".to_owned()],
            LobbyStatus::WaitingForOpponent { code } => vec![
                format!("Room code: {code}"),
                "Waiting for the other player to join with this code...".to_owned(),
            ],
            LobbyStatus::Joining { code } => vec![format!("Joining room {code}...")],
            LobbyStatus::Failed { reason } => reason.lines().map(str::to_owned).collect(),
        };
        lines.push(String::new());
        lines.push("Esc: back to the menu".to_owned());
        lines
    }
}

fn no_room(code: &str) -> String {
    format!("No answer for room {code}.\nIt may not exist, or it's full.")
}

/// A running online match and what the session has reported about it.
pub struct OnlineScreen<S: ByteSocket> {
    pub online: OnlineMatch<WebSocketTransport<S>>,
    local_player: usize,
    driver: TickDriver,
    /// The first frame whose checksum differed from the peer's (R5). Never
    /// expected; if it happens it's a determinism bug.
    pub desync: Option<Frame>,
    /// The relay said the other player is gone. The match stops.
    pub peer_left: bool,
    /// Our own connection to the relay closed. The match stops: without it
    /// the session would stall forever waiting for the other player.
    pub connection_lost: bool,
}

impl<S: ByteSocket> OnlineScreen<S> {
    fn new(local_player: usize, seed: u64, transport: WebSocketTransport<S>) -> OnlineScreen<S> {
        OnlineScreen {
            online: OnlineMatch::new(local_player, seed, transport),
            local_player,
            driver: TickDriver::default(),
            desync: None,
            peer_left: false,
            connection_lost: false,
        }
    }

    /// One render frame: run the ticks that are due with this machine's
    /// input, and collect what the session and the relay report.
    pub fn update(&mut self, elapsed_micros: u64, local_input: Input) {
        if self.peer_left || self.connection_lost {
            return;
        }
        let (ticks, inputs) = self.driver.frame(elapsed_micros, [local_input, 0]);
        for _ in 0..ticks {
            for event in self.online.tick(inputs[0]) {
                match event {
                    Event::Desync { frame } => {
                        self.desync.get_or_insert(frame);
                    }
                    Event::Disconnected => self.peer_left = true,
                    // Counted by the session; the overlay shows the total.
                    Event::Stalled => {}
                }
            }
        }
        // The session owns the transport, so the relay's PeerLeft (and a
        // dropped connection) is read through it (see
        // `P2PSession::transport_mut`).
        for event in self.online.transport_mut().lobby_events() {
            match event {
                LobbyEvent::PeerLeft => self.peer_left = true,
                LobbyEvent::ConnectionLost => self.connection_lost = true,
                _ => {}
            }
        }
    }

    /// Debug text for the screen.
    pub fn overlay(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "ONLINE  you are P{}   rolled back {} frames (deepest {})   stalled {} ticks",
                self.local_player + 1,
                self.online.rollback_frames(),
                self.online.deepest_rollback(),
                self.online.stalled_ticks()
            ),
            "You: either key set (WASD+F/G or arrows+K/L)   Esc: leave".to_owned(),
        ];
        if let Some(frame) = self.desync {
            lines.push(format!(
                "DESYNC at frame {frame}: the two games differ (a determinism bug)"
            ));
        }
        if self.peer_left {
            lines.push("The other player left. Esc: back to the menu".to_owned());
        } else if self.connection_lost {
            lines.push("Lost the connection to the server. Esc: back to the menu".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rewind::PeerMsg;
    use serde::Serialize;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    /// The relay's side of the protocol, as the client sees it on the wire.
    /// A copy of the frozen `ServerMsg` (see `server/src/protocol.rs`).
    #[derive(Serialize)]
    enum ServerMsg {
        RoomCreated { code: String },
        MatchStart { player_index: u8, seed: u64 },
        Relay(Vec<u8>),
        PeerLeft,
    }

    /// A socket the test controls: whatever the test pushes into `incoming`
    /// is what the relay "sent".
    #[derive(Clone, Default)]
    struct FakeSocket {
        incoming: Rc<RefCell<VecDeque<Vec<u8>>>>,
        closed: Rc<RefCell<bool>>,
    }

    impl FakeSocket {
        fn deliver(&self, msg: &ServerMsg) {
            let bytes = bincode::serialize(msg).unwrap();
            self.incoming.borrow_mut().push_back(bytes);
        }
    }

    impl ByteSocket for FakeSocket {
        fn send_bytes(&mut self, _data: &[u8]) {}
        fn try_recv(&mut self) -> Option<Vec<u8>> {
            self.incoming.borrow_mut().pop_front()
        }
        fn is_open(&self) -> bool {
            true
        }
        fn is_closed(&self) -> bool {
            *self.closed.borrow()
        }
    }

    fn connected() -> (WebSocketTransport<FakeSocket>, FakeSocket) {
        let socket = FakeSocket::default();
        (WebSocketTransport::with_socket(socket.clone()), socket)
    }

    const FRAME: u64 = 16_667;

    #[test]
    fn create_shows_the_code_then_starts_the_match() {
        let (transport, relay) = connected();
        let mut lobby = Lobby::create(transport);
        assert_eq!(lobby.status, LobbyStatus::CreatingRoom);

        relay.deliver(&ServerMsg::RoomCreated {
            code: "QWER".into(),
        });
        assert!(lobby.update(FRAME).is_none());
        assert_eq!(
            lobby.status,
            LobbyStatus::WaitingForOpponent {
                code: "QWER".into()
            }
        );
        assert!(lobby.lines()[0].contains("QWER"));

        relay.deliver(&ServerMsg::MatchStart {
            player_index: 0,
            seed: 99,
        });
        let screen = lobby.update(FRAME).expect("match should start");
        assert_eq!(screen.local_player, 0);
        assert_eq!(screen.online.game, crate::sim::GameState::new(99));
    }

    #[test]
    fn joining_gets_player_two() {
        let (transport, relay) = connected();
        let mut lobby = Lobby::join(transport, "ABCD");
        relay.deliver(&ServerMsg::MatchStart {
            player_index: 1,
            seed: 7,
        });
        let screen = lobby.update(FRAME).unwrap();
        assert_eq!(screen.local_player, 1);
    }

    #[test]
    fn join_with_no_answer_times_out() {
        let (transport, _relay) = connected();
        let mut lobby = Lobby::join(transport, "ZZZZ");
        for _ in 0..(JOIN_TIMEOUT_MICROS / FRAME) {
            lobby.update(FRAME);
        }
        assert!(matches!(lobby.status, LobbyStatus::Joining { .. }));
        for _ in 0..5 {
            lobby.update(FRAME);
        }
        assert!(matches!(lobby.status, LobbyStatus::Failed { .. }));
        assert!(lobby.lines()[0].contains("ZZZZ"));
    }

    #[test]
    fn opponent_leaving_before_the_start_fails_the_lobby() {
        let (transport, relay) = connected();
        let mut lobby = Lobby::create(transport);
        relay.deliver(&ServerMsg::RoomCreated {
            code: "QWER".into(),
        });
        relay.deliver(&ServerMsg::PeerLeft);
        assert!(lobby.update(FRAME).is_none());
        assert!(matches!(lobby.status, LobbyStatus::Failed { .. }));
    }

    #[test]
    fn opponent_leaving_mid_match_stops_it() {
        let (transport, relay) = connected();
        let mut lobby = Lobby::join(transport, "ABCD");
        relay.deliver(&ServerMsg::MatchStart {
            player_index: 1,
            seed: 7,
        });
        let mut screen = lobby.update(FRAME).unwrap();
        screen.update(FRAME, 0);
        assert!(!screen.peer_left);

        relay.deliver(&ServerMsg::PeerLeft);
        screen.update(FRAME, 0);
        assert!(screen.peer_left);
        let frame = screen.online.game.frame;
        screen.update(FRAME * 10, 0);
        assert_eq!(screen.online.game.frame, frame, "a stopped match stays put");
    }

    #[test]
    fn a_closed_connection_while_joining_means_no_such_room() {
        let (transport, relay) = connected();
        let mut lobby = Lobby::join(transport, "ZZZZ");
        *relay.closed.borrow_mut() = true;
        assert!(lobby.update(FRAME).is_none());
        assert!(matches!(lobby.status, LobbyStatus::Failed { .. }));
        assert!(lobby.lines()[0].contains("ZZZZ"), "without the 5 s wait");
    }

    #[test]
    fn losing_the_relay_while_waiting_fails_the_lobby() {
        let (transport, relay) = connected();
        let mut lobby = Lobby::create(transport);
        relay.deliver(&ServerMsg::RoomCreated {
            code: "QWER".into(),
        });
        *relay.closed.borrow_mut() = true;
        assert!(lobby.update(FRAME).is_none());
        assert!(lobby.lines()[0].contains("Lost the connection"));
    }

    #[test]
    fn losing_the_relay_mid_match_stops_it_with_a_message() {
        let (transport, relay) = connected();
        let mut lobby = Lobby::join(transport, "ABCD");
        relay.deliver(&ServerMsg::MatchStart {
            player_index: 1,
            seed: 7,
        });
        let mut screen = lobby.update(FRAME).unwrap();
        screen.update(FRAME, 0);
        assert!(!screen.connection_lost);

        *relay.closed.borrow_mut() = true;
        screen.update(FRAME, 0);
        assert!(screen.connection_lost);
        assert!(screen
            .overlay()
            .iter()
            .any(|l| l.contains("Lost the connection")));
        let frame = screen.online.game.frame;
        screen.update(FRAME * 10, 0);
        assert_eq!(screen.online.game.frame, frame, "a stopped match stays put");
    }

    #[test]
    fn peer_inputs_arriving_through_the_relay_reach_the_session() {
        // The peer (player 0) sends inputs for frames 0..30 straight away;
        // our session (player 1) must use them, not guesses.
        let (transport, relay) = connected();
        let mut lobby = Lobby::join(transport, "ABCD");
        relay.deliver(&ServerMsg::MatchStart {
            player_index: 1,
            seed: 7,
        });
        let peer = PeerMsg::<Input>::Inputs {
            start_frame: 0,
            inputs: vec![crate::sim::RIGHT; 30],
            ack_frame: 0,
        };
        relay.deliver(&ServerMsg::Relay(bincode::serialize(&peer).unwrap()));
        let mut screen = lobby.update(FRAME).unwrap();
        for _ in 0..20 {
            screen.update(FRAME, 0);
        }
        assert_eq!(
            screen.online.rollback_frames(),
            0,
            "inputs were known, no guesses to fix"
        );
        assert_eq!(screen.online.game.frame, 20);
    }
}
