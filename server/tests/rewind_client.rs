//! The server driven by rewind's own `WebSocketTransport` (R6), end to end:
//! real server, real sockets, the same client code the game will use.

use std::time::{Duration, Instant};

use rewind::{LobbyEvent, Transport, WebSocketTransport};
use riposte_server::{app, AppState};

/// Starts a server on a free port in a background thread with its own
/// runtime, because the client transport is blocking, not async.
fn start_server() -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, app(AppState::new())).await.unwrap();
        });
    });
    format!("ws://{}/ws", rx.recv().unwrap())
}

/// Keep asking for lobby events until one arrives or five seconds pass.
fn next_lobby_event(t: &mut WebSocketTransport) -> LobbyEvent {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(event) = t.lobby_events().into_iter().next() {
            return event;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no lobby event within 5 seconds");
}

fn next_relayed(t: &mut WebSocketTransport) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(bytes) = t.poll().into_iter().next() {
            return bytes;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no relayed bytes within 5 seconds");
}

#[test]
fn two_transports_pair_up_and_relay_bytes_both_ways() {
    let url = start_server();
    let mut host = WebSocketTransport::connect(&url).unwrap();
    let mut guest = WebSocketTransport::connect(&url).unwrap();

    host.create_room();
    let LobbyEvent::RoomCreated { code } = next_lobby_event(&mut host) else {
        panic!("expected RoomCreated");
    };
    guest.join_room(&code);

    let LobbyEvent::MatchStart {
        player_index: host_index,
        seed: host_seed,
    } = next_lobby_event(&mut host)
    else {
        panic!("host expected MatchStart");
    };
    let LobbyEvent::MatchStart {
        player_index: guest_index,
        seed: guest_seed,
    } = next_lobby_event(&mut guest)
    else {
        panic!("guest expected MatchStart");
    };
    assert_ne!(host_index, guest_index);
    assert_eq!(host_seed, guest_seed);

    host.send(&[1, 2, 3]);
    assert_eq!(next_relayed(&mut guest), vec![1, 2, 3]);
    guest.send(&[9]);
    assert_eq!(next_relayed(&mut host), vec![9]);
}

#[test]
fn leaving_tells_the_other_player() {
    let url = start_server();
    let mut host = WebSocketTransport::connect(&url).unwrap();
    let mut guest = WebSocketTransport::connect(&url).unwrap();
    host.create_room();
    let LobbyEvent::RoomCreated { code } = next_lobby_event(&mut host) else {
        panic!("expected RoomCreated");
    };
    guest.join_room(&code);
    next_lobby_event(&mut host); // MatchStart
    next_lobby_event(&mut guest);

    drop(guest);
    assert_eq!(next_lobby_event(&mut host), LobbyEvent::PeerLeft);
}

/// A game whose state depends on every input and on which player sent it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Tally {
    frame: u32,
    value: u64,
}

impl rewind::RollbackGame for Tally {
    type Input = u16;
    type State = Tally;

    fn save(&self) -> Tally {
        self.clone()
    }
    fn load(&mut self, state: &Tally) {
        *self = state.clone();
    }
    fn advance(&mut self, inputs: [u16; 2]) {
        let mixed = inputs[0] as u64 * 7 + inputs[1] as u64 * 13 + 1;
        self.value = self.value.wrapping_mul(31).wrapping_add(mixed);
        self.frame += 1;
    }
    fn checksum(&self) -> u64 {
        self.value ^ self.frame as u64
    }
}

#[test]
fn two_sessions_play_through_the_relay_without_desync() {
    use rewind::{Config, Event, P2PSession};

    let url = start_server();
    let mut host = WebSocketTransport::connect(&url).unwrap();
    let mut guest = WebSocketTransport::connect(&url).unwrap();
    host.create_room();
    let LobbyEvent::RoomCreated { code } = next_lobby_event(&mut host) else {
        panic!("expected RoomCreated");
    };
    guest.join_room(&code);
    let LobbyEvent::MatchStart { player_index, .. } = next_lobby_event(&mut host) else {
        panic!("expected MatchStart");
    };
    next_lobby_event(&mut guest);

    let config = Config {
        checksum_interval: 10,
        ..Config::default()
    };
    let (host_player, guest_player) = (player_index as usize, 1 - player_index as usize);
    let mut a = P2PSession::<Tally, _>::new(host_player, host, config);
    let mut b = P2PSession::<Tally, _>::new(guest_player, guest, config);
    let (mut ga, mut gb) = (Tally::default(), Tally::default());

    let mut events = Vec::new();
    for t in 0..200u16 {
        a.add_local_input(t / 3 % 11);
        b.add_local_input(t / 4 % 7);
        events.extend(a.tick(&mut ga));
        events.extend(b.tick(&mut gb));
        // Real sockets deliver in the background; give them a moment.
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!events.iter().any(|e| matches!(e, Event::Desync { .. })));
    assert!(a.current_frame() > 100 && b.current_frame() > 100);
}
