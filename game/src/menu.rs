//! Main menu: pick local play, create an online room, or join one with a
//! 4-letter code. Pure logic driven by `MenuKey`s, so it's testable without
//! a window; `input.rs` turns real key presses into `MenuKey`s.
//!
//! The menu is outside the sim: it never affects the game state, so it has
//! no determinism rules.

/// Room codes are 4 letters (DESIGN.md section 8).
pub const ROOM_CODE_LEN: usize = 4;

pub const MAIN_ITEMS: [&str; 3] = ["Local 2-player", "Create online room", "Join online room"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Confirm,
    Back,
    Backspace,
    Letter(char),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Main {
        selected: usize,
    },
    EnterCode {
        code: String,
    },
    /// A message with only "back" as an option.
    Notice {
        message: String,
    },
}

/// What the menu asks the rest of the game to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    StartLocal,
    CreateRoom,
    JoinRoom { code: String },
}

pub struct Menu {
    pub screen: Screen,
}

impl Default for Menu {
    fn default() -> Menu {
        Menu {
            screen: Screen::Main { selected: 0 },
        }
    }
}

impl Menu {
    /// Apply one key press. Returns an action when the player has chosen one.
    pub fn handle(&mut self, key: MenuKey) -> Option<MenuAction> {
        match &mut self.screen {
            Screen::Main { selected } => {
                let count = MAIN_ITEMS.len();
                match key {
                    // Wrap around at both ends.
                    MenuKey::Up => *selected = (*selected + count - 1) % count,
                    MenuKey::Down => *selected = (*selected + 1) % count,
                    MenuKey::Confirm => match *selected {
                        0 => return Some(MenuAction::StartLocal),
                        1 => return Some(MenuAction::CreateRoom),
                        _ => {
                            self.screen = Screen::EnterCode {
                                code: String::new(),
                            }
                        }
                    },
                    _ => {}
                }
            }
            Screen::EnterCode { code } => match key {
                MenuKey::Letter(c) => {
                    // Letters only, stored uppercase, so "abcd" and "ABCD"
                    // are the same room.
                    if c.is_ascii_alphabetic() && code.len() < ROOM_CODE_LEN {
                        code.push(c.to_ascii_uppercase());
                    }
                }
                MenuKey::Backspace => {
                    code.pop();
                }
                MenuKey::Confirm if code.len() == ROOM_CODE_LEN => {
                    return Some(MenuAction::JoinRoom { code: code.clone() });
                }
                MenuKey::Back => self.back_to_main(),
                _ => {}
            },
            Screen::Notice { .. } => {
                if key == MenuKey::Back || key == MenuKey::Confirm {
                    self.back_to_main();
                }
            }
        }
        None
    }

    pub fn show_notice(&mut self, message: String) {
        self.screen = Screen::Notice { message };
    }

    pub fn back_to_main(&mut self) {
        self.screen = Screen::Main { selected: 0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(menu: &mut Menu, keys: &[MenuKey]) -> Option<MenuAction> {
        let mut last = None;
        for &k in keys {
            last = menu.handle(k);
        }
        last
    }

    #[test]
    fn confirm_on_first_item_starts_local_play() {
        let mut menu = Menu::default();
        assert_eq!(menu.handle(MenuKey::Confirm), Some(MenuAction::StartLocal));
    }

    #[test]
    fn selection_wraps_around() {
        let mut menu = Menu::default();
        menu.handle(MenuKey::Up);
        assert_eq!(menu.screen, Screen::Main { selected: 2 });
        menu.handle(MenuKey::Down);
        assert_eq!(menu.screen, Screen::Main { selected: 0 });
    }

    #[test]
    fn second_item_creates_a_room() {
        let mut menu = Menu::default();
        let action = press(&mut menu, &[MenuKey::Down, MenuKey::Confirm]);
        assert_eq!(action, Some(MenuAction::CreateRoom));
    }

    #[test]
    fn join_takes_four_letters_uppercased() {
        let mut menu = Menu::default();
        press(&mut menu, &[MenuKey::Up, MenuKey::Confirm]);
        let keys = [
            MenuKey::Letter('a'),
            MenuKey::Letter('7'), // ignored: not a letter
            MenuKey::Letter('b'),
            MenuKey::Letter('c'),
            MenuKey::Letter('d'),
            MenuKey::Letter('e'), // ignored: already 4
            MenuKey::Confirm,
        ];
        let action = press(&mut menu, &keys);
        assert_eq!(
            action,
            Some(MenuAction::JoinRoom {
                code: "ABCD".to_owned()
            })
        );
    }

    #[test]
    fn short_code_cannot_be_confirmed() {
        let mut menu = Menu::default();
        press(&mut menu, &[MenuKey::Up, MenuKey::Confirm]);
        let keys = [MenuKey::Letter('a'), MenuKey::Letter('b'), MenuKey::Confirm];
        assert_eq!(press(&mut menu, &keys), None);
    }

    #[test]
    fn backspace_removes_the_last_letter() {
        let mut menu = Menu::default();
        press(&mut menu, &[MenuKey::Up, MenuKey::Confirm]);
        press(
            &mut menu,
            &[
                MenuKey::Letter('x'),
                MenuKey::Letter('y'),
                MenuKey::Backspace,
            ],
        );
        assert_eq!(
            menu.screen,
            Screen::EnterCode {
                code: "X".to_owned()
            }
        );
    }

    #[test]
    fn back_leaves_code_entry_and_notices() {
        let mut menu = Menu::default();
        press(&mut menu, &[MenuKey::Up, MenuKey::Confirm, MenuKey::Back]);
        assert_eq!(menu.screen, Screen::Main { selected: 0 });

        menu.show_notice("hello".to_owned());
        menu.handle(MenuKey::Back);
        assert_eq!(menu.screen, Screen::Main { selected: 0 });
    }
}
