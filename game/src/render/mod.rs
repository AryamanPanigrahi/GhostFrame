//! Drawing: stage, fighters, hit effects, HUD and menus, from the art in
//! game/assets/ (embedded by build.rs, loaded by assets.rs). Anything without
//! art falls back to the old placeholder boxes and debug text, so the game
//! runs with no art at all. Floats are fine here; this code never feeds back
//! into the sim.
//!
//! Hold Tab during a fight to see the sim's own boxes on top of the art
//! (hurtboxes, active hitboxes, held buttons): the art is decoration, those
//! boxes are what the hit detection uses.

use crate::menu::{Menu, Screen, MAIN_ITEMS, ROOM_CODE_LEN};
use crate::sim::combo::Special;
use crate::sim::fighter::{Action, Fighter, FLOOR_Y, MAX_HEALTH, SUBPIXELS};
use crate::sim::rect::Rect;
use crate::sim::round::{Phase, Round, MATCH_OVER_TICKS, READY_TICKS, ROUND_OVER_TICKS};
use crate::sim::{is_held, GameState, Input, DOWN, HEAVY, LEFT, LIGHT, RIGHT, UP};
use macroquad::prelude::*;
use macroquad::text::Font;

pub mod anim;
pub mod assets;
pub mod effects;
use anim::{Animator, Particle, Puff, FIGHT_BANNER_TICKS};
use assets::Assets;
use effects::Effects;

/// The sim's stage is 1280×720 logical pixels (DESIGN.md section 7).
const LOGICAL_W: f32 = 1280.0;
const LOGICAL_H: f32 = 720.0;

/// Fighter cell at 1x, with the feet centre at (160, 240) inside it.
const CELL_W: f32 = 320.0;
const CELL_H: f32 = 256.0;
const FEET_X: f32 = 160.0;
const FEET_Y: f32 = 240.0;
/// Stage layers are the screen plus this border on every side, so screen
/// shake never shows an edge.
const STAGE_BLEED: f32 = 32.0;

/// Palette slots shared with scripts/art/gen_art.py.
const TEXT: Color = Color::new(0.91, 0.925, 0.957, 1.0);
const DIM_TEXT: Color = Color::new(0.62, 0.6, 0.72, 1.0);
const ACCENT: Color = Color::new(0.62, 0.94, 0.415, 1.0);

const BUTTONS: [(Input, &str); 6] = [
    (LEFT, "L"),
    (RIGHT, "R"),
    (UP, "U"),
    (DOWN, "D"),
    (LIGHT, "LP"),
    (HEAVY, "HP"),
];

type MqRect = macroquad::math::Rect;

/// Maps logical stage coordinates to window pixels, keeping 16:9 and adding
/// black bars when the window is a different shape.
struct View<'a> {
    scale: f32,
    offset_x: f32,
    offset_y: f32,
    font: Option<&'a Font>,
}

