//! Messages between a client and the relay (DESIGN.md section 6).
//!
//! These enums are part of the frozen protocol: changing them needs both
//! authors to agree. The server never looks inside `Relay` bytes; they are
//! the peers' own `PeerMsg`s.

use serde::{Deserialize, Serialize};

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
