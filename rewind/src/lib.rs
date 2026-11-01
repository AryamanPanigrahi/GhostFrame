//! rewind: a game-agnostic rollback netcode library.
//!
//! A game implements `RollbackGame` (save, load, advance, checksum) and hands
//! itself to a session each tick. The session decides when to predict, roll
//! back and resimulate. Bytes move through a `Transport`, so the same session
//! runs over an in-memory pair, a fake laggy network or a WebSocket.
//!
//! rewind knows nothing about fighting games: `examples/boxes.rs` (R2) is a
//! toy game that exercises it without Riposte.

pub mod api;
pub mod input_queue;
pub mod protocol;
pub mod session;
pub mod snapshot;
pub mod transport;

pub use api::{Event, Frame, RollbackGame, Transport};
pub use input_queue::{AddOutcome, InputQueue, InputStatus};
pub use protocol::PeerMsg;
pub use session::p2p::{Config, P2PSession};
pub use session::synctest::{SyncTestError, SyncTestSession};
pub use transport::fake::{FakeConfig, FakeNetwork, FakeTransport, LinkStats};
pub use transport::memory::{memory_pair, MemoryTransport};
pub use transport::websocket::{ByteSocket, ConnectError, LobbyEvent, WebSocketTransport};
