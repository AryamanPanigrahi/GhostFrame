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

pub use api::{Event, Frame, RollbackGame, Transport};
pub use input_queue::{AddOutcome, InputQueue, InputStatus};
