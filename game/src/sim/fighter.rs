//! One fighter's movement: walking, crouching, jumping, gravity, floor, walls
//! and facing. All positions and speeds are integer subpixels.

use super::{is_held, Input, DOWN, LEFT, RIGHT, UP};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Idle,
    Walk,
    Crouch,
    Jump,
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
        }
    }

    pub fn is_airborne(&self) -> bool {
        self.action == Action::Jump
    }

    pub fn height(&self) -> i32 {
        if self.action == Action::Crouch {
            CROUCH_HEIGHT
        } else {
            STAND_HEIGHT
        }
    }

    /// Turn to face the opponent. Only on the ground: fighters keep their
    /// facing through a jump, so jumping over someone doesn't flip you mid-air.
    pub fn face(&mut self, opponent_x: i32) {
        if self.is_airborne() {
            return;
        }
        if opponent_x > self.x {
            self.facing = 1;
        } else if opponent_x < self.x {
            self.facing = -1;
        }
        // Exactly equal x: keep the old facing rather than pick a side.
    }

    /// One tick of movement: read input (on the ground only), then move,
    /// then apply gravity, the floor and the walls.
    pub fn step(&mut self, input: Input) {
        if !self.is_airborne() {
            self.ground_control(input);
        }

        self.x += self.vx;
        self.y += self.vy;
        if self.is_airborne() {
            self.vy += GRAVITY;
        }

        if self.y >= FLOOR_Y {
            self.y = FLOOR_Y;
            if self.is_airborne() {
                self.land();
            }
        }

        let half = FIGHTER_WIDTH / 2;
        self.x = self.x.clamp(half, STAGE_WIDTH - half);
    }

    fn ground_control(&mut self, input: Input) {
        let dir = horizontal(input);
        if vertical(input) < 0 {
            // No air control: the horizontal speed is fixed at takeoff.
            self.action = Action::Jump;
            self.vx = dir * JUMP_X_SPEED;
            self.vy = JUMP_VELOCITY;
        } else if vertical(input) > 0 {
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

    fn land(&mut self) {
        self.action = Action::Idle;
        self.vx = 0;
        self.vy = 0;
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
        f.step(RIGHT);
        assert_eq!(f.x, MID + WALK_FORWARD_SPEED);
        assert_eq!(f.action, Action::Walk);

        let mut f = Fighter::new(MID, 1);
        f.step(LEFT);
        assert_eq!(f.x, MID - WALK_BACK_SPEED);
    }

    #[test]
    fn releasing_stops_immediately() {
        let mut f = Fighter::new(MID, 1);
        f.step(RIGHT);
        f.step(0);
        assert_eq!(f.x, MID + WALK_FORWARD_SPEED);
        assert_eq!(f.action, Action::Idle);
    }

    #[test]
    fn left_and_right_together_is_neutral() {
        let mut f = Fighter::new(MID, 1);
        f.step(LEFT | RIGHT);
        assert_eq!(f.x, MID);
        assert_eq!(f.action, Action::Idle);
    }

    #[test]
    fn standing_still_on_the_floor_has_no_gravity() {
        let mut f = Fighter::new(MID, 1);
        for _ in 0..100 {
            f.step(0);
        }
        assert_eq!((f.y, f.vy), (FLOOR_Y, 0));
    }

    #[test]
    fn jump_leaves_floor_and_lands_after_jump_ticks() {
        let mut f = Fighter::new(MID, 1);
        // UP only matters on the takeoff tick; after that input is ignored.
        f.step(UP);
        assert!(f.is_airborne());
        assert!(f.y < FLOOR_Y);
        let mut highest = f.y;
        for _ in 2..JUMP_TICKS {
            f.step(0);
            assert!(f.is_airborne());
            highest = highest.min(f.y);
        }
        f.step(0);
        assert_eq!(f.y, FLOOR_Y);
        assert_eq!(f.action, Action::Idle);
        // 157.5 px peak, see JUMP_VELOCITY.
        assert_eq!(FLOOR_Y - highest, 40_320);
    }

    #[test]
    fn no_air_control() {
        let mut f = Fighter::new(MID, 1);
        f.step(UP | RIGHT);
        let vx = f.vx;
        assert_eq!(vx, JUMP_X_SPEED);
        for _ in 0..10 {
            f.step(LEFT);
            assert_eq!(f.vx, vx);
        }
    }

    #[test]
    fn crouch_stops_walking() {
        let mut f = Fighter::new(MID, 1);
        f.step(DOWN | RIGHT);
        assert_eq!(f.x, MID);
        assert_eq!(f.action, Action::Crouch);
        assert_eq!(f.height(), CROUCH_HEIGHT);
    }

    #[test]
    fn walls_stop_fighters_at_both_edges() {
        let half = FIGHTER_WIDTH / 2;
        let mut f = Fighter::new(half + 1, -1);
        for _ in 0..10 {
            f.step(LEFT);
        }
        assert_eq!(f.x, half);

        let mut f = Fighter::new(STAGE_WIDTH - half - 1, 1);
        for _ in 0..10 {
            f.step(RIGHT);
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

        f.step(UP);
        f.face(MID + 100);
        assert_eq!(f.facing, -1, "no turning in the air");
    }
}
