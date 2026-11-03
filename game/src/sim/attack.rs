//! Frame data: how long each attack takes, when it can hit, what it does on
//! hit, and where its hitbox is.

use super::fighter::SUBPIXELS;
use super::rect::Rect;

/// What a hit does to the defender besides damage and stun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitEffect {
    /// Pushed away horizontally at `knockback` speed.
    Push,
    /// Sent straight up (the super kick).
    Launch,
    /// Dragged across the stage to just in front of the attacker (the nado).
    Pull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attack {
    /// Ticks before the hitbox appears. The attacker is committed but harmless.
    pub startup: u16,
    /// Ticks the hitbox exists.
    pub active: u16,
    /// Ticks after the hitbox is gone before the attacker can act again. This
    /// is the window where a missed attack gets punished.
    pub recovery: u16,

    pub damage: i16,
    /// Ticks the defender can't act after being hit.
    pub hitstun: u16,
    /// Defender's horizontal speed (subpixels/tick) right after the hit,
    /// pushing them away from the attacker. Slows down during hitstun.
    pub knockback: i32,
    pub effect: HitEffect,

    /// Hitbox relative to the attacker's feet: `reach_*` is measured forward
    /// (in the direction they face), `height_*` upward from the floor.
    pub reach_near: i32,
    pub reach_far: i32,
    pub height_low: i32,
    pub height_high: i32,
}

/// Fast, short, weak, hits high: it passes over a crouching opponent
/// (crouch height is 120 px).
pub const LIGHT_ATTACK: Attack = Attack {
    startup: 4,
    active: 3,
    recovery: 8,
    damage: 50,
    hitstun: 15,
    knockback: 3 * SUBPIXELS,
    effect: HitEffect::Push,
    reach_near: 30 * SUBPIXELS,
    reach_far: 110 * SUBPIXELS,
    height_low: 130 * SUBPIXELS,
    height_high: 160 * SUBPIXELS,
};

/// Slow, long, strong, hits mid: it reaches crouching opponents too, but a
/// miss leaves a long recovery to punish.
pub const HEAVY_ATTACK: Attack = Attack {
    startup: 10,
    active: 4,
    recovery: 20,
    damage: 120,
    hitstun: 20,
    knockback: 6 * SUBPIXELS,
    effect: HitEffect::Push,
    reach_near: 30 * SUBPIXELS,
    reach_far: 140 * SUBPIXELS,
    height_low: 60 * SUBPIXELS,
    height_high: 110 * SUBPIXELS,
};

// The special moves (see combo.rs). Frame data is a first guess, like the
// normal attacks: every number here is meant to be tuned by playing.

/// Dynamite punch: blasts the defender across the stage. Hits mid, so it
/// reaches crouching opponents.
pub const DYNAMITE_ATTACK: Attack = Attack {
    startup: 14,
    active: 4,
    recovery: 28,
    damage: 150,
    hitstun: 40,
    knockback: 16 * SUBPIXELS,
    effect: HitEffect::Push,
    reach_near: 30 * SUBPIXELS,
    reach_far: 150 * SUBPIXELS,
    height_low: 60 * SUBPIXELS,
    height_high: 150 * SUBPIXELS,
};

/// Super kick: a low kick that launches the defender straight up. The long
/// hitstun keeps them helpless until they have landed.
pub const SUPER_KICK_ATTACK: Attack = Attack {
    startup: 16,
    active: 4,
    recovery: 30,
    damage: 200,
    hitstun: 80,
    knockback: 0,
    effect: HitEffect::Launch,
    reach_near: 30 * SUBPIXELS,
    reach_far: 140 * SUBPIXELS,
    height_low: 0,
    height_high: 120 * SUBPIXELS,
};

/// Nado: the hitbox reaches across the whole stage in front of the attacker,
/// and a hit drags the defender in. Little damage; the point is the pull.
/// Tall enough that jumping doesn't avoid it.
pub const NADO_ATTACK: Attack = Attack {
    startup: 18,
    active: 3,
    recovery: 22,
    damage: 60,
    hitstun: 50,
    knockback: 0,
    effect: HitEffect::Pull,
    reach_near: 0,
    reach_far: 1280 * SUBPIXELS,
    height_low: 0,
    height_high: 240 * SUBPIXELS,
};

impl Attack {
    /// Ticks from the first frame of the attack until the attacker is free.
    pub fn total(&self) -> u16 {
        self.startup + self.active + self.recovery
    }

    /// `frame` counts from 0 on the tick the attack started.
    pub fn is_active(&self, frame: u16) -> bool {
        frame >= self.startup && frame < self.startup + self.active
    }

    /// The hitbox in stage coordinates for an attacker standing at `(x, y)`.
    /// Facing left mirrors the forward reach to the other side.
    pub fn hitbox(&self, x: i32, y: i32, facing: i8) -> Rect {
        let (left, right) = if facing > 0 {
            (x + self.reach_near, x + self.reach_far)
        } else {
            (x - self.reach_far, x - self.reach_near)
        };
        Rect {
            left,
            top: y - self.height_high,
            right,
            bottom: y - self.height_low,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_only_between_startup_and_recovery() {
        let a = LIGHT_ATTACK;
        let active: Vec<u16> = (0..a.total()).filter(|&f| a.is_active(f)).collect();
        assert_eq!(active, vec![4, 5, 6]);
        assert!(!a.is_active(a.total()));
    }

    #[test]
    fn hitbox_mirrors_with_facing() {
        let a = LIGHT_ATTACK;
        let right = a.hitbox(0, 0, 1);
        let left = a.hitbox(0, 0, -1);
        assert_eq!(right.left, -left.right);
        assert_eq!(right.right, -left.left);
        assert_eq!((right.top, right.bottom), (left.top, left.bottom));
        assert!(
            right.left > 0,
            "hitbox is in front of a right-facing attacker"
        );
    }
}
