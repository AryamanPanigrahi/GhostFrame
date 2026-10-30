//! Deterministic simulation. No floats, no time, no window: given the same
//! state and the same inputs, `advance` must produce the same result on every
//! machine, because rollback re-runs it and both peers run it independently.

pub mod fighter;

use fighter::{Fighter, SUBPIXELS};

/// One player's buttons for one tick, packed into bits (DESIGN.md section 4).
/// A u16 is tiny to send over the network and cheap to compare.
pub type Input = u16;

pub const LEFT: Input = 1 << 0;
pub const RIGHT: Input = 1 << 1;
pub const UP: Input = 1 << 2;
pub const DOWN: Input = 1 << 3;
pub const LIGHT: Input = 1 << 4;
pub const HEAVY: Input = 1 << 5;

/// True if `button` (one of the constants above) is set in `input`.
pub fn is_held(input: Input, button: Input) -> bool {
    input & button != 0
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameState {
    /// Number of ticks simulated so far. At 60 ticks/s this is also the clock.
    pub frame: u32,
    /// Inputs used on the most recent tick. G3 compares new inputs against
    /// these to tell "just pressed" from "still held".
    pub inputs: [Input; 2],
    pub fighters: [Fighter; 2],
}

impl Default for GameState {
    /// Round start: both fighters on the floor, 480 px apart, facing each other.
    fn default() -> GameState {
        GameState {
            frame: 0,
            inputs: [0, 0],
            fighters: [
                Fighter::new(400 * SUBPIXELS, 1),
                Fighter::new(880 * SUBPIXELS, -1),
            ],
        }
    }
}

impl GameState {
    /// Advance exactly one tick (1/60 s).
    pub fn advance(&mut self, inputs: [Input; 2]) {
        // Both fighters turn using positions from the *start* of the tick,
        // before either moves. That way updating player 0 first can't give
        // player 0 an advantage: the result doesn't depend on update order.
        let x0 = self.fighters[0].x;
        let x1 = self.fighters[1].x;
        self.fighters[0].face(x1);
        self.fighters[1].face(x0);

        self.fighters[0].step(inputs[0]);
        self.fighters[1].step(inputs[1]);

        self.frame += 1;
        self.inputs = inputs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_use_distinct_bits() {
        let all = [LEFT, RIGHT, UP, DOWN, LIGHT, HEAVY];
        for (i, a) in all.iter().enumerate() {
            assert_eq!(a.count_ones(), 1);
            for b in &all[i + 1..] {
                assert_eq!(a & b, 0);
            }
        }
    }

    #[test]
    fn is_held_checks_only_that_button() {
        let input = LEFT | LIGHT;
        assert!(is_held(input, LEFT));
        assert!(is_held(input, LIGHT));
        assert!(!is_held(input, RIGHT));
        assert!(!is_held(input, HEAVY));
    }

    #[test]
    fn advance_counts_frames_and_records_inputs() {
        let mut state = GameState::default();
        state.advance([LEFT, HEAVY]);
        state.advance([RIGHT | UP, 0]);
        assert_eq!(state.frame, 2);
        assert_eq!(state.inputs, [RIGHT | UP, 0]);
    }

    #[test]
    fn each_player_moves_with_their_own_input() {
        let mut state = GameState::default();
        let start = state.clone();
        state.advance([RIGHT, 0]);
        assert!(state.fighters[0].x > start.fighters[0].x);
        assert_eq!(state.fighters[1], start.fighters[1]);
    }

    #[test]
    fn fighters_turn_around_after_crossing() {
        let mut state = GameState::default();
        // Put P1 to the right of P2 on the ground, as if they'd crossed.
        state.fighters[0].x = 1000 * SUBPIXELS;
        state.advance([0, 0]);
        assert_eq!(state.fighters[0].facing, -1);
        assert_eq!(state.fighters[1].facing, 1);
    }

    #[test]
    fn same_inputs_give_same_state() {
        let inputs = [[LEFT, 0], [LEFT | UP, DOWN], [0, HEAVY], [RIGHT, UP | LEFT]];
        let mut a = GameState::default();
        let mut b = GameState::default();
        for i in inputs {
            a.advance(i);
            b.advance(i);
        }
        assert_eq!(a, b);
    }
}
