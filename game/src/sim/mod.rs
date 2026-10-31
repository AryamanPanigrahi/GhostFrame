//! Deterministic simulation. No floats, no time, no window: given the same
//! state and the same inputs, `advance` must produce the same result on every
//! machine, because rollback re-runs it and both peers run it independently.

pub mod attack;
pub mod checksum;
pub mod fighter;
pub mod rect;
pub mod rng;
pub mod round;

use attack::Attack;
use checksum::Fnv1a;
use fighter::{Fighter, SUBPIXELS};
use rng::Rng;
use round::{Phase, Round, MATCH_OVER_TICKS, READY_TICKS, ROUND_OVER_TICKS, WINS_TO_MATCH};

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
    pub round: Round,
    pub rng: Rng,
}

impl Default for GameState {
    fn default() -> GameState {
        GameState::new(0)
    }
}

/// Both fighters on the floor, 480 px apart, facing each other, full health.
fn starting_fighters() -> [Fighter; 2] {
    [
        Fighter::new(400 * SUBPIXELS, 1),
        Fighter::new(880 * SUBPIXELS, -1),
    ]
}

impl GameState {
    /// A fresh match. Online, both peers get the same `seed` from the
    /// server's `MatchStart` message, so their generators agree.
    pub fn new(seed: u64) -> GameState {
        GameState {
            frame: 0,
            inputs: [0, 0],
            fighters: starting_fighters(),
            round: Round::new_match(),
            rng: Rng::new(seed),
        }
    }

    /// Advance exactly one tick (1/60 s).
    pub fn advance(&mut self, inputs: [Input; 2]) {
        // Outside the fight (pauses, after a KO) players' buttons are
        // ignored, but physics keeps running so a KO'd fighter still lands.
        let inputs = if self.round.accepts_input() {
            inputs
        } else {
            [0, 0]
        };

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

        // Hits only count during the fight, so a swing still active after a
        // KO can't hit the downed fighter out of Knockdown.
        if self.round.phase == Phase::Fight {
            self.resolve_hits();
            self.check_ko();
        }
        self.tick_round();

        self.frame += 1;
        self.inputs = inputs;
    }

    fn resolve_hits(&mut self) {
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
    }

    /// End the round if anyone is at 0 health. Both at 0 (a trade that KOs
    /// both) is a draw: nobody scores and the round is replayed.
    fn check_ko(&mut self) {
        let down0 = self.fighters[0].health == 0;
        let down1 = self.fighters[1].health == 0;
        if !down0 && !down1 {
            return;
        }
        let winner = match (down0, down1) {
            (false, true) => Some(0),
            (true, false) => Some(1),
            _ => None,
        };
        if down0 {
            self.fighters[0].knock_down();
        }
        if down1 {
            self.fighters[1].knock_down();
        }
        if let Some(w) = winner {
            self.round.wins[w as usize] += 1;
        }
        self.round.phase = Phase::RoundOver { winner };
        // +1 because tick_round counts this same tick down straight away.
        self.round.timer = ROUND_OVER_TICKS + 1;
    }

    /// Count down the current pause and move to the next phase when it ends.
    fn tick_round(&mut self) {
        if self.round.phase == Phase::Fight {
            return;
        }
        self.round.timer -= 1;
        if self.round.timer > 0 {
            return;
        }
        match self.round.phase {
            Phase::Ready => self.round.phase = Phase::Fight,
            Phase::RoundOver { .. } => {
                if self.round.wins[0] >= WINS_TO_MATCH {
                    self.end_match(0);
                } else if self.round.wins[1] >= WINS_TO_MATCH {
                    self.end_match(1);
                } else {
                    self.round.number += 1;
                    self.start_round();
                }
            }
            Phase::MatchOver { .. } => {
                self.round = Round::new_match();
                self.fighters = starting_fighters();
            }
            Phase::Fight => {}
        }
    }

    /// Fighters back to their starting spots at full health. Wins, the
    /// frame counter and the random generator carry on.
    fn start_round(&mut self) {
        self.fighters = starting_fighters();
        self.round.phase = Phase::Ready;
        self.round.timer = READY_TICKS;
    }

