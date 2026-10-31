//! Keyboard → input bitmask, and keyboard → menu keys. This is the only
//! place that knows which keys exist; the sim only ever sees `Input` bits and
//! the menu only sees `MenuKey`s.

use crate::menu::MenuKey;
use crate::sim::{Input, DOWN, HEAVY, LEFT, LIGHT, RIGHT, UP};
use macroquad::prelude::{get_char_pressed, is_key_down, is_key_pressed, KeyCode};

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

/// Read one player's buttons for this frame.
pub fn read_input(keys: &KeyMap) -> Input {
    input_from_keys(keys, held_or_tapped)
}

/// Down right now, or pressed at some point since the last frame. A quick
/// tap can go down *and* up between two frames; `is_key_down` alone would
/// never see it and the attack would be lost (seen in the browser build at
/// low frame rates). `is_key_pressed` remembers the press for one frame.
fn held_or_tapped(key: KeyCode) -> bool {
    is_key_down(key) || is_key_pressed(key)
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

/// Menu navigation. Both players' up/down keys work, so either can drive it.
const MENU_KEYS: [(KeyCode, MenuKey); 8] = [
    (KeyCode::Up, MenuKey::Up),
    (KeyCode::W, MenuKey::Up),
    (KeyCode::Down, MenuKey::Down),
    (KeyCode::S, MenuKey::Down),
    (KeyCode::Enter, MenuKey::Confirm),
    (KeyCode::KpEnter, MenuKey::Confirm),
    (KeyCode::Escape, MenuKey::Back),
    (KeyCode::Backspace, MenuKey::Backspace),
];

/// Menu keys pressed this frame, then any typed characters as `Letter`s.
/// Typing "w" in a room code also sends `Up`; screens ignore keys that don't
/// apply to them, so this is harmless.
pub fn read_menu_keys() -> Vec<MenuKey> {
    let mut keys = menu_keys_from(is_key_pressed);
    while let Some(c) = get_char_pressed() {
        keys.push(MenuKey::Letter(c));
    }
    keys
}

/// Split out like `input_from_keys`, so tests can fake the keyboard.
fn menu_keys_from(is_pressed: fn(KeyCode) -> bool) -> Vec<MenuKey> {
    let mut keys = Vec::new();
    for &(code, key) in &MENU_KEYS {
        if is_pressed(code) {
            keys.push(key);
        }
    }
    keys
}

/// Escape during a local match goes back to the menu.
pub fn back_pressed() -> bool {
    is_key_pressed(KeyCode::Escape)
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

    fn enter_and_w_pressed(key: KeyCode) -> bool {
        key == KeyCode::Enter || key == KeyCode::W
    }

    #[test]
    fn menu_keys_map_in_table_order() {
        assert_eq!(
            menu_keys_from(enter_and_w_pressed),
            vec![MenuKey::Up, MenuKey::Confirm]
        );
        assert!(menu_keys_from(nothing_held).is_empty());
    }
}
