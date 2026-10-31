//! Room bookkeeping: who is waiting, who is paired, who gets which bytes.
//!
//! This module knows nothing about sockets. Each connection is represented by
//! an id and a channel sender, so the logic can be unit tested without a
//! network.
//!
//! A connection lives and dies with its room: once the room is gone (peer left
//! or room idle), the connection is told `PeerLeft` and then closed by the
//! WebSocket handler. To play again, a client reconnects.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use crate::protocol::ServerMsg;

pub type ConnId = u64;
pub type Outbox = UnboundedSender<ServerMsg>;

/// Room codes skip I and O, which are easy to confuse with 1 and 0 when read
/// aloud or typed from a screenshot.
const CODE_LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const CODE_LEN: usize = 4;

/// A client broke the protocol. The handler closes the connection.
#[derive(Debug, PartialEq, Eq)]
pub enum RoomError {
    AlreadyInRoom,
    UnknownRoom,
    RoomFull,
}

struct Player {
    conn: ConnId,
    outbox: Outbox,
}

struct Room {
    host: Player,
    guest: Option<Player>,
    /// Updated on create, join and every relayed message.
    last_activity: Instant,
}

pub struct Rooms {
    rooms: HashMap<String, Room>,
    /// Which room each connection is in, so a Relay or a disconnect can find
    /// its room without scanning.
    room_of: HashMap<ConnId, String>,
    rng_state: u64,
}

impl Rooms {
    /// Seeds the code/seed generator from the OS. `RandomState` is the std
    /// library's randomly-keyed hasher state; hashing nothing with it gives an
    /// OS-random number without adding a `rand` dependency.
    pub fn new() -> Self {
        let os_random = RandomState::new().build_hasher().finish();
        Self::with_seed(os_random)
    }

    /// Fixed seed, for tests.
    pub fn with_seed(seed: u64) -> Self {
        Rooms {
            rooms: HashMap::new(),
            room_of: HashMap::new(),
            // xorshift gets stuck at 0 forever, so never start there.
            rng_state: if seed == 0 { 1 } else { seed },
        }
    }

    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    /// xorshift64. The server is not part of the simulation, so there is no
    /// determinism requirement here, only "different every match".
    fn next_u64(&mut self) -> u64 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        x
    }

    fn new_code(&mut self) -> String {
        loop {
            let mut code = String::with_capacity(CODE_LEN);
            for _ in 0..CODE_LEN {
                let i = (self.next_u64() % CODE_LETTERS.len() as u64) as usize;
                code.push(CODE_LETTERS[i] as char);
            }
            // 24^4 is only ~330k codes, so a collision with a live room is
            // possible. Try again until the code is free.
            if !self.rooms.contains_key(&code) {
                return code;
            }
        }
    }

    pub fn create_room(
        &mut self,
        conn: ConnId,
        outbox: Outbox,
        now: Instant,
    ) -> Result<String, RoomError> {
        if self.room_of.contains_key(&conn) {
            return Err(RoomError::AlreadyInRoom);
        }
        let code = self.new_code();
        let _ = outbox.send(ServerMsg::RoomCreated { code: code.clone() });
        self.rooms.insert(
            code.clone(),
            Room {
                host: Player { conn, outbox },
                guest: None,
                last_activity: now,
            },
        );
        self.room_of.insert(conn, code.clone());
        Ok(code)
    }

    pub fn join_room(
        &mut self,
        conn: ConnId,
        outbox: Outbox,
        code: &str,
        now: Instant,
    ) -> Result<(), RoomError> {
        if self.room_of.contains_key(&conn) {
            return Err(RoomError::AlreadyInRoom);
        }
        // Codes are uppercase; accept what a person typed in any case.
        let code = code.to_ascii_uppercase();
        // Draw the seed first: `get_mut` below borrows `self.rooms`.
        let seed = self.next_u64();
        let room = self.rooms.get_mut(&code).ok_or(RoomError::UnknownRoom)?;
        if room.guest.is_some() {
            return Err(RoomError::RoomFull);
        }
        // The same seed goes to both players so their games start identically.
        let _ = room.host.outbox.send(ServerMsg::MatchStart {
            player_index: 0,
            seed,
        });
        let _ = outbox.send(ServerMsg::MatchStart {
            player_index: 1,
            seed,
        });
        room.guest = Some(Player { conn, outbox });
        room.last_activity = now;
        self.room_of.insert(conn, code);
        Ok(())
    }

    /// Forwards bytes to the other player, unchanged. Relay from a connection
    /// that has no room or no opponent yet is dropped: there is nobody to
    /// send it to, and it is harmless.
    pub fn relay(&mut self, conn: ConnId, bytes: Vec<u8>, now: Instant) {
        let Some(code) = self.room_of.get(&conn) else {
            return;
        };
        let Some(room) = self.rooms.get_mut(code) else {
            return;
        };
        let Some(guest) = &room.guest else {
            return;
        };
        let target = if room.host.conn == conn {
            &guest.outbox
        } else {
            &room.host.outbox
        };
        let _ = target.send(ServerMsg::Relay(bytes));
        room.last_activity = now;
    }

    /// A connection ended. Closes its room and tells the other player.
    /// Safe to call for a connection that is in no room (or whose room is
    /// already gone).
    pub fn leave(&mut self, conn: ConnId) {
        let Some(code) = self.room_of.remove(&conn) else {
            return;
        };
        if let Some(room) = self.rooms.remove(&code) {
            self.close_room(room, Some(conn));
        }
    }

    /// Deletes rooms with no activity for longer than `max_idle`. Returns how
    /// many were removed.
    pub fn sweep_idle(&mut self, now: Instant, max_idle: Duration) -> usize {
        let idle: Vec<String> = self
            .rooms
            .iter()
            .filter(|(_, room)| now.duration_since(room.last_activity) > max_idle)
            .map(|(code, _)| code.clone())
            .collect();
        for code in &idle {
            if let Some(room) = self.rooms.remove(code) {
                self.close_room(room, None);
            }
        }
        idle.len()
    }

    /// Removes both players from `room_of` and sends `PeerLeft` to everyone
    /// except `leaver` (who is already gone). The handler closes a connection
    /// after it forwards `PeerLeft`.
    fn close_room(&mut self, room: Room, leaver: Option<ConnId>) {
        for player in std::iter::once(room.host).chain(room.guest) {
            self.room_of.remove(&player.conn);
            if Some(player.conn) != leaver {
                let _ = player.outbox.send(ServerMsg::PeerLeft);
            }
        }
    }
}

