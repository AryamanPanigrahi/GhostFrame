//! Keyboard → input bitmask. This is the only place that knows which keys
//! exist; the sim only ever sees `Input` bits.

use crate::sim::{Input, DOWN, HEAVY, LEFT, LIGHT, RIGHT, UP};
use macroquad::prelude::{is_key_down, KeyCode};

/// Which key sets which button bit, for one player.
pub type KeyMap = [(KeyCode, Input); 6];

/// Player 1, left side of the keyboard.
pub const P1_KEYS: KeyMap = [
    (KeyCode::A, LEFT),
    (KeyCode::D, RIGHT),
    (KeyCode::W, UP),
    (KeyCode::S, DOWN),
    (KeyCode::F, LIGHT),
    (KeyCode::G, HEAVY),
];

/// Player 2, right side of the keyboard.
pub const P2_KEYS: KeyMap = [
    (KeyCode::Left, LEFT),
    (KeyCode::Right, RIGHT),
    (KeyCode::Up, UP),
    (KeyCode::Down, DOWN),
    (KeyCode::K, LIGHT),
    (KeyCode::L, HEAVY),
];

/// Read the keys held right now for one player.
pub fn read_input(keys: &KeyMap) -> Input {
    input_from_keys(keys, is_key_down)
}

/// Split out from `read_input` so tests can pass a fake keyboard instead of
/// macroquad's, which needs a real window.
fn input_from_keys(keys: &KeyMap, is_down: fn(KeyCode) -> bool) -> Input {
    let mut input = 0;
    for &(key, button) in keys {
        if is_down(key) {
            input |= button;
        }
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing_held(_: KeyCode) -> bool {
        false
    }

    fn a_and_f_held(key: KeyCode) -> bool {
        key == KeyCode::A || key == KeyCode::F
    }

    #[test]
    fn no_keys_is_zero() {
        assert_eq!(input_from_keys(&P1_KEYS, nothing_held), 0);
    }

    #[test]
    fn held_keys_set_their_bits() {
        assert_eq!(input_from_keys(&P1_KEYS, a_and_f_held), LEFT | LIGHT);
    }

    #[test]
    fn players_dont_share_keys() {
        // P1's keys must not register for P2, or one keyboard can't hold both.
        assert_eq!(input_from_keys(&P2_KEYS, a_and_f_held), 0);
        for (p1_key, _) in P1_KEYS {
            assert!(P2_KEYS.iter().all(|&(p2_key, _)| p2_key != p1_key));
        }
    }
}
