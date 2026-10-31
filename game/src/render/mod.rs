//! PLACEHOLDER debug view: grey boxes and text, just enough to see the
//! movement, attacks and hits work. The real look comes from third-party art and
//! replaces this. Floats are fine here; this code never feeds back into the sim.

use crate::sim::fighter::{Fighter, FLOOR_Y, SUBPIXELS};
use crate::sim::rect::Rect;
use crate::sim::{is_held, GameState, Input, DOWN, HEAVY, LEFT, LIGHT, RIGHT, UP};
use macroquad::prelude::*;

pub mod effects;
use effects::Effects;

/// The sim's stage is 1280×720 logical pixels (DESIGN.md section 7).
const LOGICAL_W: f32 = 1280.0;
const LOGICAL_H: f32 = 720.0;

const BUTTONS: [(Input, &str); 6] = [
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
    fn for_window(shake: (f32, f32)) -> View {
        let scale = (screen_width() / LOGICAL_W).min(screen_height() / LOGICAL_H);
        View {
            scale,
            offset_x: (screen_width() - LOGICAL_W * scale) / 2.0 + shake.0 * scale,
            offset_y: (screen_height() - LOGICAL_H * scale) / 2.0 + shake.1 * scale,
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

    /// A rectangle given in sim units (subpixels), filled or outlined.
    fn sim_rect(&self, r: Rect, color: Color, filled: bool) {
        let (x, y) = (px(r.left), px(r.top));
        let (w, h) = (px(r.right - r.left), px(r.bottom - r.top));
        if filled {
            self.rect(x, y, w, h, color);
        } else {
            draw_rectangle_lines(
                self.offset_x + x * self.scale,
                self.offset_y + y * self.scale,
                w * self.scale,
                h * self.scale,
                2.0,
                color,
            );
        }
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

pub fn draw(game: &GameState, effects: &mut Effects) {
    clear_background(BLACK);
    effects.observe(game);
    let view = View::for_window(effects.shake_offset());
    view.rect(0.0, 0.0, LOGICAL_W, LOGICAL_H, DARKGRAY);

    let floor_y = px(FLOOR_Y);
    view.rect(0.0, floor_y, LOGICAL_W, LOGICAL_H - floor_y, GRAY);

    for (player, fighter) in game.fighters.iter().enumerate() {
        draw_fighter(&view, player, fighter, effects.flash(player));
        draw_held_buttons(&view, player, game.inputs[player]);
    }

    let debug = format!("frame {}   render fps {}", game.frame, get_fps());
    view.text(&debug, 500.0, 30.0, 24.0, WHITE);
}

/// Sim units (integer subpixels) to logical pixels. This is the boundary
/// where integers become floats; nothing here goes back into the sim.
fn px(subpixels: i32) -> f32 {
    subpixels as f32 / SUBPIXELS as f32
}

/// The fighter drawn as its hurtbox, with a notch on the side it faces and
/// an outline of its hitbox while an attack is active. Drawing the sim's own
/// boxes means what you see is exactly what the hit detection uses.
fn draw_fighter(view: &View, player: usize, fighter: &Fighter, flash: f32) {
    let body = fighter.hurtbox();
    view.sim_rect(body, flash_tint(LIGHTGRAY, flash), true);
    if let Some(hitbox) = fighter.active_hitbox() {
        view.sim_rect(hitbox, RED, false);
    }

    let (left, top) = (px(body.left), px(body.top));
    let w = px(body.right - body.left);

    let notch_x = if fighter.facing > 0 {
        left + w - 10.0
    } else {
        left
    };
    view.rect(notch_x, top + 20.0, 10.0, 20.0, BLACK);

    let label = format!("P{} {:?} {}hp", player + 1, fighter.action, fighter.health);
    view.text(&label, left, top - 10.0, 24.0, WHITE);
}

/// Which buttons this player held on the last tick, top-left for P1 and
/// top-right for P2.
fn draw_held_buttons(view: &View, player: usize, input: Input) {
    let x0 = if player == 0 { 20.0 } else { 1000.0 };
    for (i, (button, name)) in BUTTONS.iter().enumerate() {
        let x = x0 + i as f32 * 44.0;
        let color = if is_held(input, *button) { WHITE } else { GRAY };
        view.rect(x, 50.0, 40.0, 40.0, color);
        view.text(name, x + 6.0, 78.0, 22.0, BLACK);
    }
}

/// Blends ase toward red by mount (0.0 to 1.0) for the hit flash.
fn flash_tint(base: Color, amount: f32) -> Color {
    Color::new(
        base.r + (RED.r - base.r) * amount,
        base.g + (RED.g - base.g) * amount,
        base.b + (RED.b - base.b) * amount,
        base.a,
    )
}
