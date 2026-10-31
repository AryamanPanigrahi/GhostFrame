//! PLACEHOLDER debug view: grey boxes and text, just enough to see the
//! movement, attacks, hits and rounds work. The real look comes from third-party art and
//! replaces this. Floats are fine here; this code never feeds back into the sim.

use crate::menu::{Menu, Screen, MAIN_ITEMS, ROOM_CODE_LEN};
use crate::sim::fighter::{Fighter, FLOOR_Y, MAX_HEALTH, SUBPIXELS};
use crate::sim::rect::Rect;
use crate::sim::round::{Phase, Round};
use crate::sim::{is_held, GameState, Input, DOWN, HEAVY, LEFT, LIGHT, RIGHT, UP};
use macroquad::prelude::*;

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

pub fn draw(game: &GameState) {
    clear_background(BLACK);
    let view = View::for_window();
    view.rect(0.0, 0.0, LOGICAL_W, LOGICAL_H, DARKGRAY);

    let floor_y = px(FLOOR_Y);
    view.rect(0.0, floor_y, LOGICAL_W, LOGICAL_H - floor_y, GRAY);

    for (player, fighter) in game.fighters.iter().enumerate() {
        draw_fighter(&view, player, fighter);
        draw_health_bar(&view, player, fighter.health, game.round.wins[player]);
        draw_held_buttons(&view, player, game.inputs[player]);
    }
    draw_round_status(&view, &game.round);

    // The checksum is shown so two runs (or, later, two laptops) can be
    // compared by eye: same inputs must give the same number.
    let debug = format!(
        "frame {}   checksum {:016x}   render fps {}",
        game.frame,
        game.checksum(),
        get_fps()
    );
    view.text(&debug, 20.0, 700.0, 22.0, WHITE);
}

/// Placeholder health bar along the top: an outline, filled in proportion
/// to health, with the round wins underneath.
fn draw_health_bar(view: &View, player: usize, health: i16, wins: u8) {
    let (x, w, h) = (if player == 0 { 20.0 } else { 700.0 }, 560.0, 24.0);
    let fill = w * health as f32 / MAX_HEALTH as f32;
    view.rect(x, 20.0, w, h, GRAY);
    view.rect(x, 20.0, fill, h, LIGHTGRAY);
    let label = format!("P{}  {}hp  wins {}", player + 1, health, wins);
    view.text(&label, x, 66.0, 22.0, WHITE);
}

/// Round number and what's happening, in the middle of the screen.
fn draw_round_status(view: &View, round: &Round) {
    view.text(&format!("Round {}", round.number), 600.0, 40.0, 24.0, WHITE);
    let banner = match round.phase {
        Phase::Ready => "READY".to_owned(),
        Phase::Fight => return,
        Phase::RoundOver { winner: Some(w) } => format!("K.O.  P{} wins the round", w + 1),
        Phase::RoundOver { winner: None } => "DOUBLE K.O.  no score".to_owned(),
        Phase::MatchOver { winner } => format!("P{} WINS THE MATCH", winner + 1),
    };
    view.text(&banner, 420.0, 300.0, 48.0, WHITE);
}

/// Sim units (integer subpixels) to logical pixels. This is the boundary
/// where integers become floats; nothing here goes back into the sim.
fn px(subpixels: i32) -> f32 {
    subpixels as f32 / SUBPIXELS as f32
}

/// The fighter drawn as its hurtbox, with a notch on the side it faces and
/// an outline of its hitbox while an attack is active. Drawing the sim's own
/// boxes means what you see is exactly what the hit detection uses.
fn draw_fighter(view: &View, player: usize, fighter: &Fighter) {
    let body = fighter.hurtbox();
    view.sim_rect(body, LIGHTGRAY, true);
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
        view.rect(x, 80.0, 40.0, 40.0, color);
        view.text(name, x + 6.0, 108.0, 22.0, BLACK);
    }
}

/// Placeholder menu: plain text, the selected item marked with ">".
pub fn draw_menu(menu: &Menu) {
    clear_background(BLACK);
    let view = View::for_window();
    view.rect(0.0, 0.0, LOGICAL_W, LOGICAL_H, DARKGRAY);
    view.text("RIPOSTE", 80.0, 140.0, 72.0, WHITE);

    match &menu.screen {
        Screen::Main { selected } => {
            for (i, item) in MAIN_ITEMS.iter().enumerate() {
                let marker = if i == *selected { ">" } else { " " };
                let line = format!("{marker} {item}");
                view.text(&line, 80.0, 260.0 + i as f32 * 50.0, 36.0, WHITE);
            }
            view.text(
                "Up/Down (or W/S) to choose, Enter to select",
                80.0,
                560.0,
                24.0,
                GRAY,
            );
        }
        Screen::EnterCode { code } => {
            view.text("Enter room code:", 80.0, 260.0, 36.0, WHITE);
            // Show unfilled places as underscores: "AB__".
            let shown = format!("{code:_<width$}", width = ROOM_CODE_LEN);
            view.text(&shown, 80.0, 330.0, 60.0, WHITE);
            view.text(
                "Type letters, Enter to join, Esc to go back",
                80.0,
                560.0,
                24.0,
                GRAY,
            );
        }
        Screen::Notice { message } => {
            // draw_text doesn't wrap, so messages carry their own line breaks.
            for (i, line) in message.lines().enumerate() {
                view.text(line, 80.0, 260.0 + i as f32 * 40.0, 26.0, WHITE);
            }
            view.text("Enter or Esc to go back", 80.0, 560.0, 24.0, GRAY);
        }
    }
}
