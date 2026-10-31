//! The interface between rewind and any game (DESIGN.md section 4).
//!
//! FROZEN: both authors build against this. Changing it needs agreement from
//! both people.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// A frame number: 0 is the first tick of the match. At 60 ticks/s a u32
/// lasts over two years, and the protocol sends frames as u32 too.
pub type Frame = u32;

/// Implemented by any game that wants rollback.
pub trait RollbackGame {
    /// One player's input for one frame. Riposte uses a u16 bitmask.
    type Input: Copy + PartialEq + Default + Serialize + DeserializeOwned;
    /// A full copy of everything the simulation needs.
    type State: Clone;

    fn save(&self) -> Self::State;
    fn load(&mut self, state: &Self::State);
    /// Advance exactly one tick (1/60 s) using both players' inputs.
    fn advance(&mut self, inputs: [Self::Input; 2]);
    /// Deterministic hash of the current state, used for desync detection.
    fn checksum(&self) -> u64;
}

/// How rewind sends and receives bytes. Swappable: in-memory, fake-latency,
/// WebSocket.
pub trait Transport {
    fn send(&mut self, bytes: &[u8]);
    /// Non-blocking: all messages received since the last poll.
    fn poll(&mut self) -> Vec<Vec<u8>>;
}

/// Something a session tells the game about while ticking (DESIGN.md
/// section 4, "How the game uses a session").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Our checksum for `frame` differs from the peer's: the two simulations
    /// have drifted apart. Always a bug (non-determinism) to log loudly.
    Desync { frame: Frame },
    /// We are too far ahead of the opponent, so this tick was skipped.
    Stalled,
    /// The peer is gone.
    Disconnected,
}
