//! Deterministic simulation. No floats, no time, no window: given the same
//! state and the same inputs, `advance` must produce the same result on every
//! machine, because rollback re-runs it and both peers run it independently.

pub mod attack;
pub mod fighter;
pub mod rect;

use attack::Attack;
use fighter::{Fighter, SUBPIXELS};

/// One player's buttons for one tick, packed into bits (DESIGN.md section 4).
/// A u16 is tiny to send over the network and cheap to compare.
pub type Input = u16;

pub const LEFT: Input = 1 << 0;
pub const RIGHT: Input = 1 << 1;
pub const UP: Input = 1 << 2;
pub const DOWN: Input = 1 << 3;
pub const LIGHT: Input = 1 << 4;
pub const HEAVY: Input = 1 << 5;

/// True if `button` (one of the constants above) is set in `input`.
pub fn is_held(input: Input, button: Input) -> bool {
    input & button != 0
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameState {
    /// Number of ticks simulated so far. At 60 ticks/s this is also the clock.
    pub frame: u32,
    /// Inputs used on the most recent tick, to tell "just pressed" from
    /// "still held". Part of the state so rollback restores it too.
    pub inputs: [Input; 2],
    pub fighters: [Fighter; 2],
}

impl Default for GameState {
    /// Round start: both fighters on the floor, 480 px apart, facing each other.
    fn default() -> GameState {
        GameState {
            frame: 0,
            inputs: [0, 0],
            fighters: [
                Fighter::new(400 * SUBPIXELS, 1),
                Fighter::new(880 * SUBPIXELS, -1),
            ],
        }
    }
}

impl GameState {
    /// Advance exactly one tick (1/60 s).
    pub fn advance(&mut self, inputs: [Input; 2]) {
        // Both fighters turn using positions from the *start* of the tick,
        // before either moves. That way updating player 0 first can't give
        // player 0 an advantage: the result doesn't depend on update order.
        let x0 = self.fighters[0].x;
        let x1 = self.fighters[1].x;
        self.fighters[0].face(x1);
        self.fighters[1].face(x0);

        // Pressed = down now and not down last tick.
        let pressed0 = inputs[0] & !self.inputs[0];
        let pressed1 = inputs[1] & !self.inputs[1];
        self.fighters[0].step(inputs[0], pressed0);
        self.fighters[1].step(inputs[1], pressed1);

        // Check both hits before applying either. If P1's hit were applied
        // first, P2 would already be in hitstun and their own hit would be
        // cancelled: P1 would always win ties. This way both land (a trade).
        let hit_on_1 = check_hit(&self.fighters[0], &self.fighters[1]);
        let hit_on_0 = check_hit(&self.fighters[1], &self.fighters[0]);
        if let Some(attack) = hit_on_1 {
            self.fighters[0].attack_connected = true;
            self.fighters[1].take_hit(attack, self.fighters[0].facing);
        }
        if let Some(attack) = hit_on_0 {
            self.fighters[1].attack_connected = true;
            self.fighters[0].take_hit(attack, self.fighters[1].facing);
        }

        self.frame += 1;
        self.inputs = inputs;
    }
}

/// The attack that `attacker` lands on `defender` this tick, if any: an
/// active hitbox overlapping the defender's hurtbox, on a swing that hasn't
/// already hit.
fn check_hit(attacker: &Fighter, defender: &Fighter) -> Option<Attack> {
    if attacker.attack_connected {
        return None;
    }
    let hitbox = attacker.active_hitbox()?;
    if hitbox.overlaps(&defender.hurtbox()) {
        attacker.current_attack()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_use_distinct_bits() {
        let all = [LEFT, RIGHT, UP, DOWN, LIGHT, HEAVY];
        for (i, a) in all.iter().enumerate() {
            assert_eq!(a.count_ones(), 1);
            for b in &all[i + 1..] {
                assert_eq!(a & b, 0);
            }
        }
    }

    #[test]
    fn is_held_checks_only_that_button() {
        let input = LEFT | LIGHT;
        assert!(is_held(input, LEFT));
        assert!(is_held(input, LIGHT));
        assert!(!is_held(input, RIGHT));
        assert!(!is_held(input, HEAVY));
    }

    #[test]
    fn advance_counts_frames_and_records_inputs() {
        let mut state = GameState::default();
        state.advance([LEFT, HEAVY]);
        state.advance([RIGHT | UP, 0]);
        assert_eq!(state.frame, 2);
        assert_eq!(state.inputs, [RIGHT | UP, 0]);
    }

    #[test]
    fn each_player_moves_with_their_own_input() {
        let mut state = GameState::default();
        let start = state.clone();
        state.advance([RIGHT, 0]);
        assert!(state.fighters[0].x > start.fighters[0].x);
        assert_eq!(state.fighters[1], start.fighters[1]);
    }

    #[test]
    fn fighters_turn_around_after_crossing() {
        let mut state = GameState::default();
        // Put P1 to the right of P2 on the ground, as if they'd crossed.
        state.fighters[0].x = 1000 * SUBPIXELS;
        state.advance([0, 0]);
        assert_eq!(state.fighters[0].facing, -1);
        assert_eq!(state.fighters[1].facing, 1);
    }

    #[test]
    fn same_inputs_give_same_state() {
        let inputs = [
            [LEFT, 0],
            [LEFT | UP, DOWN],
            [0, HEAVY],
            [RIGHT | LIGHT, UP | LEFT],
            [HEAVY, 0],
        ];
        let mut a = GameState::default();
        let mut b = GameState::default();
        for i in inputs {
            a.advance(i);
            b.advance(i);
        }
        assert_eq!(a, b);
    }

    use attack::{HEAVY_ATTACK, LIGHT_ATTACK};
    use fighter::{Action, MAX_HEALTH};

    /// P1 at 400 px and P2 at `400 + gap` px, facing each other.
    fn fighters_apart(gap_px: i32) -> GameState {
        let mut state = GameState::default();
        state.fighters[0].x = 400 * SUBPIXELS;
        state.fighters[1].x = (400 + gap_px) * SUBPIXELS;
        state
    }

    /// Run `ticks` ticks with the same inputs held throughout.
    fn hold(state: &mut GameState, inputs: [Input; 2], ticks: u16) {
        for _ in 0..ticks {
            state.advance(inputs);
        }
    }

    #[test]
    fn light_hits_on_its_first_active_frame() {
        let mut state = fighters_apart(100);
        // Frames 0..3 are startup: 4 ticks, no hit yet.
        hold(&mut state, [LIGHT, 0], LIGHT_ATTACK.startup);
        assert_eq!(state.fighters[1].health, MAX_HEALTH);
        // Frame 4 is the first active frame.
        state.advance([LIGHT, 0]);
        let p2 = &state.fighters[1];
        assert_eq!(p2.health, MAX_HEALTH - LIGHT_ATTACK.damage);
        assert_eq!(p2.action, Action::Hitstun);
    }

    #[test]
    fn one_swing_hits_only_once() {
        let mut state = fighters_apart(100);
        hold(&mut state, [LIGHT, 0], LIGHT_ATTACK.total());
        assert_eq!(state.fighters[1].health, MAX_HEALTH - LIGHT_ATTACK.damage);
    }

    #[test]
    fn out_of_range_attack_whiffs() {
        let mut state = fighters_apart(300);
        hold(&mut state, [HEAVY, 0], HEAVY_ATTACK.total());
        assert_eq!(state.fighters[1].health, MAX_HEALTH);
    }

    #[test]
    fn light_whiffs_over_crouch_but_heavy_hits_it() {
        let mut state = fighters_apart(100);
        hold(&mut state, [LIGHT, DOWN], LIGHT_ATTACK.total());
        assert_eq!(state.fighters[1].health, MAX_HEALTH);

        let mut state = fighters_apart(100);
        hold(&mut state, [HEAVY, DOWN], HEAVY_ATTACK.total());
        assert_eq!(state.fighters[1].health, MAX_HEALTH - HEAVY_ATTACK.damage);
    }

    #[test]
    fn knockback_pushes_the_defender_away() {
        let mut state = fighters_apart(100);
        let start_x = state.fighters[1].x;
        hold(&mut state, [HEAVY, 0], HEAVY_ATTACK.total());
        assert!(state.fighters[1].x > start_x);

        // Mirrored: P2 hits P1, who is pushed left.
        let mut state = fighters_apart(100);
        let start_x = state.fighters[0].x;
        hold(&mut state, [0, HEAVY], HEAVY_ATTACK.total());
        assert!(state.fighters[0].x < start_x);
    }

    #[test]
    fn simultaneous_hits_trade() {
        let mut state = fighters_apart(100);
        hold(&mut state, [HEAVY, HEAVY], HEAVY_ATTACK.total());
        let expected = MAX_HEALTH - HEAVY_ATTACK.damage;
        assert_eq!(state.fighters[0].health, expected);
        assert_eq!(state.fighters[1].health, expected);
    }

    #[test]
    fn faster_attack_interrupts_slower_one() {
        let mut state = fighters_apart(100);
        hold(&mut state, [LIGHT, HEAVY], HEAVY_ATTACK.total());
        // Light (4 startup) lands before Heavy (10 startup) and cancels it.
        assert_eq!(state.fighters[1].health, MAX_HEALTH - LIGHT_ATTACK.damage);
        assert_eq!(state.fighters[0].health, MAX_HEALTH);
    }

    /// Ticks between the defender and the attacker becoming actionable
    /// after `button`'s attack hits. Positive means the attacker acts first.
    fn advantage_on_hit(button: Input) -> i32 {
        let mut state = fighters_apart(100);
        state.advance([button, 0]);
        let mut p1_free_at = None;
        let mut p2_free_at = None;
        for tick in 1..100 {
            state.advance([0, 0]);
            let p1 = &state.fighters[0];
            let p2 = &state.fighters[1];
            if p1_free_at.is_none() && p1.is_actionable() {
                p1_free_at = Some(tick);
            }
            if p2_free_at.is_none() && p2.health < MAX_HEALTH && p2.is_actionable() {
                p2_free_at = Some(tick);
            }
        }
        p2_free_at.unwrap() - p1_free_at.unwrap()
    }

    #[test]
    fn frame_advantage_on_hit() {
        // Light: the attacker acts first. Heavy: the defender does.
        assert_eq!(advantage_on_hit(LIGHT), 4);
        assert_eq!(advantage_on_hit(HEAVY), -4);
    }
}
