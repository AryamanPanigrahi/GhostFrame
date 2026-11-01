mod bot;
mod clock;
mod driver;
mod input;
mod menu;
mod netplay;
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
use netplay::{Intent, Lobby, LobbyOutcome, RemoteMatch};
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
    /// In a relay room, waiting for the match to start.
    Lobby(Box<Lobby>),
    /// An online match; boxed for the same reason as `Practice`.
    Online(Box<RemoteMatch>),
}

/// Connect to the relay, or go back to the menu with the reason it failed.
fn enter_lobby(menu: &mut Menu, intent: Intent) -> Option<App> {
    let url = netplay::relay_url();
    match Lobby::connect(&url, intent) {
        Ok(lobby) => Some(App::Lobby(Box::new(lobby))),
        Err(e) => {
            menu.show_notice(format!("{e}\nIs the relay running? ({url})"));
            None
        }
    }
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
                            switch_to = enter_lobby(menu, Intent::Create);
                        }
                        Some(MenuAction::JoinRoom { code }) => {
                            switch_to = enter_lobby(menu, Intent::Join { code });
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
            App::Lobby(lobby) => {
                if back_pressed() {
                    switch_to = Some(App::Menu(Menu::default()));
                } else {
                    match lobby.update() {
                        LobbyOutcome::Waiting => {}
                        LobbyOutcome::Start { player_index, seed } => {
                            // The match keeps using the lobby's connection.
                            let transport = lobby.take_transport();
                            let remote = RemoteMatch::new(player_index, seed, transport);
                            switch_to = Some(App::Online(Box::new(remote)));
                        }
                        LobbyOutcome::Failed(message) => {
                            let mut menu = Menu::default();
                            menu.show_notice(message);
                            switch_to = Some(App::Menu(menu));
                        }
                    }
                }
                clear_background(BLACK);
                let mut lines = lobby.lines();
                lines.push("Esc: cancel".to_owned());
                render::draw_overlay(&lines);
            }
            App::Online(remote) => {
                if back_pressed() || remote.is_disconnected() {
                    let mut menu = Menu::default();
                    if remote.is_disconnected() {
                        menu.show_notice("Your opponent disconnected.".to_owned());
                    }
                    switch_to = Some(App::Menu(menu));
                } else {
                    remote.update(elapsed, read_either_player());
                    render::draw(&remote.online.game, &mut remote.online.effects);
                    render::draw_overlay(&remote.overlay());
                }
            }
        }
        if let Some(next) = switch_to {
            app = next;
        }

        next_frame().await;
    }
}
