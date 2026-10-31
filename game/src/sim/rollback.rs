//! Riposte's side of the rewind interface (DESIGN.md section 4): how a
//! session saves, restores, steps and fingerprints the game.
//!
//! Everything here is thin on purpose. The hard part is making
//! `GameState::advance` deterministic and making `GameState` hold *all* the
//! state; this file just exposes that to rewind. The sync test below is the
//! proof that both are true.

use rewind::RollbackGame;

use super::{GameState, Input};

impl RollbackGame for GameState {
    type Input = Input;
    /// The whole `GameState` is the snapshot: it's a few hundred bytes, so
    /// cloning it every frame is cheap, and a full copy can't forget a field.
    type State = GameState;

    fn save(&self) -> GameState {
        self.clone()
    }

    fn load(&mut self, state: &GameState) {
        // `clone_from` reuses this value's memory where it can instead of
        // building a new one; the result is the same as `*self = state.clone()`.
        self.clone_from(state);
    }

    fn advance(&mut self, inputs: [Input; 2]) {
        // Calls the inherent `GameState::advance` (inherent methods win over
        // trait methods with the same name).
        GameState::advance(self, inputs);
    }

    fn checksum(&self) -> u64 {
        GameState::checksum(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::fighter::{Action, MAX_HEALTH};
    use crate::sim::rng::Rng;
    use crate::sim::round::Phase;
    use crate::sim::{DOWN, HEAVY, LEFT, LIGHT, RIGHT, UP};
    use rewind::SyncTestSession;

    /// How far back every tick rolls. At least the online session's
    /// max_prediction (8), the furthest a real rollback can go.
    const CHECK_DISTANCE: usize = 8;

    /// A crude bot: every few frames it picks something to do, biased
    /// toward walking in and attacking so that hits, KOs and whole matches
    /// happen within a few thousand frames.
    struct Bot {
        rng: Rng,
        held: Input,
        frames_left: u32,
    }

    impl Bot {
        fn new(seed: u64) -> Bot {
            Bot {
                rng: Rng::new(seed),
                held: 0,
                frames_left: 0,
            }
        }

        fn input(&mut self, me: usize, game: &GameState) -> Input {
            if self.frames_left == 0 {
                let other = 1 - me;
                let forward = if game.fighters[other].x > game.fighters[me].x {
                    RIGHT
                } else {
                    LEFT
                };
                let back = if forward == RIGHT { LEFT } else { RIGHT };
                // A fresh choice each segment, so attack buttons are released
                // and pressed again (attacks start on a press, not a hold).
                self.held = match self.rng.next_u64() % 20 {
                    0..=6 => forward,
                    7..=10 => LIGHT,
                    11..=13 => HEAVY,
                    14 => forward | LIGHT,
                    15 => UP | forward,
                    16 => DOWN,
                    17 => DOWN | HEAVY,
                    18 => back,
                    _ => 0,
                };
                self.frames_left = 3 + (self.rng.next_u64() % 10) as u32;
            }
            self.frames_left -= 1;
            self.held
        }
    }

    /// What a run went through, to prove the sync test reached the
    /// interesting code and didn't just watch two idle fighters.
    #[derive(Default, Debug)]
    struct Coverage {
        hits: u32,
        knockdowns: u32,
        round_overs: u32,
        match_overs: u32,
        jumps: u32,
    }

    /// Play `frames` frames of bot-vs-bot under the sync test. Panics with
    /// the frame and both checksums on the first desync.
    fn sync_test(seed: u64, frames: u32) -> Coverage {
        let mut game = GameState::new(seed);
        let mut session: SyncTestSession<GameState> = SyncTestSession::new(CHECK_DISTANCE);
        let mut bots = [Bot::new(seed ^ 0x1111), Bot::new(seed ^ 0x2222)];
        let mut cov = Coverage::default();
        let mut last = game.clone();

        for _ in 0..frames {
            for (player, bot) in bots.iter_mut().enumerate() {
                session.add_local_input(player, bot.input(player, &game));
            }
            if let Err(e) = session.tick(&mut game) {
                panic!("seed {seed}: {e}");
            }

            for p in 0..2 {
                let (was, now) = (&last.fighters[p], &game.fighters[p]);
                if now.health < was.health {
                    cov.hits += 1;
                }
                if now.action == Action::Knockdown && was.action != Action::Knockdown {
                    cov.knockdowns += 1;
                }
                if now.action == Action::Jump && was.action != Action::Jump {
                    cov.jumps += 1;
                }
            }
            let entered = |phase_now: bool, phase_was: bool| phase_now && !phase_was;
            let is_round_over = |s: &GameState| matches!(s.round.phase, Phase::RoundOver { .. });
            let is_match_over = |s: &GameState| matches!(s.round.phase, Phase::MatchOver { .. });
            if entered(is_round_over(&game), is_round_over(&last)) {
                cov.round_overs += 1;
            }
            if entered(is_match_over(&game), is_match_over(&last)) {
                cov.match_overs += 1;
            }
            last = game.clone();
        }
        cov
    }

    #[test]
    fn riposte_passes_the_sync_test() {
        // Several seeds: different bot behaviour and a different PRNG seed in
        // the state. 6000 frames is 100 s of play per seed.
        for seed in [1, 2, 3, 42] {
            let cov = sync_test(seed, 6000);
            // Visible with `cargo test -p riposte-game sync -- --nocapture`.
            println!("seed {seed}: {cov:?}");
            assert!(cov.hits > 10, "seed {seed}: too few hits: {cov:?}");
            assert!(cov.jumps > 5, "seed {seed}: too few jumps: {cov:?}");
            assert!(cov.knockdowns >= 2, "seed {seed}: no KOs: {cov:?}");
            assert!(cov.round_overs >= 2, "seed {seed}: {cov:?}");
            assert!(cov.match_overs >= 1, "seed {seed}: no full match: {cov:?}");
        }
    }

    #[test]
    fn save_then_load_restores_everything() {
        let mut game = GameState::new(9);
        let mut bot = [Bot::new(1), Bot::new(2)];
        for _ in 0..400 {
            let inputs = [bot[0].input(0, &game), bot[1].input(1, &game)];
            RollbackGame::advance(&mut game, inputs);
        }
        let saved = game.save();
        let checksum = RollbackGame::checksum(&game);
        for _ in 0..50 {
            RollbackGame::advance(&mut game, [RIGHT | LIGHT, LEFT]);
        }
        game.load(&saved);
        assert_eq!(game, saved);
        assert_eq!(RollbackGame::checksum(&game), checksum);
    }

    #[test]
    fn trait_methods_match_the_game_methods() {
        let mut a = GameState::new(5);
        let mut b = GameState::new(5);
        for inputs in [[RIGHT, LEFT], [LIGHT, 0], [0, HEAVY], [UP, DOWN]] {
            RollbackGame::advance(&mut a, inputs);
            b.advance(inputs);
        }
        assert_eq!(a, b);
        assert_eq!(RollbackGame::checksum(&a), b.checksum());
        assert!(a.fighters.iter().all(|f| f.health <= MAX_HEALTH));
    }
}
