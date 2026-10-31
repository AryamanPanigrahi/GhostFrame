mod clock;
mod input;
mod render;
mod session;
mod sim;

use clock::FixedClock;
use input::{read_input, P1_KEYS, P2_KEYS};
use macroquad::prelude::*;
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

// macroquad's main is async: `next_frame().await` hands control back to the
// window system (and the browser, in WASM) until the next frame is due.
#[macroquad::main(window_conf)]
async fn main() {
    let mut game = GameState::new(LOCAL_SEED);
    let mut session = LocalSession::default();
    let mut clock = FixedClock::default();
    let mut last = now_micros();

    loop {
        // One render frame: work out how many 1/60 s ticks are due, run them,
        // then draw once. The sim never sees how long the frame took.
        let now = now_micros();
        let ticks = clock.advance(now - last);
        last = now;

        let p1 = read_input(&P1_KEYS);
        let p2 = read_input(&P2_KEYS);
        for _ in 0..ticks {
            session.add_local_input(0, p1);
            session.add_local_input(1, p2);
            session.tick(&mut game);
        }

        render::draw(&game);
        next_frame().await;
    }
}
