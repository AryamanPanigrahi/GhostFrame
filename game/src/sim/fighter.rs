//! One fighter: movement (walk, crouch, jump, gravity, floor, walls, facing)
//! and the action state machine (attacks, hitstun). All positions and speeds
//! are integer subpixels.

use super::attack::{Attack, HEAVY_ATTACK, LIGHT_ATTACK};
use super::checksum::Fnv1a;
use super::rect::Rect;
use super::{is_held, Input, DOWN, HEAVY, LEFT, LIGHT, RIGHT, UP};

/// 1 pixel = 256 subpixels. Speeds like 3.5 px/tick become whole numbers
/// (896) without floats.
pub const SUBPIXELS: i32 = 256;

/// Logical stage width (DESIGN.md section 7). y grows downward, like the
/// screen, so "up" is negative.
pub const STAGE_WIDTH: i32 = 1280 * SUBPIXELS;
/// y of the floor surface: 80 px from the bottom of the 720 px stage.
pub const FLOOR_Y: i32 = 640 * SUBPIXELS;

pub const FIGHTER_WIDTH: i32 = 80 * SUBPIXELS;
pub const STAND_HEIGHT: i32 = 200 * SUBPIXELS;
pub const CROUCH_HEIGHT: i32 = 120 * SUBPIXELS;

/// Horizontal speeds, subpixels per tick. Walking backward is slower, a
/// fighting-game convention: retreating should cost something.
pub const WALK_FORWARD_SPEED: i32 = 4 * SUBPIXELS;
pub const WALK_BACK_SPEED: i32 = 3 * SUBPIXELS;
pub const JUMP_X_SPEED: i32 = 4 * SUBPIXELS;

/// Upward speed at takeoff and downward acceleration, per tick. With
/// "move, then apply gravity" these give a jump that lasts exactly 41 ticks
/// (about 0.68 s) and peaks 157.5 px above the floor.
pub const JUMP_VELOCITY: i32 = -15 * SUBPIXELS;
pub const GRAVITY: i32 = 192;

pub const MAX_HEALTH: i16 = 1000;

/// How much knockback speed is lost per tick of hitstun (subpixels/tick).
pub const KNOCKBACK_FRICTION: i32 = 64;

/// What the fighter is doing. Only Idle, Walk and Crouch accept new input;
/// every other action has to finish (or be interrupted by a hit) first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Idle,
    Walk,
    Crouch,
    Jump,
    Light,
    Heavy,
    Hitstun,
    /// KO'd: down for the rest of the round, no input.
    Knockdown,
}

