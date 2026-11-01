mod bot;
mod clock;
mod driver;
mod input;
mod menu;
mod online;
mod practice;
mod render;
mod session;
mod sim;

use driver::TickDriver;
use input::{
    back_pressed, lag_preset_pressed, read_either_player, read_input, read_menu_keys, P1_KEYS,
    P2_KEYS,
};
use macroquad::prelude::*;
use menu::{Menu, MenuAction};
use practice::Practice;
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
    /// A fresh driver per match, so time spent in the menu isn't banked as
    /// ticks.
    driver: TickDriver,
    /// Hit flash and screen shake (G6). Kept per match so a new match never
    /// starts with a leftover shake.
    effects: render::effects::Effects,
}

impl LocalMatch {
    fn new() -> LocalMatch {
        LocalMatch {
            game: GameState::new(LOCAL_SEED),
            session: LocalSession::default(),
            driver: TickDriver::default(),
            effects: render::effects::Effects::default(),
        }
    }

    /// One render frame: run the 1/60 s ticks that are due. The sim never
    /// sees how long the frame took.
    fn update(&mut self, elapsed_micros: u64) {
        let keys = [read_input(&P1_KEYS), read_input(&P2_KEYS)];
        let (ticks, inputs) = self.driver.frame(elapsed_micros, keys);
        for _ in 0..ticks {
            self.session.add_local_input(0, inputs[0]);
            self.session.add_local_input(1, inputs[1]);
            self.session.tick(&mut self.game);
        }
    }
}

/// Which screen the program is on.
enum App {
    Menu(Menu),
    Local(LocalMatch),
    /// Boxed because it holds two games and two snapshot rings, much bigger
    /// than the other variants.
    Practice(Box<Practice>),
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
                        Some(MenuAction::StartPractice) => {
                            switch_to = Some(App::Practice(Box::new(Practice::new())));
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
                render::draw(&local.game, &mut local.effects);
            }
            App::Practice(practice) => {
                if back_pressed() {
                    switch_to = Some(App::Menu(Menu::default()));
                }
                if let Some(preset) = lag_preset_pressed() {
                    practice.set_preset(preset);
                }
                practice.update(elapsed, read_either_player());
                render::draw(&practice.you.game, &mut practice.you.effects);
                render::draw_overlay(&practice.overlay());
            }
        }
        if let Some(next) = switch_to {
            app = next;
        }

        next_frame().await;
    }
}