    fn end_match(&mut self, winner: u8) {
        self.round.phase = Phase::MatchOver { winner };
        self.round.timer = MATCH_OVER_TICKS;
    }

    /// Fingerprint of the whole state, for desync detection (DESIGN.md
    /// section 5, step 7). Like `Fighter::hash_into`, the destructuring
    /// `let` makes forgetting a new field a compile error.
    pub fn checksum(&self) -> u64 {
        let GameState {
            frame,
            inputs,
            fighters,
            round,
            rng,
        } = self;
        let mut h = Fnv1a::new();
        h.write_u32(*frame);
        h.write_u16(inputs[0]);
        h.write_u16(inputs[1]);
        fighters[0].hash_into(&mut h);
        fighters[1].hash_into(&mut h);
        hash_round(round, &mut h);
        h.write_u64(rng.state);
        h.finish()
    }
}

fn hash_round(round: &Round, h: &mut Fnv1a) {
    let Round {
        number,
        wins,
        phase,
        timer,
    } = *round;
    h.write_u8(number);
    h.write_u8(wins[0]);
    h.write_u8(wins[1]);
    // Phase as (tag, winner): 255 stands for "no winner".
    let (tag, winner) = match phase {
        Phase::Ready => (0, 255),
        Phase::Fight => (1, 255),
        Phase::RoundOver { winner } => (2, winner.unwrap_or(255)),
        Phase::MatchOver { winner } => (3, winner),
    };
    h.write_u8(tag);
    h.write_u8(winner);
    h.write_u16(timer);
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

/// A state already past the Ready pause, for tests that want control at once.
#[cfg(test)]
pub fn fighting_state() -> GameState {
    let mut state = GameState::default();
    state.round.phase = Phase::Fight;
    state
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
        let mut state = fighting_state();
        state.advance([LEFT, HEAVY]);
        state.advance([RIGHT | UP, 0]);
        assert_eq!(state.frame, 2);
        assert_eq!(state.inputs, [RIGHT | UP, 0]);
    }

    #[test]
    fn each_player_moves_with_their_own_input() {
        let mut state = fighting_state();
        let start = state.clone();
        state.advance([RIGHT, 0]);
        assert!(state.fighters[0].x > start.fighters[0].x);
        assert_eq!(state.fighters[1], start.fighters[1]);
    }

    #[test]
    fn fighters_turn_around_after_crossing() {
        let mut state = fighting_state();
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
        let mut a = fighting_state();
        let mut b = fighting_state();
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
        let mut state = fighting_state();
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

    use fighter::STAGE_WIDTH;

    /// Fight state where P2 is one light attack from KO, in range of P1.
    fn p2_nearly_out() -> GameState {
        let mut state = fighters_apart(100);
        state.fighters[1].health = LIGHT_ATTACK.damage;
        state
    }

    /// Land P1's light on P2 (KO'ing them in `p2_nearly_out`).
    fn p1_lands_light(state: &mut GameState) {
        hold(state, [LIGHT, 0], LIGHT_ATTACK.startup + 1);
    }

    #[test]
    fn match_starts_in_ready_and_ignores_input() {
        let mut state = GameState::new(7);
        assert_eq!(state.round.phase, Phase::Ready);
        let start_x = state.fighters[0].x;
        hold(&mut state, [RIGHT, 0], READY_TICKS - 1);
        assert_eq!(state.fighters[0].x, start_x);
        assert_eq!(state.round.phase, Phase::Ready);
        state.advance([RIGHT, 0]);
        assert_eq!(state.round.phase, Phase::Fight);
        state.advance([RIGHT, 0]);
        assert!(state.fighters[0].x > start_x);
    }

    #[test]
    fn ko_ends_the_round_and_scores() {
        let mut state = p2_nearly_out();
        p1_lands_light(&mut state);
        assert_eq!(state.fighters[1].health, 0);
        assert_eq!(state.fighters[1].action, Action::Knockdown);
        assert_eq!(state.round.phase, Phase::RoundOver { winner: Some(0) });
        assert_eq!(state.round.wins, [1, 0]);
    }

    #[test]
    fn input_is_ignored_after_ko() {
        let mut state = p2_nearly_out();
        p1_lands_light(&mut state);
        hold(&mut state, [0, 0], LIGHT_ATTACK.total());
        let p1_x = state.fighters[0].x;
        hold(&mut state, [RIGHT, LEFT | UP], 30);
        assert_eq!(state.fighters[0].x, p1_x);
        assert_eq!(state.fighters[1].action, Action::Knockdown);
    }

    #[test]
    fn next_round_resets_fighters_but_keeps_score() {
        let mut state = p2_nearly_out();
        p1_lands_light(&mut state);
        hold(&mut state, [0, 0], ROUND_OVER_TICKS);
        assert_eq!(state.round.phase, Phase::Ready);
        assert_eq!(state.round.number, 2);
        assert_eq!(state.round.wins, [1, 0]);
        assert_eq!(state.fighters, starting_fighters());
    }

    #[test]
    fn round_over_lasts_exactly_its_pause() {
        let mut state = p2_nearly_out();
        p1_lands_light(&mut state);
        hold(&mut state, [0, 0], ROUND_OVER_TICKS - 1);
        assert!(matches!(state.round.phase, Phase::RoundOver { .. }));
        state.advance([0, 0]);
        assert_eq!(state.round.phase, Phase::Ready);
    }

    #[test]
    fn second_win_takes_the_match_then_a_new_match_starts() {
        let mut state = p2_nearly_out();
        state.round.wins = [1, 0];
        p1_lands_light(&mut state);
        hold(&mut state, [0, 0], ROUND_OVER_TICKS);
        assert_eq!(state.round.phase, Phase::MatchOver { winner: 0 });

        hold(&mut state, [0, 0], MATCH_OVER_TICKS);
        assert_eq!(state.round, Round::new_match());
        assert_eq!(state.fighters, starting_fighters());
    }

    #[test]
    fn double_ko_is_a_draw_and_replays_the_round() {
        let mut state = fighters_apart(100);
        state.fighters[0].health = HEAVY_ATTACK.damage;
        state.fighters[1].health = HEAVY_ATTACK.damage;
        hold(&mut state, [HEAVY, HEAVY], HEAVY_ATTACK.startup + 1);
        assert_eq!(state.round.phase, Phase::RoundOver { winner: None });
        assert_eq!(state.round.wins, [0, 0]);

        hold(&mut state, [0, 0], ROUND_OVER_TICKS);
        assert_eq!(state.round.number, 2);
        assert_eq!(state.round.wins, [0, 0]);
    }

    #[test]
    fn equal_states_have_equal_checksums() {
        let inputs = [[RIGHT, LEFT], [LIGHT, 0], [0, UP | RIGHT], [HEAVY, DOWN]];
        let mut a = GameState::new(99);
        let mut b = GameState::new(99);
        for _ in 0..80 {
            for i in inputs {
                a.advance(i);
                b.advance(i);
            }
        }
        assert_eq!(a.checksum(), b.checksum());
    }

    #[test]
    fn changing_any_part_of_the_state_changes_the_checksum() {
        let base = GameState::new(1);
        let changes: [fn(&mut GameState); 9] = [
            |s| s.frame += 1,
            |s| s.inputs[1] = LIGHT,
            |s| s.fighters[0].x += 1,
            |s| s.fighters[1].health -= 1,
            |s| s.fighters[0].action = Action::Walk,
            |s| s.fighters[1].attack_connected = true,
            |s| s.round.wins[0] = 1,
            |s| s.round.phase = Phase::Fight,
            |s| s.rng.state += 1,
        ];
        for change in changes {
            let mut changed = base.clone();
            change(&mut changed);
            assert_ne!(changed.checksum(), base.checksum());
        }
    }

    #[test]
    fn swapping_the_players_changes_the_checksum() {
        // Same values in a different order must not hash the same.
        let mut a = fighting_state();
        a.fighters[0].x = 300 * SUBPIXELS;
        a.fighters[1].x = STAGE_WIDTH - 300 * SUBPIXELS;
        let mut b = a.clone();
        b.fighters.swap(0, 1);
        assert_ne!(a.checksum(), b.checksum());
    }
}
