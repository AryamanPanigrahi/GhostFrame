mod clock;
mod input;
mod menu;
mod render;
mod session;
mod sim;

use clock::FixedClock;
use input::{back_pressed, read_input, read_menu_keys, P1_KEYS, P2_KEYS};
use macroquad::prelude::*;
use menu::{Menu, MenuAction};
use session::LocalSession;
use sim::GameState;

fn window_conf() -> Conf {
    Conf {
        window_title: "Riposte".to_owned(),
        window_width: 1280,
        window_height: 720,
        window_resizable: true,
        ..Default::default()
    }
}

/// Local play has no server to hand out a seed, so any fixed value works.
/// Online, the seed comes from the server's `MatchStart` message.
const LOCAL_SEED: u64 = 1;

/// Shown for the online menu items until the relay server (S1), the
/// WebSocket transport (R6) and the online session are joined up in I1.
/// `\n` starts a new line on screen.
const ONLINE_NOT_READY: &str = "Online play isn't connected yet.\nIt needs the relay server and WebSocket transport (milestone I1).";

/// Microseconds since the game started. macroquad gives seconds as f64; we
/// convert once here so the clock works in whole numbers.
fn now_micros() -> u64 {
    (get_time() * 1_000_000.0) as u64
}

/// A local match in progress: the game and what drives it.
struct LocalMatch {
    game: GameState,
    session: LocalSession,
    clock: FixedClock,
    /// Buttons seen on any frame since the last tick, per player.
    buffered: [sim::Input; 2],
}

impl LocalMatch {
    fn new() -> LocalMatch {
        LocalMatch {
            game: GameState::new(LOCAL_SEED),
            session: LocalSession::default(),
            // A fresh clock, so time spent in the menu isn't banked as ticks.
            clock: FixedClock::default(),
            buffered: [0, 0],
        }
    }

    /// One render frame: work out how many 1/60 s ticks are due and run
    /// them. The sim never sees how long the frame took.
    ///
    /// Input is collected across frames: on a 144 Hz screen most frames run
    /// no tick at all, and a tap seen on one of those frames would otherwise
    /// never reach the sim.
    fn update(&mut self, elapsed_micros: u64) {
        self.buffered[0] |= read_input(&P1_KEYS);
        self.buffered[1] |= read_input(&P2_KEYS);
        let ticks = self.clock.advance(elapsed_micros);
        if ticks == 0 {
            return;
        }
        for _ in 0..ticks {
            self.session.add_local_input(0, self.buffered[0]);
            self.session.add_local_input(1, self.buffered[1]);
            self.session.tick(&mut self.game);
        }
        self.buffered = [0, 0];
    }
}

/// Which screen the program is on.
enum App {
    Menu(Menu),
    Local(LocalMatch),
}

// macroquad's main is async: `next_frame().await` hands control back to the
// window system (and the browser, in WASM) until the next frame is due.
#[macroquad::main(window_conf)]
async fn main() {
    let mut app = App::Menu(Menu::default());
    let mut last = now_micros();

    loop {
        let now = now_micros();
        let elapsed = now - last;
        last = now;

        // Switching screens is decided inside the match and applied after
        // it, because `app` can't be replaced while it's borrowed.
        let mut switch_to = None;
        match &mut app {
            App::Menu(menu) => {
                for key in read_menu_keys() {
                    match menu.handle(key) {
                        Some(MenuAction::StartLocal) => {
                            switch_to = Some(App::Local(LocalMatch::new()));
                        }
                        Some(MenuAction::CreateRoom) => {
                            menu.show_notice(ONLINE_NOT_READY.to_owned());
                        }
                        Some(MenuAction::JoinRoom { code }) => {
                            menu.show_notice(format!("Room {code}\n{ONLINE_NOT_READY}"));
                        }
                        None => {}
                    }
                }
                render::draw_menu(menu);
            }
            App::Local(local) => {
                if back_pressed() {
                    switch_to = Some(App::Menu(Menu::default()));
                }
                local.update(elapsed);
                render::draw(&local.game);
            }
        }
        if let Some(next) = switch_to {
            app = next;
        }

        next_frame().await;
    }
}
