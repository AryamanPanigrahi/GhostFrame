//! Real online play (I1): the lobby that talks to the relay, and the match
//! that runs once the relay says both players are in.
//!
//! Flow: the menu asks for `Create` or `Join`; `Lobby::connect` opens one
//! WebSocket to the relay and sends the request; each frame `Lobby::update`
//! reads the relay's news until `MatchStart` arrives; then `RemoteMatch`
//! takes over *the same transport*, because the relay ties a room to that
//! one connection.

use rewind::{Event, LobbyEvent, WebSocketTransport};

use crate::driver::TickDriver;
use crate::online::OnlineMatch;
use crate::sim::Input;

/// Where the relay runs by default: `cargo run -p riposte-server` on this
/// machine. Override with the `RIPOSTE_RELAY` environment variable (desktop
/// only; the browser build can't read environment variables).
pub const DEFAULT_RELAY_URL: &str = "ws://127.0.0.1:8080/ws";

pub fn relay_url() -> String {
    std::env::var("RIPOSTE_RELAY").unwrap_or_else(|_| DEFAULT_RELAY_URL.to_owned())
}

/// What the player asked the menu for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    Create,
    Join { code: String },
}

/// What the lobby learned this frame.
#[derive(Debug, PartialEq, Eq)]
pub enum LobbyOutcome {
    Waiting,
    /// Both players are in: build a `RemoteMatch` with these.
    Start {
        player_index: u8,
        seed: u64,
    },
    /// Show this message on the menu.
    Failed(String),
}

/// What the lobby knows, apart from the connection, so it can be tested
/// without a socket.
struct LobbyState {
    intent: Intent,
    /// The room code to show the friend, once the relay has made the room.
    room_code: Option<String>,
}

/// Waiting in a room for the match to start.
pub struct Lobby {
    /// `None` only after `take_transport` has handed it to the match.
    transport: Option<WebSocketTransport>,
    state: LobbyState,
}

impl Lobby {
    /// Open the connection and send the create or join request. On desktop
    /// this blocks until connected (or fails).
    pub fn connect(url: &str, intent: Intent) -> Result<Lobby, String> {
        let mut transport = WebSocketTransport::connect(url).map_err(|e| e.to_string())?;
        match &intent {
            Intent::Create => transport.create_room(),
            Intent::Join { code } => transport.join_room(code),
        }
        let room_code = match &intent {
            Intent::Create => None,
            Intent::Join { code } => Some(code.clone()),
        };
        Ok(Lobby {
            transport: Some(transport),
            state: LobbyState { intent, room_code },
        })
    }

    /// Read the relay's news. Call every frame.
    pub fn update(&mut self) -> LobbyOutcome {
        let Some(transport) = self.transport.as_mut() else {
            return LobbyOutcome::Waiting;
        };
        let events = transport.lobby_events();
        self.state.apply(events)
    }

    pub fn lines(&self) -> Vec<String> {
        self.state.lines()
    }

    /// Hand the connection to the match. Only called once, when the match
    /// starts; the lobby is dropped right after.
    pub fn take_transport(&mut self) -> WebSocketTransport {
        self.transport
            .take()
            .expect("the transport is only taken once, at match start")
    }
}

impl LobbyState {
    fn apply(&mut self, events: Vec<LobbyEvent>) -> LobbyOutcome {
        for event in events {
            match event {
                LobbyEvent::RoomCreated { code } => self.room_code = Some(code),
                LobbyEvent::MatchStart { player_index, seed } => {
                    return LobbyOutcome::Start { player_index, seed };
                }
                // Before the match starts, the relay only says this when it
                // closes on us, e.g. a join for a room that doesn't exist.
                LobbyEvent::PeerLeft => return LobbyOutcome::Failed(self.left_message()),
            }
        }
        LobbyOutcome::Waiting
    }

    fn left_message(&self) -> String {
        match &self.intent {
            Intent::Create => "The room was closed.".to_owned(),
            Intent::Join { code } => format!("Couldn't join room {code}."),
        }
    }

    pub fn lines(&self) -> Vec<String> {
        match (&self.intent, &self.room_code) {
            (Intent::Create, Some(code)) => vec![
                format!("Room code: {code}"),
                "Tell your friend to pick Join and type it.".to_owned(),
                "Waiting for them to join...".to_owned(),
            ],
            (Intent::Create, None) => vec!["Asking the relay for a room...".to_owned()],
            (Intent::Join { code }, _) => vec![format!("Joining room {code}...")],
        }
    }
}

/// An online match over the relay.
pub struct RemoteMatch {
    pub online: OnlineMatch<WebSocketTransport>,
    driver: TickDriver,
    /// Latest thing worth telling the player (a stall, a desync).
    note: Option<String>,
    disconnected: bool,
}

impl RemoteMatch {
    pub fn new(player_index: u8, seed: u64, transport: WebSocketTransport) -> RemoteMatch {
        RemoteMatch {
            online: OnlineMatch::new(player_index as usize, seed, transport),
            driver: TickDriver::default(),
            note: None,
            disconnected: false,
        }
    }

    /// One render frame. Only this machine's input goes in; the opponent's
    /// arrives over the network inside the session.
    pub fn update(&mut self, elapsed_micros: u64, local_input: Input) {
        let (ticks, inputs) = self.driver.frame(elapsed_micros, [local_input, 0]);
        for _ in 0..ticks {
            for event in self.online.tick(inputs[0]) {
                match event {
                    Event::Desync { frame } => {
                        // Always a bug; say so loudly (on screen and in the
                        // terminal).
                        eprintln!("DESYNC at frame {frame}");
                        self.note = Some(format!("DESYNC at frame {frame}"));
                    }
                    Event::Stalled => self.note = Some("Waiting for opponent...".to_owned()),
                    Event::Disconnected => self.disconnected = true,
                }
            }
        }
    }

    pub fn is_disconnected(&self) -> bool {
        self.disconnected
    }

    pub fn overlay(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "ONLINE  rolled back {} frames, deepest {}   Esc: leave",
            self.online.rollback_frames(),
            self.online.deepest_rollback()
        )];
        if let Some(note) = &self.note {
            lines.push(note.clone());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lobby(intent: Intent) -> LobbyState {
        LobbyState {
            intent,
            room_code: None,
        }
    }

    #[test]
    fn room_created_records_the_code_and_keeps_waiting() {
        let mut l = lobby(Intent::Create);
        let out = l.apply(vec![LobbyEvent::RoomCreated {
            code: "ABCD".to_owned(),
        }]);
        assert_eq!(out, LobbyOutcome::Waiting);
        assert_eq!(l.room_code.as_deref(), Some("ABCD"));
    }

    #[test]
    fn match_start_carries_player_and_seed() {
        let mut l = lobby(Intent::Join {
            code: "WXYZ".to_owned(),
        });
        let out = l.apply(vec![LobbyEvent::MatchStart {
            player_index: 1,
            seed: 99,
        }]);
        assert_eq!(
            out,
            LobbyOutcome::Start {
                player_index: 1,
                seed: 99
            }
        );
    }

    #[test]
    fn peer_left_before_start_fails_with_a_message() {
        let mut l = lobby(Intent::Join {
            code: "WXYZ".to_owned(),
        });
        let out = l.apply(vec![LobbyEvent::PeerLeft]);
        assert_eq!(
            out,
            LobbyOutcome::Failed("Couldn't join room WXYZ.".to_owned())
        );
    }
}
