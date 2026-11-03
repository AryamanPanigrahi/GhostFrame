//! Picks which sprite frame each fighter shows, and tracks the short-lived
//! render-only things the sim doesn't store: hit sparks, dust puffs, the
//! trailing damage bar, and how long the FIGHT banner has been up.
//!
//! Like effects.rs, this only *watches* the sim state, once per draw, and
//! counts in sim ticks (frame numbers), never wall-clock time. Nothing here
//! is read by the sim, so it can't change a hit, a position or a checksum.
//! Floats are fine here.
//!
//! Most frames come straight from sim fields (an attack's `action_frame`,
//! the jump's vertical speed, the round timer). A few need a little memory
//! that the sim doesn't keep (ticks spent crouching, whether a hitstun came
//! from a light or heavy attack, the landing squash); that is tracked here.
//! After a rollback the tracked values can be off by a tick or two for a
//! moment, which is purely cosmetic.

use crate::sim::attack::{HEAVY_ATTACK, LIGHT_ATTACK};
use crate::sim::fighter::{Action, Fighter, FLOOR_Y, GRAVITY, JUMP_VELOCITY, SUBPIXELS};
use crate::sim::round::{Phase, READY_TICKS, ROUND_OVER_TICKS};
use crate::sim::GameState;

/// Ticks each drawing of the light attack is held: 4 startup, 3 active
/// (drawing 02 is the full reach), 8 recovery. Must match LIGHT_ATTACK.
const LIGHT_TIMELINE: [u16; 6] = [2, 2, 3, 3, 3, 2];
/// Heavy: 10 startup, 4 active (drawings 03-04), 20 recovery.
const HEAVY_TIMELINE: [u16; 9] = [4, 3, 3, 2, 2, 5, 5, 5, 5];
/// Hit reactions: the impact pose for 4 ticks, then recover.
const HIT_HIGH_TIMELINE: [u16; 4] = [4, 4, 4, 3];
const HIT_MID_TIMELINE: [u16; 4] = [4, 6, 5, 5];
const KNOCKDOWN_TICKS_PER_DRAWING: u32 = 4;
const LAND_TICKS: u8 = 4;
const TURN_TICKS: u8 = 3;
/// Damage at or above this is from a heavy attack (light 50, heavy 120).
const HEAVY_DAMAGE: i16 = 100;
/// The damage bar holds the lost chunk this long, then drains.
const DAMAGE_HOLD_TICKS: u8 = 30;
const DAMAGE_DRAIN_PER_TICK: f32 = 8.0;
pub const SPARK_TICKS_PER_DRAWING: u32 = 2;
pub const DUST_TICKS_PER_DRAWING: u32 = 2;
/// How long the FIGHT banner stays up.
pub const FIGHT_BANNER_TICKS: u32 = 30;

/// One fighter's current sprite: animation folder name (under
/// fighter/p1/ or fighter/p2/), drawing index and how many drawings that
/// animation was designed with. If an artist delivers a different count,
/// the index is rescaled to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub anim: &'static str,
    pub index: usize,
    pub designed: usize,
}

impl Pose {
    fn new(anim: &'static str, index: usize, designed: usize) -> Pose {
        Pose {
            anim,
            index: index.min(designed - 1),
            designed,
        }
    }

