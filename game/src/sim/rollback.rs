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
    use crate::bot::Bot;
    use crate::sim::fighter::{Action, MAX_HEALTH};
    use crate::sim::round::Phase;
    use crate::sim::{DOWN, HEAVY, LEFT, LIGHT, RIGHT, UP};
    use rewind::SyncTestSession;

    /// How far back every tick rolls. At least the online session's
    /// max_prediction (8), the furthest a real rollback can go.
    const CHECK_DISTANCE: usize = 8;

    /// What a run went through, to prove the sync test reached the
    /// interesting code and didn't just watch two idle fighters.
    #[derive(Default, Debug)]
    struct Coverage {
        hits: u32,
        knockdowns: u32,
        round_overs: u32,
        match_overs: u32,
        jumps: u32,
        /// Special moves performed (combo.rs).
        dynamites: u32,
        super_kicks: u32,
        nados: u32,
    }

    impl Coverage {
        fn add(&mut self, other: &Coverage) {
            self.hits += other.hits;
            self.dynamites += other.dynamites;
            self.super_kicks += other.super_kicks;
            self.nados += other.nados;
        }
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
                let started = |action| now.action == action && was.action != action;
                if started(Action::Dynamite) {
                    cov.dynamites += 1;
                }
                if started(Action::SuperKick) {
                    cov.super_kicks += 1;
                }
                if started(Action::Nado) {
                    cov.nados += 1;
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
        let mut total = Coverage::default();
        for seed in [1, 2, 3, 42] {
            let cov = sync_test(seed, 6000);
            total.add(&cov);
            // Visible with `cargo test -p riposte-game sync -- --nocapture`.
            println!("seed {seed}: {cov:?}");
            assert!(cov.hits > 10, "seed {seed}: too few hits: {cov:?}");
            assert!(cov.jumps > 5, "seed {seed}: too few jumps: {cov:?}");
            assert!(cov.knockdowns >= 2, "seed {seed}: no KOs: {cov:?}");
            assert!(cov.round_overs >= 2, "seed {seed}: {cov:?}");
            assert!(cov.match_overs >= 1, "seed {seed}: no full match: {cov:?}");
        }
        // Random bots only occasionally earn a special; the test below plays
        // them on purpose.
        println!("specials over all seeds: {total:?}");
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

    /// Riposte itself over a bad fake network (R4): latency, jitter,
    /// reordering and lost messages, with each peer's bot playing from its
    /// own (sometimes guessed) copy of the game. Once every input is in,
    /// both copies must be identical.
    #[test]
    fn riposte_converges_over_a_bad_network() {
        use rewind::{Config, FakeConfig, FakeNetwork, FakeTransport, P2PSession};

        for seed in [1u64, 2, 3] {
            let net_config = FakeConfig {
                latency: 2,
                jitter: 2,
                loss_percent: 20,
                max_loss_streak: 2,
                in_order: seed % 2 == 0,
                seed,
            };
            let (net, ta, tb) = FakeNetwork::new(net_config);
            let mut peers: [P2PSession<GameState, FakeTransport>; 2] = [
                P2PSession::new(0, ta, Config::default()),
                P2PSession::new(1, tb, Config::default()),
            ];
            let mut games = [GameState::new(seed), GameState::new(seed)];
            let mut bots = [Bot::new(seed ^ 0x1111), Bot::new(seed ^ 0x2222)];
            let mut hits_seen = 0;

            // 40 s of play, then 1 s of no input so every input arrives.
            for t in 0..2400 + 60 {
                for p in 0..2 {
                    let input = if t < 2400 {
                        bots[p].input(p, &games[p])
                    } else {
                        0
                    };
                    peers[p].add_local_input(input);
                    peers[p].tick(&mut games[p]);
                }
                if games[0].fighters.iter().any(|f| f.health < MAX_HEALTH) {
                    hits_seen += 1;
                }
                net.advance();
            }

            assert_eq!(games[0], games[1], "seed {seed}: peers ended up different");
            assert!(
                peers[0].rollback_frames() > 0,
                "seed {seed}: never rolled back"
            );
            // Checked during the run, not at the end: a match can finish
            // and reset to full health just before the run stops.
            assert!(hits_seen > 0, "seed {seed}: nobody was ever hit");
        }
    }

    /// A scripted attacker that keeps performing the three special moves in
    /// turn against a defender who just stands there: three punches then a
    /// dynamite punch, four kicks then a super kick, two punches and a kick
    /// then a nado. Random bots rarely get that far, so this is what makes
    /// sure the streak counters, the launch and the pull are replayed from
    /// snapshots by the sync test.
    struct ComboPlayer {
        phase: usize,
        was_special: bool,
    }

    impl ComboPlayer {
        fn input(&mut self, game: &GameState) -> Input {
            let me = &game.fighters[0];
            let is_special = matches!(
                me.action,
                Action::Dynamite | Action::SuperKick | Action::Nado
            );
            if is_special && !self.was_special {
                self.phase += 1; // on to the next special
            }
            self.was_special = is_special;

            let gap = (game.fighters[1].x - me.x) / crate::sim::fighter::SUBPIXELS;
            if !me.is_actionable() {
                return 0;
            }
            // Release the buttons first: attacks start on a fresh press.
            if game.inputs[0] & (LIGHT | HEAVY) != 0 {
                return 0;
            }
            if gap > 100 {
                return RIGHT;
            }
            match me.ready_special() {
                Some(crate::sim::combo::Special::SuperKick) => HEAVY,
                Some(_) => LIGHT,
                None => match self.phase % 3 {
                    0 => LIGHT,
                    1 => HEAVY,
                    _ if me.punches < 2 => LIGHT,
                    _ => HEAVY,
                },
            }
        }
    }

    #[test]
    fn the_special_moves_pass_the_sync_test() {
        let mut game = GameState::new(5);
        let mut session: SyncTestSession<GameState> = SyncTestSession::new(CHECK_DISTANCE);
        let mut player = ComboPlayer {
            phase: 0,
            was_special: false,
        };
        let mut cov = Coverage::default();
        let mut last = game.clone();

        for _ in 0..4000 {
            session.add_local_input(0, player.input(&game));
            session.add_local_input(1, 0);
            if let Err(e) = session.tick(&mut game) {
                panic!("{e}");
            }
            for p in 0..2 {
                let (was, now) = (&last.fighters[p], &game.fighters[p]);
                let started = |action| now.action == action && was.action != action;
                cov.dynamites += started(Action::Dynamite) as u32;
                cov.super_kicks += started(Action::SuperKick) as u32;
                cov.nados += started(Action::Nado) as u32;
            }
            last = game.clone();
        }
        println!("scripted specials: {cov:?}");
        assert!(cov.dynamites >= 2, "{cov:?}");
        assert!(cov.super_kicks >= 2, "{cov:?}");
        assert!(cov.nados >= 2, "{cov:?}");
    }
}