impl Default for Rooms {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    fn conn() -> (Outbox, UnboundedReceiver<ServerMsg>) {
        unbounded_channel()
    }

    /// Creates a room with `host` (conn 1) and joins `guest` (conn 2).
    fn paired(
        rooms: &mut Rooms,
        now: Instant,
    ) -> (
        String,
        UnboundedReceiver<ServerMsg>,
        UnboundedReceiver<ServerMsg>,
    ) {
        let (host_tx, mut host_rx) = conn();
        let (guest_tx, guest_rx) = conn();
        let code = rooms.create_room(1, host_tx, now).unwrap();
        rooms.join_room(2, guest_tx, &code, now).unwrap();
        host_rx.try_recv().unwrap(); // RoomCreated
        (code, host_rx, guest_rx)
    }

    #[test]
    fn create_room_sends_a_four_letter_code() {
        let mut rooms = Rooms::with_seed(7);
        let (tx, mut rx) = conn();
        let code = rooms.create_room(1, tx, Instant::now()).unwrap();
        assert_eq!(code.len(), 4);
        assert!(code.bytes().all(|b| CODE_LETTERS.contains(&b)));
        assert_eq!(rx.try_recv().unwrap(), ServerMsg::RoomCreated { code });
    }

    #[test]
    fn join_pairs_players_with_indices_and_one_shared_seed() {
        let mut rooms = Rooms::with_seed(7);
        let (_, mut host_rx, mut guest_rx) = paired(&mut rooms, Instant::now());
        let host = host_rx.try_recv().unwrap();
        let guest = guest_rx.try_recv().unwrap();
        let (
            ServerMsg::MatchStart {
                player_index: 0,
                seed: s0,
            },
            ServerMsg::MatchStart {
                player_index: 1,
                seed: s1,
            },
        ) = (host, guest)
        else {
            panic!("expected MatchStart for index 0 and 1");
        };
        assert_eq!(s0, s1);
    }

    #[test]
    fn join_accepts_a_lowercase_code() {
        let mut rooms = Rooms::with_seed(7);
        let (host_tx, _host_rx) = conn();
        let (guest_tx, _guest_rx) = conn();
        let code = rooms.create_room(1, host_tx, Instant::now()).unwrap();
        let lower = code.to_ascii_lowercase();
        assert_eq!(rooms.join_room(2, guest_tx, &lower, Instant::now()), Ok(()));
    }

    #[test]
    fn join_unknown_or_full_room_is_an_error() {
        let mut rooms = Rooms::with_seed(7);
        let (code, _h, _g) = paired(&mut rooms, Instant::now());
        let (tx, _rx) = conn();
        assert_eq!(
            rooms.join_room(3, tx.clone(), "ZZZZ", Instant::now()),
            Err(RoomError::UnknownRoom)
        );
        assert_eq!(
            rooms.join_room(3, tx, &code, Instant::now()),
            Err(RoomError::RoomFull)
        );
    }