impl<'a> View<'a> {
    /// `shake` is in logical pixels; (0.0, 0.0) means no screen shake.
    fn for_window(shake: (f32, f32), assets: &'a Assets) -> View<'a> {
        let scale = (screen_width() / LOGICAL_W).min(screen_height() / LOGICAL_H);
        View {
            scale,
            offset_x: (screen_width() - LOGICAL_W * scale) / 2.0 + shake.0 * scale,
            offset_y: (screen_height() - LOGICAL_H * scale) / 2.0 + shake.1 * scale,
            font: assets.font.as_ref(),
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

    /// Text with its baseline at `y`. The pixel font is sharpest at sizes
    /// that are multiples of 10 (one font pixel = size / 10).
    fn text(&self, text: &str, x: f32, y: f32, size: f32, color: Color) {
        let (sx, sy) = (
            self.offset_x + x * self.scale,
            self.offset_y + y * self.scale,
        );
        match self.font {
            Some(font) => {
                draw_text_ex(
                    text,
                    sx,
                    sy,
                    TextParams {
                        font: Some(font),
                        font_size: (size * self.scale).round().max(1.0) as u16,
                        color,
                        ..Default::default()
                    },
                );
            }
            None => {
                draw_text(text, sx, sy, size * self.scale, color);
            }
        }
    }

    /// Text with a dark copy 2 px down-right, for text over busy art.
    fn text_shadowed(&self, text: &str, x: f32, y: f32, size: f32, color: Color) {
        self.text(text, x + 2.0, y + 2.0, size, Color::new(0.0, 0.0, 0.0, 0.7));
        self.text(text, x, y, size, color);
    }

    /// Width of `text` in logical pixels.
    fn text_width(&self, text: &str, size: f32) -> f32 {
        let font_size = (size * self.scale).round().max(1.0) as u16;
        measure_text(text, self.font, font_size, 1.0).width / self.scale
    }

    /// A texture drawn into the logical rectangle `dest`, optionally only
    /// the part `source` (in texture pixels), optionally mirrored.
    fn texture(
        &self,
        tex: &Texture2D,
        dest: MqRect,
        source: Option<MqRect>,
        flip_x: bool,
        color: Color,
    ) {
        draw_texture_ex(
            tex,
            self.offset_x + dest.x * self.scale,
            self.offset_y + dest.y * self.scale,
            color,
            DrawTextureParams {
                dest_size: Some(vec2(dest.w * self.scale, dest.h * self.scale)),
                source,
                flip_x,
                ..Default::default()
            },
        );
    }

    /// A texture at (x, y) at its 1x size. `design_w` is the width the art
    /// checklist gives for it, so a 2x (high-res) export draws the same size.
    fn image(&self, tex: &Texture2D, x: f32, y: f32, design_w: f32, flip_x: bool) {
        let h = tex.height() * design_w / tex.width();
        self.texture(tex, MqRect::new(x, y, design_w, h), None, flip_x, WHITE);
    }

    /// A panel stretched to any size as a 9-slice: the 24 px corners stay
    /// sharp, the edges and middle stretch. `design_w` is the 1x width.
    fn panel(&self, tex: &Texture2D, dest: MqRect, design_w: f32) {
        let k = tex.width() / design_w; // texture pixels per logical pixel
        let c = 24.0;
        let (tw, th) = (tex.width(), tex.height());
        let cols = [
            (0.0, c, 0.0, c * k),
            (c, dest.w - 2.0 * c, c * k, tw - 2.0 * c * k),
            (dest.w - c, c, tw - c * k, c * k),
        ];
        let rows = [
            (0.0, c, 0.0, c * k),
            (c, dest.h - 2.0 * c, c * k, th - 2.0 * c * k),
            (dest.h - c, c, th - c * k, c * k),
        ];
        for &(dy, dh, sy, sh) in &rows {
            for &(dx, dw, sx, sw) in &cols {
                let to = MqRect::new(dest.x + dx, dest.y + dy, dw, dh);
                self.texture(tex, to, Some(MqRect::new(sx, sy, sw, sh)), false, WHITE);
            }
        }
    }
}

pub fn draw(game: &GameState, effects: &mut Effects, assets: &Assets) {
    clear_background(BLACK);
    effects.observe(game);
    let shake = effects.shake_offset();
    // `view` moves with the screen shake; `fixed` (stage window, HUD) doesn't.
    let view = View::for_window(shake, assets);
    let fixed = View::for_window((0.0, 0.0), assets);
    let anim = &effects.anim;

    draw_stage(&fixed, assets, shake);

    // Shadows first, so neither fighter's shadow covers the other fighter.
    for fighter in &game.fighters {
        draw_shadow(&view, assets, fighter);
    }
    // The fighter mid-attack goes on top, so the strike is never hidden.
    let order = if game.fighters[0].current_attack().is_some()
        && game.fighters[1].current_attack().is_none()
    {
        [1, 0]
    } else {
        [0, 1]
    };
    let mut has_art = true;
    for player in order {
        has_art &= draw_fighter(&view, assets, anim, game, player, effects.flash(player));
    }
    for particle in &anim.particles {
        draw_particle(&view, assets, particle, game.frame);
    }
    if let Some(fg) = assets.image("stage/fg.png") {
        draw_stage_layer(&fixed, fg, shake);
    }

    // Without fighter art, or with Tab held, show the sim's own boxes.
    if !has_art || is_key_down(KeyCode::Tab) {
        for (player, fighter) in game.fighters.iter().enumerate() {
            draw_debug_fighter(&view, player, fighter, effects.flash(player), has_art);
            draw_held_buttons(&fixed, player, game.inputs[player]);
        }
    }

    for (player, fighter) in game.fighters.iter().enumerate() {
        let wins = game.round.wins[player];
        draw_health_bar(&fixed, assets, anim, player, fighter.health, wins);
        draw_combo_hud(&fixed, player, fighter);
    }
    draw_round_status(&fixed, assets, anim, &game.round);
    draw_special_banner(&fixed, game);

    // The checksum is shown so two runs (or two laptops) can be compared by
    // eye: same inputs must give the same number.
    let debug = format!(
        "frame {}   checksum {:016x}   render fps {}",
        game.frame,
        game.checksum(),
        get_fps()
    );
    fixed.text_shadowed(&debug, 20.0, 708.0, 20.0, DIM_TEXT);
}

/// Streak counter and "special ready" prompt under each health bar. Plain
/// text on purpose: what it says is gameplay information, how it looks is
/// for the art pass.
fn draw_combo_hud(view: &View, player: usize, fighter: &Fighter) {
    let mut lines: Vec<(String, Color)> = Vec::new();
    let hits = u16::from(fighter.punches) + u16::from(fighter.kicks);
    if fighter.streak_timer > 0 && hits > 0 {
        lines.push((format!("COMBO x{hits}"), WHITE));
    }
    if let Some(special) = fighter.ready_special() {
        let (name, button) = match special {
            Special::DynamitePunch => ("DYNAMITE PUNCH", "punch"),
            Special::SuperKick => ("SUPER KICK", "kick"),
            Special::Nado => ("NADO", "punch or kick"),
        };
        lines.push((format!("{name} READY: press {button}"), YELLOW));
    }
    for (i, (text, color)) in lines.iter().enumerate() {
        let size = 22.0;
        let x = if player == 1 {
            1260.0 - view.text_width(text, size)
        } else {
            20.0
        };
        view.text_shadowed(text, x, 104.0 + i as f32 * 24.0, size, *color);
    }
}

/// The name of a special move across the top while it is being performed.
fn draw_special_banner(view: &View, game: &GameState) {
    for fighter in &game.fighters {
        let name = match fighter.action {
            Action::Dynamite => "DYNAMITE PUNCH!",
            Action::SuperKick => "SUPER KICK!",
            Action::Nado => "NADO!",
            _ => continue,
        };
        let size = 40.0;
        let x = 640.0 - view.text_width(name, size) / 2.0;
        view.text_shadowed(name, x, 150.0, size, YELLOW);
    }
}

/// Sim units (integer subpixels) to logical pixels. This is the boundary
/// where integers become floats; nothing here goes back into the sim.
fn px(subpixels: i32) -> f32 {
    subpixels as f32 / SUBPIXELS as f32
}

// ---------------------------------------------------------------- stage

fn draw_stage(view: &View, assets: &Assets, shake: (f32, f32)) {
    match assets.image("stage/bg_far.png") {
        Some(tex) => draw_stage_layer(view, tex, shake),
        None => view.rect(0.0, 0.0, LOGICAL_W, LOGICAL_H, DARKGRAY),
    }
    if let Some(tex) = assets.image("stage/bg_mid.png") {
        draw_stage_layer(view, tex, shake);
    }
    match assets.image("stage/floor.png") {
        Some(tex) => draw_stage_layer(view, tex, shake),
        None => {
            let floor_y = px(FLOOR_Y);
            view.rect(0.0, floor_y, LOGICAL_W, LOGICAL_H - floor_y, GRAY);
        }
    }
}

/// Draws the visible 1280×720 window of a stage layer. Shake moves which
/// part of the (larger) layer is shown rather than the layer itself, so the
/// picture never spills into the black bars around the stage.
fn draw_stage_layer(view: &View, tex: &Texture2D, shake: (f32, f32)) {
    let k = tex.width() / (LOGICAL_W + 2.0 * STAGE_BLEED);
    let source = MqRect::new(
        (STAGE_BLEED - shake.0) * k,
        (STAGE_BLEED - shake.1) * k,
        LOGICAL_W * k,
        LOGICAL_H * k,
    );
    let dest = MqRect::new(0.0, 0.0, LOGICAL_W, LOGICAL_H);
    view.texture(tex, dest, Some(source), false, WHITE);
}

// ---------------------------------------------------------------- fighters

/// Ellipse under the feet, shrinking as the fighter rises.
fn draw_shadow(view: &View, assets: &Assets, fighter: &Fighter) {
    let Some(tex) = assets.image("fx/shadow.png") else {
        return;
    };
    let height = px(FLOOR_Y - fighter.y);
    let k = (1.0 - height / 300.0).clamp(0.4, 1.0);
    let (w, h) = (96.0 * k, 20.0 * k);
    let dest = MqRect::new(px(fighter.x) - w / 2.0, px(FLOOR_Y) - h / 2.0, w, h);
    view.texture(tex, dest, None, false, WHITE);
}

/// Draws the fighter's sprite. Returns false if there's no art for it.
fn draw_fighter(
    view: &View,
    assets: &Assets,
    anim: &Animator,
    game: &GameState,
    player: usize,
    flash: f32,
) -> bool {
    let fighter = &game.fighters[player];
    let pose = anim.pose(game, player);
    let scheme = if player == 0 { "p1" } else { "p2" };
    // Optional animations (intro, win, turn) fall back to idle if missing.
    let frames = assets
        .anim(&format!("fighter/{scheme}/{}", pose.anim))
        .map(|f| (f, pose.index_for(f.len())))
        .or_else(|| {
            assets
                .anim(&format!("fighter/{scheme}/idle"))
                .map(|f| (f, 0))
        });
    let Some((frames, index)) = frames else {
        return false;
    };
    let dest = MqRect::new(
        px(fighter.x) - FEET_X,
        px(fighter.y) - FEET_Y,
        CELL_W,
        CELL_H,
    );
    // Art faces right; the cell is mirrored around the feet for facing left.
    let flip = fighter.facing < 0;
    // Hit flash: tint toward red by turning the other channels down.
    let tint = Color::new(1.0, 1.0 - 0.7 * flash, 1.0 - 0.7 * flash, 1.0);
    view.texture(&frames[index], dest, None, flip, tint);
    true
}

fn draw_particle(view: &View, assets: &Assets, p: &Particle, frame: u32) {
    let (folder, w, h, ticks) = match p.kind {
        Puff::SparkLight => ("fx/spark_light", 96.0, 96.0, anim::SPARK_TICKS_PER_DRAWING),
        Puff::SparkHeavy => (
            "fx/spark_heavy",
            160.0,
            160.0,
            anim::SPARK_TICKS_PER_DRAWING,
        ),
        Puff::DustJump => ("fx/dust_jump", 128.0, 48.0, anim::DUST_TICKS_PER_DRAWING),
        Puff::DustLand => ("fx/dust_land", 128.0, 48.0, anim::DUST_TICKS_PER_DRAWING),
    };
    let Some(frames) = assets.anim(folder) else {
        return;
    };
    let age = frame.saturating_sub(p.start);
    let index = ((age / ticks) as usize).min(frames.len() - 1);
    // Sparks are anchored at their centre, dust at its bottom centre.
    let top = match p.kind {
        Puff::SparkLight | Puff::SparkHeavy => p.y - h / 2.0,
        Puff::DustJump | Puff::DustLand => p.y - h,
    };
    let dest = MqRect::new(p.x - w / 2.0, top, w, h);
    view.texture(&frames[index], dest, None, p.facing < 0, WHITE);
}

/// The placeholder view: the hurtbox (filled only when there's no art), a
/// notch on the side it faces, the active hitbox and a state label.
fn draw_debug_fighter(view: &View, player: usize, fighter: &Fighter, flash: f32, has_art: bool) {
    let body = fighter.hurtbox();
    let (left, top) = (px(body.left), px(body.top));
    if has_art {
        view.sim_rect(body, GREEN, false);
    } else {
        view.sim_rect(body, flash_tint(LIGHTGRAY, flash), true);
        let w = px(body.right - body.left);
        let notch_x = if fighter.facing > 0 {
            left + w - 10.0
        } else {
            left
        };
        view.rect(notch_x, top + 20.0, 10.0, 20.0, BLACK);
    }
    if let Some(hitbox) = fighter.active_hitbox() {
        view.sim_rect(hitbox, RED, false);
    }
    let label = format!("P{} {:?} {}hp", player + 1, fighter.action, fighter.health);
    view.text_shadowed(&label, left, top - 10.0, 20.0, WHITE);
}

/// Which buttons this player held on the last tick (debug view only).
fn draw_held_buttons(view: &View, player: usize, input: Input) {
    let x0 = if player == 0 { 20.0 } else { 1000.0 };
    for (i, (button, name)) in BUTTONS.iter().enumerate() {
        let x = x0 + i as f32 * 44.0;
        let color = if is_held(input, *button) { WHITE } else { GRAY };
        view.rect(x, 92.0, 40.0, 40.0, color);
        view.text(name, x + 6.0, 120.0, 20.0, BLACK);
    }
}

/// Blends `base` toward red by `amount` (0.0 to 1.0) for the hit flash.
fn flash_tint(base: Color, amount: f32) -> Color {
    Color::new(
        base.r + (RED.r - base.r) * amount,
        base.g + (RED.g - base.g) * amount,
        base.b + (RED.b - base.b) * amount,
        base.a,
    )
}

// ---------------------------------------------------------------- HUD

/// Health bar along the top: frame, trailing damage, health, tag and round
/// wins. P2's side is P1's mirrored.
fn draw_health_bar(
    view: &View,
    assets: &Assets,
    anim: &Animator,
    player: usize,
    health: i16,
    wins: u8,
) {
    let mirror = player == 1;
    let frame = assets.image("hud/bar_frame.png");
    let (Some(frame), Some(fill)) = (frame, assets.image("hud/bar_fill.png")) else {
        // Placeholder bar.
        let (x, w, h) = (if mirror { 700.0 } else { 20.0 }, 560.0, 24.0);
        let filled = w * f32::from(health) / f32::from(MAX_HEALTH);
        view.rect(x, 20.0, w, h, GRAY);
        view.rect(x, 20.0, filled, h, LIGHTGRAY);
        let label = format!("P{}  {}hp  wins {}", player + 1, health, wins);
        view.text(&label, x, 66.0, 20.0, WHITE);
        return;
    };

    view.image(
        frame,
        if mirror { 692.0 } else { 12.0 },
        12.0,
        576.0,
        mirror,
    );

    // A bar cropped to `amount` (0..1) of its width and anchored at the
    // outer end: P1's drains toward the left edge, P2's toward the right.
    let cropped = |tex: &Texture2D, amount: f32| {
        let w = 560.0 * amount.clamp(0.0, 1.0);
        let k = tex.width() / 560.0;
        let source = MqRect::new(0.0, 0.0, w * k, tex.height());
        let x = if mirror { 1260.0 - w } else { 20.0 };
        view.texture(
            tex,
            MqRect::new(x, 20.0, w, 24.0),
            Some(source),
            mirror,
            WHITE,
        );
    };
    let max = f32::from(MAX_HEALTH);
    if let Some(damage) = assets.image("hud/bar_damage.png") {
        cropped(damage, anim.shown_health(player) / max);
    }
    cropped(fill, f32::from(health) / max);

    let tag = if mirror {
        "hud/p2_tag.png"
    } else {
        "hud/p1_tag.png"
    };
    if let Some(tag) = assets.image(tag) {
        view.image(tag, if mirror { 1204.0 } else { 12.0 }, 50.0, 64.0, false);
    }
    for i in 0..2u8 {
        let name = if wins > i {
            "hud/win_pip_full.png"
        } else {
            "hud/win_pip_empty.png"
        };
        if let Some(pip) = assets.image(name) {
            let offset = 84.0 + f32::from(i) * 28.0;
            let x = if mirror {
                LOGICAL_W - offset - 24.0
            } else {
                offset
            };
            view.image(pip, x, 54.0, 24.0, false);
        }
    }
}

/// Round number between the bars, and the banner for the current phase.
fn draw_round_status(view: &View, assets: &Assets, anim: &Animator, round: &Round) {
    let w = view.text_width("ROUND", 20.0);
    view.text_shadowed("ROUND", 640.0 - w / 2.0, 28.0, 20.0, DIM_TEXT);
    let number = round.number.to_string();
    let w = view.text_width(&number, 30.0);
    view.text_shadowed(&number, 640.0 - w / 2.0, 64.0, 30.0, TEXT);

    // (art file, text fallback, ticks since this banner appeared)
    let (file, fallback, elapsed): (String, String, u32) = match round.phase {
        Phase::Ready => {
            let elapsed = READY_TICKS.saturating_sub(round.timer);
            if elapsed < 30 {
                (
                    format!("hud/banner_round_{}.png", round.number),
                    format!("ROUND {}", round.number),
                    u32::from(elapsed),
                )
            } else {
                (
                    "hud/banner_ready.png".into(),
                    "READY".into(),
                    u32::from(elapsed - 30),
                )
            }
        }
        Phase::Fight if anim.fight_ticks < FIGHT_BANNER_TICKS => (
            "hud/banner_fight.png".into(),
            "FIGHT!".into(),
            anim.fight_ticks,
        ),
        Phase::Fight => return,
        Phase::RoundOver { winner } => {
            let elapsed = u32::from((ROUND_OVER_TICKS + 1).saturating_sub(round.timer));
            match winner {
                Some(_) => ("hud/banner_ko.png".into(), "K.O.".into(), elapsed),
                None => (
                    "hud/banner_double_ko.png".into(),
                    "DOUBLE K.O.".into(),
                    elapsed,
                ),
            }
        }
        Phase::MatchOver { winner } => (
            format!("hud/banner_p{}_wins.png", winner + 1),
            format!("P{} WINS", winner + 1),
            u32::from(MATCH_OVER_TICKS.saturating_sub(round.timer)),
        ),
    };
    match assets.image(&file) {
        Some(tex) => {
            // Pops in: starts at 1.5x and settles to full size over 6 ticks.
            // Banners are at most 800 px wide at 1x; a 2x export is halved.
            let pop = 1.0 + 0.5 * (1.0 - elapsed.min(6) as f32 / 6.0);
            let k = if tex.width() > 800.0 { 0.5 } else { 1.0 } * pop;
            let (w, h) = (tex.width() * k, tex.height() * k);
            let dest = MqRect::new(640.0 - w / 2.0, 300.0 - h / 2.0, w, h);
            view.texture(tex, dest, None, false, WHITE);
        }
        None => {
            let w = view.text_width(&fallback, 60.0);
            view.text_shadowed(&fallback, 640.0 - w / 2.0, 320.0, 60.0, WHITE);
        }
    }
}

/// Practice-mode and online stats over the fight.
pub fn draw_overlay(lines: &[String], assets: &Assets) {
    let view = View::for_window((0.0, 0.0), assets);
    for (i, line) in lines.iter().enumerate() {
        view.text_shadowed(line, 300.0, 170.0 + i as f32 * 28.0, 20.0, WHITE);
    }
}

// ---------------------------------------------------------------- menus

/// Background and logo shared by every screen outside a fight.
fn draw_backdrop(view: &View, assets: &Assets) {
    match assets.image("ui/menu_bg.png") {
        Some(bg) => view.image(bg, 0.0, 0.0, LOGICAL_W, false),
        None => view.rect(0.0, 0.0, LOGICAL_W, LOGICAL_H, DARKGRAY),
    }
    match assets.image("ui/logo.png") {
        Some(logo) => view.image(logo, 80.0, 60.0, logo.width().min(640.0), false),
        None => view.text("GHOSTFRAME", 80.0, 140.0, 70.0, WHITE),
    }
}

/// A box behind lobby, code-entry and notice text.
fn draw_panel(view: &View, assets: &Assets, dest: MqRect) {
    match assets.image("ui/panel.png") {
        Some(tex) => view.panel(tex, dest, 800.0),
        None => view.rect(
            dest.x,
            dest.y,
            dest.w,
            dest.h,
            Color::new(0.0, 0.0, 0.0, 0.5),
        ),
    }
}

/// A row of control hints: groups of key icons, each followed by a label.
/// Without an icon the key name is written out instead.
fn draw_key_hints(view: &View, assets: &Assets, x: f32, y: f32, hints: &[(&[&str], &str)]) {
    let mut x = x;
    for (keys, label) in hints {
        for key in *keys {
            match assets.image(&format!("ui/keys/{key}.png")) {
                Some(tex) => {
                    let w = if matches!(*key, "enter" | "esc") {
                        96.0
                    } else {
                        48.0
                    };
                    view.image(tex, x, y, w, false);
                    x += w + 4.0;
                }
                None => {
                    let text = format!("[{}]", key.to_uppercase());
                    view.text(&text, x, y + 32.0, 20.0, WHITE);
                    x += view.text_width(&text, 20.0) + 6.0;
                }
            }
        }
        view.text_shadowed(label, x + 6.0, y + 32.0, 20.0, DIM_TEXT);
        x += view.text_width(label, 20.0) + 36.0;
    }
}

/// The online lobby: what's happening, in a panel, with a spinner while
/// waiting.
pub fn draw_text_screen(lines: &[String], assets: &Assets) {
    clear_background(BLACK);
    let view = View::for_window((0.0, 0.0), assets);
    draw_backdrop(&view, assets);
    view.text_shadowed("ONLINE", 84.0, 200.0, 30.0, ACCENT);
    draw_panel(&view, assets, MqRect::new(80.0, 230.0, 1120.0, 300.0));
    for (i, line) in lines.iter().enumerate() {
        view.text(line, 120.0, 300.0 + i as f32 * 44.0, 30.0, TEXT);
    }
    let waiting = lines.first().is_some_and(|l| l.ends_with("..."));
    if let (true, Some(frames)) = (waiting, assets.anim("ui/spinner")) {
        // Wall-clock time is fine here: there's no sim on this screen.
        let i = (get_time() * 12.0) as usize % frames.len();
        let dest = MqRect::new(1110.0, 250.0, 64.0, 64.0);
        view.texture(&frames[i], dest, None, false, WHITE);
    }
}

pub fn draw_menu(menu: &Menu, assets: &Assets) {
    clear_background(BLACK);
    let view = View::for_window((0.0, 0.0), assets);
    draw_backdrop(&view, assets);

    match &menu.screen {
        Screen::Main { selected } => {
            for (i, item) in MAIN_ITEMS.iter().enumerate() {
                let y = 280.0 + i as f32 * 50.0;
                let chosen = i == *selected;
                if chosen {
                    match assets.image("ui/cursor.png") {
                        Some(cursor) => view.image(cursor, 76.0, y - 27.0, 32.0, false),
                        None => view.text(">", 84.0, y, 30.0, ACCENT),
                    }
                }
                let color = if chosen { ACCENT } else { TEXT };
                view.text_shadowed(item, 120.0, y, 30.0, color);
            }
            let choose: &[(&[&str], &str)] = &[
                (&["w", "s"], "or"),
                (&["up", "down"], "choose"),
                (&["enter"], "select"),
            ];
            draw_key_hints(&view, assets, 80.0, 500.0, choose);
            let p1: &[(&[&str], &str)] = &[
                (&[], "P1"),
                (&["a", "d", "w", "s"], "move"),
                (&["f"], "punch"),
                (&["g"], "kick"),
            ];
            draw_key_hints(&view, assets, 80.0, 580.0, p1);
            let p2: &[(&[&str], &str)] = &[
                (&[], "P2"),
                (&["left", "right", "up", "down"], "move"),
                (&["k"], "punch"),
                (&["l"], "kick"),
            ];
            draw_key_hints(&view, assets, 80.0, 640.0, p2);
        }
        Screen::EnterCode { code } => {
            draw_panel(&view, assets, MqRect::new(80.0, 230.0, 800.0, 260.0));
            view.text("Enter room code:", 120.0, 290.0, 30.0, TEXT);
            let letters: Vec<char> = code.chars().collect();
            for i in 0..ROOM_CODE_LEN {
                let (x, y) = (120.0 + i as f32 * 88.0, 320.0);
                let slot = if i == letters.len() {
                    "ui/code_slot_active.png"
                } else {
                    "ui/code_slot.png"
                };
                match assets.image(slot) {
                    Some(tex) => view.image(tex, x, y, 72.0, false),
                    None => view.text("_", x + 20.0, y + 70.0, 60.0, WHITE),
                }
                if let Some(c) = letters.get(i) {
                    let s = c.to_string();
                    let w = view.text_width(&s, 60.0);
                    view.text(&s, x + 36.0 - w / 2.0, y + 69.0, 60.0, TEXT);
                }
            }
            let hints: &[(&[&str], &str)] = &[
                (&[], "Type letters"),
                (&["enter"], "join"),
                (&["esc"], "back"),
            ];
            draw_key_hints(&view, assets, 80.0, 540.0, hints);
        }
        Screen::Notice { message } => {
            draw_panel(&view, assets, MqRect::new(80.0, 230.0, 1120.0, 260.0));
            // draw_text doesn't wrap, so messages carry their own line breaks.
            for (i, line) in message.lines().enumerate() {
                view.text(line, 120.0, 300.0 + i as f32 * 44.0, 30.0, TEXT);
            }
            draw_key_hints(&view, assets, 80.0, 540.0, &[(&["enter", "esc"], "back")]);
        }
    }
}