impl Action {
    /// A fixed number per variant for the checksum. Written out rather than
    /// using `as u8`, so reordering the enum can't silently change hashes.
    fn tag(self) -> u8 {
        match self {
            Action::Idle => 0,
            Action::Walk => 1,
            Action::Crouch => 2,
            Action::Jump => 3,
            Action::Light => 4,
            Action::Heavy => 5,
            Action::Hitstun => 6,
            Action::Knockdown => 7,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fighter {
    /// Centre of the fighter's feet, in subpixels.
    pub x: i32,
    pub y: i32,
    /// Velocity in subpixels per tick.
    pub vx: i32,
    pub vy: i32,
    /// +1 facing right, -1 facing left.
    pub facing: i8,
    pub action: Action,
    /// Ticks since the current attack started (0 on its first tick).
    pub action_frame: u16,
    /// Ticks of hitstun left.
    pub hitstun: u16,
    pub health: i16,
    /// The current attack already hit, so its remaining active frames
    /// mustn't hit again: one swing, one hit.
    pub attack_connected: bool,
}

impl Fighter {
    pub fn new(x: i32, facing: i8) -> Fighter {
        Fighter {
            x,
            y: FLOOR_Y,
            vx: 0,
            vy: 0,
            facing,
            action: Action::Idle,
            action_frame: 0,
            hitstun: 0,
            health: MAX_HEALTH,
            attack_connected: false,
        }
    }

    /// Based on position, not action: a fighter hit out of a jump is in
    /// Hitstun but still has to fall.
    pub fn is_airborne(&self) -> bool {
        self.y < FLOOR_Y
    }

    /// Free to walk, crouch, jump or start an attack this tick.
    pub fn is_actionable(&self) -> bool {
        let free = matches!(self.action, Action::Idle | Action::Walk | Action::Crouch);
        free && !self.is_airborne()
    }

    /// Frame data of the attack in progress, if any.
    pub fn current_attack(&self) -> Option<Attack> {
        match self.action {
            Action::Light => Some(LIGHT_ATTACK),
            Action::Heavy => Some(HEAVY_ATTACK),
            _ => None,
        }
    }

    /// The area where this fighter can be hit: their whole body.
    pub fn hurtbox(&self) -> Rect {
        let half = FIGHTER_WIDTH / 2;
        Rect {
            left: self.x - half,
            top: self.y - self.height(),
            right: self.x + half,
            bottom: self.y,
        }
    }

    /// The hitbox of the current attack, only during its active frames.
    pub fn active_hitbox(&self) -> Option<Rect> {
        let attack = self.current_attack()?;
        if attack.is_active(self.action_frame) {
            Some(attack.hitbox(self.x, self.y, self.facing))
        } else {
            None
        }
    }

    pub fn height(&self) -> i32 {
        if self.action == Action::Crouch {
            CROUCH_HEIGHT
        } else {
            STAND_HEIGHT
        }
    }

    /// Turn to face the opponent, but only when actionable: fighters keep
    /// their facing through a jump (no flipping mid-air), an attack (the
    /// hitbox can't swap sides mid-swing) and hitstun.
    pub fn face(&mut self, opponent_x: i32) {
        if !self.is_actionable() {
            return;
        }
        if opponent_x > self.x {
            self.facing = 1;
        } else if opponent_x < self.x {
            self.facing = -1;
        }
        // Exactly equal x: keep the old facing rather than pick a side.
    }

    /// One tick: count down the current action, read input if actionable,
    /// then move and apply gravity, the floor and the walls.
    ///
    /// `held` is every button down this tick; `pressed` is only the ones that
    /// weren't down last tick. Attacks start on a press, so holding a button
    /// doesn't repeat the attack.
    pub fn step(&mut self, held: Input, pressed: Input) {
        self.tick_action();
        if self.is_actionable() {
            self.ground_control(held, pressed);
        }

        self.x += self.vx;
        self.y += self.vy;
        if self.is_airborne() {
            self.vy += GRAVITY;
        }

        if self.y >= FLOOR_Y {
            self.y = FLOOR_Y;
            self.vy = 0;
            if self.action == Action::Jump {
                self.land();
            }
        }

        let half = FIGHTER_WIDTH / 2;
        self.x = self.x.clamp(half, STAGE_WIDTH - half);
    }

    /// Advance attack and hitstun timers. Either one ending makes the fighter
    /// Idle, and so actionable again on this same tick.
    fn tick_action(&mut self) {
        if let Some(attack) = self.current_attack() {
            self.action_frame += 1;
            if self.action_frame >= attack.total() {
                self.action = Action::Idle;
            }
        } else if self.action == Action::Knockdown {
            self.vx = toward_zero(self.vx, KNOCKBACK_FRICTION);
        } else if self.action == Action::Hitstun {
            self.hitstun -= 1;
            self.vx = toward_zero(self.vx, KNOCKBACK_FRICTION);
            if self.hitstun == 0 {
                self.action = Action::Idle;
                self.vx = 0;
            }
        }
    }

    fn ground_control(&mut self, held: Input, pressed: Input) {
        // Attacks beat movement: pressing LIGHT while walking attacks.
        if is_held(pressed, LIGHT) {
            self.start_attack(Action::Light);
            return;
        }
        if is_held(pressed, HEAVY) {
            self.start_attack(Action::Heavy);
            return;
        }

        let dir = horizontal(held);
        if vertical(held) < 0 {
            // No air control: the horizontal speed is fixed at takeoff.
            self.action = Action::Jump;
            self.vx = dir * JUMP_X_SPEED;
            self.vy = JUMP_VELOCITY;
        } else if vertical(held) > 0 {
            self.action = Action::Crouch;
            self.vx = 0;
        } else if dir != 0 {
            self.action = Action::Walk;
            self.vx = if dir == self.facing as i32 {
                dir * WALK_FORWARD_SPEED
            } else {
                dir * WALK_BACK_SPEED
            };
        } else {
            self.action = Action::Idle;
            self.vx = 0;
        }
    }

    /// Attacks root the fighter in place for their whole duration.
    fn start_attack(&mut self, action: Action) {
        self.action = action;
        self.action_frame = 0;
        self.attack_connected = false;
        self.vx = 0;
    }

    /// Called when `attack` from an attacker facing `from_facing` connects.
    /// Interrupts whatever this fighter was doing, including its own attack.
    pub fn take_hit(&mut self, attack: Attack, from_facing: i8) {
        self.health = (self.health - attack.damage).max(0);
        self.action = Action::Hitstun;
        self.hitstun = attack.hitstun;
        self.vx = from_facing as i32 * attack.knockback;
    }

    /// Health reached 0. Keeps any knockback slide; stays down until the
    /// next round resets the fighter.
    pub fn knock_down(&mut self) {
        self.action = Action::Knockdown;
        self.hitstun = 0;
    }

    /// Feed every field into the hash. The destructuring `let` lists all
    /// fields by name, so adding a field to `Fighter` without hashing it is
    /// a compile error instead of a silent gap in desync detection.
    pub fn hash_into(&self, h: &mut Fnv1a) {
        let Fighter {
            x,
            y,
            vx,
            vy,
            facing,
            action,
            action_frame,
            hitstun,
            health,
            attack_connected,
        } = *self;
        h.write_i32(x);
        h.write_i32(y);
        h.write_i32(vx);
        h.write_i32(vy);
        h.write_i8(facing);
        h.write_u8(action.tag());
        h.write_u16(action_frame);
        h.write_u16(hitstun);
        h.write_i16(health);
        h.write_bool(attack_connected);
    }

    fn land(&mut self) {
        self.action = Action::Idle;
        self.vx = 0;
        self.vy = 0;
    }
}

/// Move `v` toward 0 by `amount` without overshooting past 0.
fn toward_zero(v: i32, amount: i32) -> i32 {
    if v > 0 {
        (v - amount).max(0)
    } else {
        (v + amount).min(0)
    }
}

/// -1 for left, +1 for right, 0 for neither *or both*. Treating opposite
/// directions as neutral (SOCD cleaning) stops a player holding both from
/// getting an undefined result.
fn horizontal(input: Input) -> i32 {
    is_held(input, RIGHT) as i32 - is_held(input, LEFT) as i32
}

/// -1 for up, +1 for down (y grows downward), 0 for neither or both.
fn vertical(input: Input) -> i32 {
    is_held(input, DOWN) as i32 - is_held(input, UP) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const MID: i32 = STAGE_WIDTH / 2;
    /// Height after n airborne ticks is 3840n - 96n(n-1) subpixels, which is
    /// back to 0 at n = 41.
    const JUMP_TICKS: u32 = 41;

    #[test]
    fn walking_forward_is_faster_than_back() {
        let mut f = Fighter::new(MID, 1);
        f.step(RIGHT, 0);
        assert_eq!(f.x, MID + WALK_FORWARD_SPEED);
        assert_eq!(f.action, Action::Walk);

        let mut f = Fighter::new(MID, 1);
        f.step(LEFT, 0);
        assert_eq!(f.x, MID - WALK_BACK_SPEED);
    }

    #[test]
    fn releasing_stops_immediately() {
        let mut f = Fighter::new(MID, 1);
        f.step(RIGHT, 0);
        f.step(0, 0);
        assert_eq!(f.x, MID + WALK_FORWARD_SPEED);
        assert_eq!(f.action, Action::Idle);
    }

    #[test]
    fn left_and_right_together_is_neutral() {
        let mut f = Fighter::new(MID, 1);
        f.step(LEFT | RIGHT, 0);
        assert_eq!(f.x, MID);
        assert_eq!(f.action, Action::Idle);
    }

    #[test]
    fn standing_still_on_the_floor_has_no_gravity() {
        let mut f = Fighter::new(MID, 1);
        for _ in 0..100 {
            f.step(0, 0);
        }
        assert_eq!((f.y, f.vy), (FLOOR_Y, 0));
    }

    #[test]
    fn jump_leaves_floor_and_lands_after_jump_ticks() {
        let mut f = Fighter::new(MID, 1);
        // UP only matters on the takeoff tick; after that input is ignored.
        f.step(UP, 0);
        assert!(f.is_airborne());
        assert!(f.y < FLOOR_Y);
        let mut highest = f.y;
        for _ in 2..JUMP_TICKS {
            f.step(0, 0);
            assert!(f.is_airborne());
            highest = highest.min(f.y);
        }
        f.step(0, 0);
        assert_eq!(f.y, FLOOR_Y);
        assert_eq!(f.action, Action::Idle);
        // 157.5 px peak, see JUMP_VELOCITY.
        assert_eq!(FLOOR_Y - highest, 40_320);
    }

    #[test]
    fn no_air_control() {
        let mut f = Fighter::new(MID, 1);
        f.step(UP | RIGHT, 0);
        let vx = f.vx;
        assert_eq!(vx, JUMP_X_SPEED);
        for _ in 0..10 {
            f.step(LEFT, 0);
            assert_eq!(f.vx, vx);
        }
    }

    #[test]
    fn crouch_stops_walking() {
        let mut f = Fighter::new(MID, 1);
        f.step(DOWN | RIGHT, 0);
        assert_eq!(f.x, MID);
        assert_eq!(f.action, Action::Crouch);
        assert_eq!(f.height(), CROUCH_HEIGHT);
    }

    #[test]
    fn walls_stop_fighters_at_both_edges() {
        let half = FIGHTER_WIDTH / 2;
        let mut f = Fighter::new(half + 1, -1);
        for _ in 0..10 {
            f.step(LEFT, 0);
        }
        assert_eq!(f.x, half);

        let mut f = Fighter::new(STAGE_WIDTH - half - 1, 1);
        for _ in 0..10 {
            f.step(RIGHT, 0);
        }
        assert_eq!(f.x, STAGE_WIDTH - half);
    }

    #[test]
    fn faces_opponent_on_ground_only() {
        let mut f = Fighter::new(MID, 1);
        f.face(MID - 100);
        assert_eq!(f.facing, -1);
        f.face(MID);
        assert_eq!(f.facing, -1, "equal x keeps old facing");

        f.step(UP, 0);
        f.face(MID + 100);
        assert_eq!(f.facing, -1, "no turning in the air");
    }

    use crate::sim::attack::LIGHT_ATTACK;

    #[test]
    fn attack_starts_on_press_and_roots_in_place() {
        let mut f = Fighter::new(MID, 1);
        f.step(LIGHT | RIGHT, LIGHT);
        assert_eq!(f.action, Action::Light);
        assert_eq!(f.x, MID);
    }

    #[test]
    fn holding_the_button_does_not_start_an_attack() {
        let mut f = Fighter::new(MID, 1);
        f.step(LIGHT, 0);
        assert_eq!(f.action, Action::Idle);
    }

    #[test]
    fn attack_lasts_exactly_its_total_frames() {
        let mut f = Fighter::new(MID, 1);
        f.step(LIGHT, LIGHT);
        for _ in 1..LIGHT_ATTACK.total() {
            f.step(RIGHT, 0);
            assert_eq!(f.action, Action::Light);
        }
        // Free again: this tick's input is used straight away.
        f.step(RIGHT, 0);
        assert_eq!(f.action, Action::Walk);
    }

    #[test]
    fn hitbox_exists_only_on_active_frames() {
        let mut f = Fighter::new(MID, 1);
        f.step(LIGHT, LIGHT);
        let mut active = vec![];
        for frame in 0..LIGHT_ATTACK.total() {
            if f.active_hitbox().is_some() {
                active.push(frame);
            }
            f.step(0, 0);
        }
        assert_eq!(active, vec![4, 5, 6]);
    }

    #[test]
    fn hitstun_blocks_input_then_ends() {
        let mut f = Fighter::new(MID, -1);
        f.take_hit(LIGHT_ATTACK, 1);
        assert_eq!(f.health, MAX_HEALTH - LIGHT_ATTACK.damage);
        for _ in 1..LIGHT_ATTACK.hitstun {
            f.step(LEFT, LIGHT);
            assert_eq!(f.action, Action::Hitstun);
        }
        f.step(LEFT, LIGHT);
        assert_eq!(f.action, Action::Light, "acts on the tick hitstun ends");
    }

    #[test]
    fn knockback_slows_down_during_hitstun() {
        let mut f = Fighter::new(MID, -1);
        f.take_hit(LIGHT_ATTACK, 1);
        let mut last_vx = f.vx;
        assert!(last_vx > 0);
        for _ in 1..LIGHT_ATTACK.hitstun {
            f.step(0, 0);
            assert!(f.vx <= last_vx && f.vx >= 0);
            last_vx = f.vx;
        }
        assert!(f.x > MID);
    }

    #[test]
    fn hit_in_the_air_still_falls() {
        let mut f = Fighter::new(MID, 1);
        f.step(UP, 0);
        f.take_hit(LIGHT_ATTACK, -1);
        for _ in 0..60 {
            f.step(0, 0);
        }
        assert_eq!(f.y, FLOOR_Y);
        assert!(f.is_actionable());
    }

    #[test]
    fn health_never_goes_below_zero() {
        let mut f = Fighter::new(MID, 1);
        f.health = 10;
        f.take_hit(LIGHT_ATTACK, 1);
        assert_eq!(f.health, 0);
    }

    #[test]
    fn knocked_down_fighter_cannot_act_or_turn() {
        let mut f = Fighter::new(MID, 1);
        f.knock_down();
        f.face(MID - 100);
        f.step(LEFT | UP, LIGHT);
        assert_eq!(f.action, Action::Knockdown);
        assert_eq!(f.facing, 1);
        assert_eq!(f.x, MID);
    }
}
