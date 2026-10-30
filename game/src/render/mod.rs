//! PLACEHOLDER debug view: grey boxes and text, just enough to see that the
//! tick loop and inputs work. The real look comes from third-party art and
//! replaces this. Floats are fine here; this code never feeds back into the sim.

use crate::sim::{is_held, GameState, DOWN, HEAVY, LEFT, LIGHT, RIGHT, UP};
use macroquad::prelude::*;

/// The sim's stage is 1280×720 logical pixels (DESIGN.md section 7).
const LOGICAL_W: f32 = 1280.0;
const LOGICAL_H: f32 = 720.0;

const BUTTONS: [(u16, &str); 6] = [
    (LEFT, "L"),
    (RIGHT, "R"),
    (UP, "U"),
    (DOWN, "D"),
    (LIGHT, "LP"),
    (HEAVY, "HP"),
];

/// Maps logical stage coordinates to window pixels, keeping 16:9 and adding
/// black bars when the window is a different shape.
struct View {
    scale: f32,
    offset_x: f32,
    offset_y: f32,
}

impl View {
    fn for_window() -> View {
        let scale = (screen_width() / LOGICAL_W).min(screen_height() / LOGICAL_H);
        View {
            scale,
            offset_x: (screen_width() - LOGICAL_W * scale) / 2.0,
            offset_y: (screen_height() - LOGICAL_H * scale) / 2.0,
        }
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        draw_rectangle(
            self.offset_x + x * self.scale,
            self.offset_y + y * self.scale,
            w * self.scale,
            h * self.scale,
            color,
        );
    }

    fn text(&self, text: &str, x: f32, y: f32, size: f32, color: Color) {
        draw_text(
            text,
            self.offset_x + x * self.scale,
            self.offset_y + y * self.scale,
            size * self.scale,
            color,
        );
    }
}

pub fn draw(game: &GameState) {
    clear_background(BLACK);
    let view = View::for_window();
    view.rect(0.0, 0.0, LOGICAL_W, LOGICAL_H, DARKGRAY);

    view.text(
        &format!("sim frame {}", game.frame),
        40.0,
        60.0,
        40.0,
        WHITE,
    );
    view.text(
        &format!("sim seconds {}", game.frame / 60),
        40.0,
        100.0,
        40.0,
        WHITE,
    );
    view.text(
        &format!("render fps {}", get_fps()),
        40.0,
        140.0,
        40.0,
        GRAY,
    );

    for player in 0..2 {
        let x = 40.0 + player as f32 * 640.0;
        let label = format!("P{}", player + 1);
        view.text(&label, x, 260.0, 40.0, WHITE);
        for (i, (button, name)) in BUTTONS.iter().enumerate() {
            let bx = x + i as f32 * 90.0;
            let color = if is_held(game.inputs[player], *button) {
                LIGHTGRAY
            } else {
                GRAY
            };
            view.rect(bx, 290.0, 80.0, 80.0, color);
            view.text(name, bx + 20.0, 340.0, 32.0, BLACK);
        }
    }
}