    #[test]
    fn a_connection_cannot_create_or_join_twice() {
        let mut rooms = Rooms::with_seed(7);
        let (tx, _rx) = conn();
        let code = rooms.create_room(1, tx.clone(), Instant::now()).unwrap();
        assert_eq!(
            rooms.create_room(1, tx.clone(), Instant::now()),
            Err(RoomError::AlreadyInRoom)
        );
        assert_eq!(
            rooms.join_room(1, tx, &code, Instant::now()),
            Err(RoomError::AlreadyInRoom)
        );
    }

    #[test]
    fn relay_forwards_unchanged_to_the_other_player_only() {
        let mut rooms = Rooms::with_seed(7);
        let (_, mut host_rx, mut guest_rx) = paired(&mut rooms, Instant::now());
        host_rx.try_recv().unwrap(); // MatchStart
        guest_rx.try_recv().unwrap(); // MatchStart

        rooms.relay(1, vec![1, 2, 3], Instant::now());
        rooms.relay(2, vec![9], Instant::now());

        assert_eq!(
            guest_rx.try_recv().unwrap(),
            ServerMsg::Relay(vec![1, 2, 3])
        );
        assert!(guest_rx.try_recv().is_err());
        assert_eq!(host_rx.try_recv().unwrap(), ServerMsg::Relay(vec![9]));
        assert!(host_rx.try_recv().is_err());
    }

    #[test]
    fn relay_before_the_match_starts_is_dropped() {
        let mut rooms = Rooms::with_seed(7);
        let (tx, mut rx) = conn();
        rooms.create_room(1, tx, Instant::now()).unwrap();
        rx.try_recv().unwrap(); // RoomCreated
        rooms.relay(1, vec![1], Instant::now());
        rooms.relay(99, vec![1], Instant::now()); // no room at all
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn leaving_tells_the_peer_and_closes_the_room() {
        let mut rooms = Rooms::with_seed(7);
        let (_, mut host_rx, mut guest_rx) = paired(&mut rooms, Instant::now());
        host_rx.try_recv().unwrap();
        guest_rx.try_recv().unwrap();

        rooms.leave(1);

        assert_eq!(guest_rx.try_recv().unwrap(), ServerMsg::PeerLeft);
        assert!(host_rx.try_recv().is_err()); // the leaver is not told
        assert_eq!(rooms.room_count(), 0);
        // The second player's own disconnect afterwards is a harmless no-op.
        rooms.leave(2);
        assert_eq!(rooms.room_count(), 0);
    }

    #[test]
    fn leaving_a_waiting_room_deletes_it() {
        let mut rooms = Rooms::with_seed(7);
        let (tx, _rx) = conn();
        let code = rooms.create_room(1, tx, Instant::now()).unwrap();
        rooms.leave(1);
        assert_eq!(rooms.room_count(), 0);
        let (tx2, _rx2) = conn();
        assert_eq!(
            rooms.join_room(2, tx2, &code, Instant::now()),
            Err(RoomError::UnknownRoom)
        );
    }

    #[test]
    fn sweep_removes_only_rooms_idle_past_the_limit() {
        let mut rooms = Rooms::with_seed(7);
        let start = Instant::now();
        let limit = Duration::from_secs(300);
        let (_, mut host_rx, mut guest_rx) = paired(&mut rooms, start);
        host_rx.try_recv().unwrap();
        guest_rx.try_recv().unwrap();
        let (tx, mut waiting_rx) = conn();
        rooms
            .create_room(3, tx, start + Duration::from_secs(50))
            .unwrap();
        waiting_rx.try_recv().unwrap(); // RoomCreated

        // Relay keeps the first room alive; the second has been quiet.
        rooms.relay(1, vec![0], start + Duration::from_secs(250));
        assert_eq!(rooms.sweep_idle(start + Duration::from_secs(400), limit), 1);
        assert_eq!(rooms.room_count(), 1);
        assert_eq!(waiting_rx.try_recv().unwrap(), ServerMsg::PeerLeft);

        // Nothing more happens, so the first room is now idle too.
        assert_eq!(rooms.sweep_idle(start + Duration::from_secs(600), limit), 1);
        assert_eq!(guest_rx.try_recv().unwrap(), ServerMsg::Relay(vec![0]));
        assert_eq!(guest_rx.try_recv().unwrap(), ServerMsg::PeerLeft);
        assert_eq!(host_rx.try_recv().unwrap(), ServerMsg::PeerLeft);
        assert_eq!(rooms.room_count(), 0);
    }

    #[test]
    fn room_codes_do_not_collide_with_live_rooms() {
        let mut rooms = Rooms::with_seed(7);
        let mut codes = Vec::new();
        for id in 0..500 {
            let (tx, _rx) = conn();
            codes.push(rooms.create_room(id, tx, Instant::now()).unwrap());
        }
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), 500);
    }
}
