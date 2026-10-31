//! Round and match flow: a pause before each round, the fight, a pause
//! after a KO, and a pause after the match is decided.

/// Round wins needed to take the match (best of 3).
pub const WINS_TO_MATCH: u8 = 2;

/// Ticks for each pause. At 60 ticks/s: 1 s, 2 s and 4 s.
pub const READY_TICKS: u16 = 60;
pub const ROUND_OVER_TICKS: u16 = 120;
pub const MATCH_OVER_TICKS: u16 = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Before the round: fighters are placed but input is ignored.
    Ready,
    Fight,
    /// Someone was KO'd. `winner` is None for a double KO.
    RoundOver {
        winner: Option<u8>,
    },
    MatchOver {
        winner: u8,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    /// 1 for the first round of a match.
    pub number: u8,
    pub wins: [u8; 2],
    pub phase: Phase,
    /// Ticks left in the current pause. Unused during Fight.
    pub timer: u16,
}

impl Round {
    pub fn new_match() -> Round {
        Round {
            number: 1,
            wins: [0, 0],
            phase: Phase::Ready,
            timer: READY_TICKS,
        }
    }

    /// Players' inputs only count while fighting.
    pub fn accepts_input(&self) -> bool {
        self.phase == Phase::Fight
    }
}
