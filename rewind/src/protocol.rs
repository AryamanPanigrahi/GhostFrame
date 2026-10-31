//! Messages between the two peers (DESIGN.md section 6). The relay server
//! never looks inside them; it forwards the bytes unchanged.
//!
//! FROZEN: part of the protocol both authors agreed on. Changing it needs
//! agreement from both people.

use serde::{Deserialize, Serialize};

use crate::api::Frame;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerMsg<I> {
    /// The sender's inputs for frames `start_frame`, `start_frame + 1`, ...
    /// Every input the receiver hasn't acknowledged yet is sent again in
    /// every message, so a lost message is covered by the next one.
    ///
    /// `ack_frame` is the first frame the sender has *not* received from the
    /// receiver yet, i.e. "I have all your inputs before this frame". Using
    /// "next frame needed" rather than "last frame received" means 0 works as
    /// "nothing received yet", without an Option.
    Inputs {
        start_frame: Frame,
        inputs: Vec<I>,
        ack_frame: Frame,
    },
    /// The sender's game checksum after `frame`, for desync detection (R5).
    Checksum { frame: Frame, hash: u64 },
}