    /// Index into an animation that actually has `len` drawings.
    pub fn index_for(&self, len: usize) -> usize {
        if len == self.designed {
            self.index
        } else {
            (self.index * len / self.designed).min(len.saturating_sub(1))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Puff {
    SparkLight,
    SparkHeavy,
    DustJump,
    DustLand,
}

/// A spark or dust cloud: where (logical pixels) and on which sim frame it
/// started. `facing` mirrors sparks to point away from the attacker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub kind: Puff,
    pub x: f32,
    pub y: f32,
    pub facing: i8,
    pub start: u32,
}

impl Particle {
    pub fn lifetime(&self) -> u32 {
        match self.kind {
            Puff::SparkLight => 4 * SPARK_TICKS_PER_DRAWING,
            Puff::SparkHeavy => 6 * SPARK_TICKS_PER_DRAWING,
            Puff::DustJump | Puff::DustLand => 3 * DUST_TICKS_PER_DRAWING,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Track {
    action: Action,
    /// Ticks since `action` started (as far as we've seen).
    ticks: u32,
    airborne: bool,
    facing: i8,
    land_left: u8,
    turn_left: u8,
    /// Ticks spent knocked down *on the floor* (a mid-air KO falls first).
    ko_ticks: u32,
    /// Health drawn on the damage bar: lags behind real health.
    shown_health: f32,
    damage_hold: u8,
    health: i16,
}

impl Default for Track {
    fn default() -> Track {
        Track {
            action: Action::Idle,
            ticks: 0,
            airborne: false,
            facing: 1,
            land_left: 0,
            turn_left: 0,
            ko_ticks: 0,
            shown_health: 1000.0,
            damage_hold: 0,
            health: 1000,
        }
    }
}

#[derive(Default)]
pub struct Animator {
    last_frame: Option<u32>,
    last_health: [i16; 2],
    last_phase_fight: bool,
    tracks: [Track; 2],
    pub particles: Vec<Particle>,
    /// Ticks since the round's fight phase began.
    pub fight_ticks: u32,
}

impl Animator {
    /// Call once per draw with the current sim state.
    pub fn observe(&mut self, game: &GameState) {
        let passed = match self.last_frame {
            Some(last) if game.frame >= last => game.frame - last,
            // First call, or the frame went backwards (restart): start over.
            _ => {
                *self = Animator::default();
                for (track, f) in self.tracks.iter_mut().zip(&game.fighters) {
                    track.action = f.action;
                    track.airborne = f.is_airborne();
                    track.facing = f.facing;
                    track.shown_health = f32::from(f.health);
                    track.health = f.health;
                }
                self.last_frame = Some(game.frame);
                self.last_health = [game.fighters[0].health, game.fighters[1].health];
                self.last_phase_fight = game.round.phase == Phase::Fight;
                return;
            }
        };

        // Hits: a spark where the attack met the body.
        for defender in 0..2 {
            let damage = self.last_health[defender] - game.fighters[defender].health;
            if damage > 0 {
                let attacker = &game.fighters[1 - defender];
                self.particles.push(spark(
                    &game.fighters[defender],
                    attacker,
                    damage,
                    game.frame,
                ));
            }
        }

        for (track, f) in self.tracks.iter_mut().zip(&game.fighters) {
            step_track(track, f, passed, game.frame, &mut self.particles);
        }

        let fighting = game.round.phase == Phase::Fight;
        if fighting && !self.last_phase_fight {
            self.fight_ticks = 0;
        } else {
            self.fight_ticks = self.fight_ticks.saturating_add(passed);
        }
        self.last_phase_fight = fighting;

        let frame = game.frame;
        self.particles
            .retain(|p| frame.saturating_sub(p.start) < p.lifetime());

        self.last_frame = Some(game.frame);
        self.last_health = [game.fighters[0].health, game.fighters[1].health];
    }

    /// Health to draw on the trailing damage bar for `player`.
    pub fn shown_health(&self, player: usize) -> f32 {
        self.tracks[player].shown_health
    }

    /// Which drawing `player` shows this frame.
    pub fn pose(&self, game: &GameState, player: usize) -> Pose {
        pose_for(&game.fighters[player], &self.tracks[player], game, player)
    }
}

fn step_track(t: &mut Track, f: &Fighter, passed: u32, frame: u32, particles: &mut Vec<Particle>) {
    let airborne = f.is_airborne();
    let x = px(f.x);

    if f.action != t.action {
        t.action = f.action;
        t.ticks = 0;
    } else {
        t.ticks = t.ticks.saturating_add(passed);
    }

    // Takeoff and landing puffs, and the short landing squash.
    if airborne && !t.airborne && f.action == Action::Jump {
        particles.push(dust(Puff::DustJump, x, frame));
    }
    if !airborne && t.airborne {
        particles.push(dust(Puff::DustLand, x, frame));
        if f.is_actionable() {
            t.land_left = LAND_TICKS;
        }
    } else {
        t.land_left = t.land_left.saturating_sub(passed.min(255) as u8);
    }
    if !f.is_actionable() {
        t.land_left = 0;
    }

    // Facing flipped while free to move: a quick cosmetic turn.
    if f.facing != t.facing && f.is_actionable() {
        t.turn_left = TURN_TICKS;
    } else {
        t.turn_left = t.turn_left.saturating_sub(passed.min(255) as u8);
    }
    if !matches!(f.action, Action::Idle | Action::Walk) {
        t.turn_left = 0;
    }

    if f.action == Action::Knockdown && !airborne {
        let before = t.ko_ticks;
        t.ko_ticks = t.ko_ticks.saturating_add(passed);
        // Body hits the floor on knockdown drawing 03: a dust puff.
        let impact = 3 * KNOCKDOWN_TICKS_PER_DRAWING;
        if before < impact && t.ko_ticks >= impact {
            particles.push(dust(Puff::DustLand, x - 40.0 * f32::from(f.facing), frame));
        }
    } else if f.action != Action::Knockdown {
        t.ko_ticks = 0;
    }

    // Damage bar: jumps up on a heal (round reset), holds the lost chunk
    // for a moment after each hit, then drains down to real health.
    let health = f32::from(f.health);
    if health >= t.shown_health {
        t.shown_health = health;
        t.damage_hold = 0;
    } else if f.health < t.health {
        t.damage_hold = DAMAGE_HOLD_TICKS;
    } else if t.damage_hold > 0 {
        t.damage_hold = t.damage_hold.saturating_sub(passed.min(255) as u8);
    } else {
        t.shown_health = (t.shown_health - DAMAGE_DRAIN_PER_TICK * passed as f32).max(health);
    }
    t.health = f.health;

    t.airborne = airborne;
    t.facing = f.facing;
}

fn pose_for(f: &Fighter, t: &Track, game: &GameState, player: usize) -> Pose {
    let airborne = f.is_airborne();
    match f.action {
        // The special moves reuse the light and heavy drawings until they
        // get their own art: the dynamite punch is a punch, the super kick
        // and the nado are kicks.
        Action::Light | Action::Dynamite => {
            Pose::new("light", timeline(&LIGHT_TIMELINE, f.action_frame), 6)
        }
        Action::Heavy | Action::SuperKick | Action::Nado => {
            Pose::new("heavy", timeline(&HEAVY_TIMELINE, f.action_frame), 9)
        }
        Action::Jump => {
            // Ticks since takeoff, straight from the vertical speed:
            // vy = JUMP_VELOCITY + GRAVITY * t.
            let t = ((f.vy - JUMP_VELOCITY) / GRAVITY).max(0);
            let index = match t {
                0..=8 => 0,
                9..=17 => 1,
                18..=23 => 2,
                24..=32 => 3,
                _ => 4,
            };
            Pose::new("jump", index, 5)
        }
        Action::Hitstun if airborne => Pose::new("hit_air", usize::from(t.ticks >= 3), 2),
        Action::Hitstun => {
            // The sim doesn't record which attack hit, but the total
            // hitstun does: light gives 15 ticks, heavy 20.
            let total = u32::from(f.hitstun) + t.ticks;
            let frame = t.ticks.min(u32::from(u16::MAX)) as u16;
            if total > u32::from(LIGHT_ATTACK.hitstun + HEAVY_ATTACK.hitstun) / 2 {
                Pose::new("hit_mid", timeline(&HIT_MID_TIMELINE, frame), 4)
            } else {
                Pose::new("hit_high", timeline(&HIT_HIGH_TIMELINE, frame), 4)
            }
        }
        Action::Knockdown if airborne => Pose::new("hit_air", 1, 2),
        Action::Knockdown => {
            let index = (t.ko_ticks / KNOCKDOWN_TICKS_PER_DRAWING) as usize;
            if index < 6 {
                Pose::new("knockdown", index, 6)
            } else {
                // Lying still, breathing slowly.
                Pose::new("down", ((t.ko_ticks / 30) % 2) as usize, 2)
            }
        }
        Action::Crouch => Pose::new("crouch", t.ticks.min(2) as usize, 3),
        Action::Walk if t.turn_left > 0 => turn_pose(t),
        Action::Walk => {
            // 8 drawings, 4 ticks each, for 128 px per cycle at 4 px/tick.
            // Walking back (3 px/tick) plays backwards and slower.
            let forward = f.vx.signum() == i32::from(f.facing);
            let index = if forward {
                (t.ticks / 4) % 8
            } else {
                7 - ((t.ticks * 3 / 16) % 8)
            };
            Pose::new("walk", index as usize, 8)
        }
        Action::Idle => idle_pose(f, t, game, player),
    }
}

fn idle_pose(_f: &Fighter, t: &Track, game: &GameState, player: usize) -> Pose {
    let round = &game.round;
    match round.phase {
        Phase::Ready => {
            let elapsed = READY_TICKS.saturating_sub(round.timer);
            return Pose::new("intro", usize::from(elapsed / 10), 6);
        }
        Phase::RoundOver { winner: Some(w) } if usize::from(w) == player => {
            let elapsed = (ROUND_OVER_TICKS + 1).saturating_sub(round.timer);
            return Pose::new("win", usize::from(elapsed / 8), 6);
        }
        Phase::MatchOver { winner } if usize::from(winner) == player => {
            return Pose::new("win", 5, 6);
        }
        _ => {}
    }
    if t.land_left > 0 {
        return Pose::new("land", usize::from(t.land_left <= LAND_TICKS / 2), 2);
    }
    if t.turn_left > 0 {
        return turn_pose(t);
    }
    // 8 drawings, 7 ticks each: a 56-tick breathing cycle, from the global
    // frame. P2 is offset so the two don't breathe in lockstep.
    let tick = game.frame + player as u32 * 23;
    Pose::new("idle", ((tick / 7) % 8) as usize, 8)
}

fn turn_pose(t: &Track) -> Pose {
    Pose::new("turn", usize::from(t.turn_left <= 1), 2)
}

/// Which drawing of a fixed timeline `frame` (ticks since start) falls on.
fn timeline(ticks_per_drawing: &[u16], frame: u16) -> usize {
    let mut end = 0;
    for (i, &n) in ticks_per_drawing.iter().enumerate() {
        end += n;
        if frame < end {
            return i;
        }
    }
    ticks_per_drawing.len() - 1
}

fn spark(defender: &Fighter, attacker: &Fighter, damage: i16, frame: u32) -> Particle {
    let heavy = damage >= HEAVY_DAMAGE;
    let attack = if heavy { HEAVY_ATTACK } else { LIGHT_ATTACK };
    let facing = attacker.facing;
    // Where the hitbox meets the near edge of the defender's body, at the
    // middle of the attack's height band.
    let body = defender.hurtbox();
    let edge = if facing > 0 { body.left } else { body.right };
    let height = (attack.height_low + attack.height_high) / 2;
    Particle {
        kind: if heavy {
            Puff::SparkHeavy
        } else {
            Puff::SparkLight
        },
        x: px(edge) + 8.0 * f32::from(facing),
        y: px(attacker.y - height),
        facing,
        start: frame,
    }
}

fn dust(kind: Puff, x: f32, frame: u32) -> Particle {
    Particle {
        kind,
        x,
        y: px(FLOOR_Y),
        facing: 1,
        start: frame,
    }
}

fn px(subpixels: i32) -> f32 {
    subpixels as f32 / SUBPIXELS as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::fighting_state;

    #[test]
    fn light_timeline_matches_frame_data() {
        let total: u16 = LIGHT_TIMELINE.iter().sum();
        assert_eq!(total, LIGHT_ATTACK.total());
        // The reach drawing (02) covers exactly the active ticks.
        for frame in 0..LIGHT_ATTACK.total() {
            let reach = timeline(&LIGHT_TIMELINE, frame) == 2;
            assert_eq!(reach, LIGHT_ATTACK.is_active(frame), "tick {frame}");
        }
    }

    #[test]
    fn heavy_timeline_matches_frame_data() {
        let total: u16 = HEAVY_TIMELINE.iter().sum();
        assert_eq!(total, HEAVY_ATTACK.total());
        for frame in 0..HEAVY_ATTACK.total() {
            let i = timeline(&HEAVY_TIMELINE, frame);
            let reach = i == 3 || i == 4;
            assert_eq!(reach, HEAVY_ATTACK.is_active(frame), "tick {frame}");
        }
    }

    #[test]
    fn hit_timelines_fit_hitstun() {
        let high: u16 = HIT_HIGH_TIMELINE.iter().sum();
        let mid: u16 = HIT_MID_TIMELINE.iter().sum();
        assert_eq!(high, LIGHT_ATTACK.hitstun);
        assert_eq!(mid, HEAVY_ATTACK.hitstun);
    }

    #[test]
    fn rescaled_index_stays_in_range() {
        let p = Pose::new("heavy", 8, 9);
        assert_eq!(p.index_for(9), 8);
        assert_eq!(p.index_for(4), 3);
        assert_eq!(p.index_for(1), 0);
    }

    #[test]
    fn jump_drawings_follow_the_arc() {
        let mut game = fighting_state();
        let mut anim = Animator::default();
        anim.observe(&game);
        // Hold UP for P1 and watch the drawings advance through the jump.
        let mut seen = Vec::new();
        for _ in 0..41 {
            game.advance([crate::sim::UP, 0]);
            anim.observe(&game);
            let pose = anim.pose(&game, 0);
            if pose.anim == "jump" && seen.last() != Some(&pose.index) {
                seen.push(pose.index);
            }
        }
        assert_eq!(seen, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn observing_never_changes_the_game() {
        let mut game = fighting_state();
        let mut anim = Animator::default();
        let before = game.checksum();
        anim.observe(&game);
        let _ = anim.pose(&game, 0);
        assert_eq!(game.checksum(), before);
        game.advance([crate::sim::LIGHT, 0]);
        anim.observe(&game);
        assert_eq!(anim.pose(&game, 0).anim, "light");
    }
}
