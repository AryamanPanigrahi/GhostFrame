//! A determinism test that runs offline: every tick, roll back and
//! resimulate the last few frames, and check the checksums match the first
//! run.
//!
//! Online, both peers run the same game independently and roll back
//! whenever a prediction was wrong. That only works if loading a saved state
//! and re-running the same inputs gives *exactly* the same result. A game that
//! keeps something outside its saved state, reads the clock, or uses floats
//! that round differently will drift, and the drift shows up as a desync
//! minutes into a match. `SyncTestSession` forces a rollback on every tick,
//! so the same bug shows up within a few frames, on one machine, with the
//! frame number where it happened.

use std::collections::VecDeque;
use std::fmt;

use crate::api::{Frame, RollbackGame};

/// The first run and the resimulation disagreed: the game is not
/// deterministic (or `save`/`load` miss part of the state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncTestError {
    /// The frame whose checksum differed.
    pub frame: Frame,
    /// Checksum after that frame on the first run.
    pub expected: u64,
    /// Checksum after that frame when resimulated.
    pub actual: u64,
}

impl fmt::Display for SyncTestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "desync at frame {}: first run checksum {:#018x}, resimulated {:#018x}",
            self.frame, self.expected, self.actual
        )
    }
}

impl std::error::Error for SyncTestError {}

/// Everything needed to replay one frame: the state before it, the inputs
/// used, and the checksum the first run produced after it.
struct Record<G: RollbackGame> {
    frame: Frame,
    state_before: G::State,
    inputs: [G::Input; 2],
    checksum_after: u64,
}

pub struct SyncTestSession<G: RollbackGame> {
    /// How many frames to roll back on every tick.
    check_distance: usize,
    /// Inputs given since the last tick, one per player.
    pending: [G::Input; 2],
    /// The last `check_distance` frames, oldest first.
    history: VecDeque<Record<G>>,
    /// The next frame to simulate.
    frame: Frame,
}

impl<G: RollbackGame> SyncTestSession<G> {
    /// `check_distance` is how far back to roll every tick. Use at least the
    /// online session's `max_prediction` (8), since that's the furthest a
    /// real rollback can go.
    pub fn new(check_distance: usize) -> SyncTestSession<G> {
        assert!(check_distance > 0, "check_distance must be at least 1");
        SyncTestSession {
            check_distance,
            pending: [G::Input::default(); 2],
            history: VecDeque::with_capacity(check_distance),
            frame: 0,
        }
    }

    /// Set `player`'s (0 or 1) input for the next tick. Both players are
    /// local here: there is no network.
    pub fn add_local_input(&mut self, player: usize, input: G::Input) {
        self.pending[player] = input;
    }

    /// The next frame `tick` will simulate.
    pub fn current_frame(&self) -> Frame {
        self.frame
    }

    /// Advance one frame, then roll back `check_distance` frames and
    /// resimulate them, comparing every checksum with the first run.
    ///
    /// On success the game ends up exactly where a plain `advance` would have
    /// left it. On error the game is left mid-resimulation and the test
    /// should stop.
    pub fn tick(&mut self, game: &mut G) -> Result<(), SyncTestError> {
        // First run of this frame, recording what's needed to replay it.
        let inputs = self.pending;
        let state_before = game.save();
        game.advance(inputs);
        self.history.push_back(Record {
            frame: self.frame,
            state_before,
            inputs,
            checksum_after: game.checksum(),
        });
        if self.history.len() > self.check_distance {
            self.history.pop_front();
        }
        self.frame += 1;

        // Roll back to the oldest kept frame and replay up to now. This is
        // exactly what an online session does after a misprediction, except
        // the inputs are the same, so every checksum must match.
        game.load(&self.history[0].state_before);
        for record in &self.history {
            game.advance(record.inputs);
            let actual = game.checksum();
            if actual != record.checksum_after {
                return Err(SyncTestError {
                    frame: record.frame,
                    expected: record.checksum_after,
                    actual,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A counter that adds both inputs each tick. Fully deterministic.
    #[derive(Default)]
    struct Counter {
        total: u64,
        ticks: u32,
    }

    impl RollbackGame for Counter {
        type Input = u8;
        type State = (u64, u32);

        fn save(&self) -> (u64, u32) {
            (self.total, self.ticks)
        }
        fn load(&mut self, state: &(u64, u32)) {
            (self.total, self.ticks) = *state;
        }
        fn advance(&mut self, inputs: [u8; 2]) {
            self.total += inputs[0] as u64 * 3 + inputs[1] as u64;
            self.ticks += 1;
        }
        fn checksum(&self) -> u64 {
            self.total.wrapping_mul(31) ^ self.ticks as u64
        }
    }

    /// Like `Counter`, but `save` forgets `hidden`, so a rollback can't
    /// restore it: the classic determinism bug.
    #[derive(Default)]
    struct Leaky {
        total: u64,
        hidden: u64,
    }

    impl RollbackGame for Leaky {
        type Input = u8;
        type State = u64;

        fn save(&self) -> u64 {
            self.total
        }
        fn load(&mut self, state: &u64) {
            self.total = *state;
        }
        fn advance(&mut self, inputs: [u8; 2]) {
            self.hidden += 1;
            self.total += inputs[0] as u64 + self.hidden;
        }
        fn checksum(&self) -> u64 {
            self.total
        }
    }

    fn run<G: RollbackGame<Input = u8> + Default>(
        frames: u32,
        distance: usize,
    ) -> (G, Result<(), SyncTestError>) {
        let mut game = G::default();
        let mut session = SyncTestSession::new(distance);
        for frame in 0..frames {
            session.add_local_input(0, (frame % 7) as u8);
            session.add_local_input(1, (frame % 3) as u8);
            if let Err(error) = session.tick(&mut game) {
                return (game, Err(error));
            }
        }
        (game, Ok(()))
    }

    #[test]
    fn deterministic_game_passes() {
        let (_, result) = run::<Counter>(200, 8);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn rollbacks_leave_the_same_state_as_a_plain_run() {
        let (rolled_back, _) = run::<Counter>(200, 8);
        let mut plain = Counter::default();
        for frame in 0..200u32 {
            plain.advance([(frame % 7) as u8, (frame % 3) as u8]);
        }
        assert_eq!(rolled_back.save(), plain.save());
    }

    #[test]
    fn passes_before_history_is_full() {
        // Fewer frames than the check distance: rolls back to frame 0.
        let (game, result) = run::<Counter>(3, 8);
        assert_eq!(result, Ok(()));
        assert_eq!(game.ticks, 3);
    }

    #[test]
    fn state_missing_from_save_is_caught() {
        let (_, result) = run::<Leaky>(200, 8);
        let error = result.unwrap_err();
        // The very first resimulation replays frame 0 with `hidden` already
        // at 1, so it's caught on frame 0.
        assert_eq!(error.frame, 0);
        assert_ne!(error.expected, error.actual);
    }

    #[test]
    fn counts_frames() {
        let mut game = Counter::default();
        let mut session = SyncTestSession::new(4);
        assert_eq!(session.current_frame(), 0);
        session.tick(&mut game).unwrap();
        session.tick(&mut game).unwrap();
        assert_eq!(session.current_frame(), 2);
    }

    #[test]
    fn inputs_persist_until_replaced() {
        let mut game = Counter::default();
        let mut session = SyncTestSession::new(2);
        session.add_local_input(0, 1);
        session.tick(&mut game).unwrap();
        session.tick(&mut game).unwrap();
        assert_eq!(game.total, 6);
    }

    #[test]
    #[should_panic(expected = "at least 1")]
    fn zero_distance_is_rejected() {
        SyncTestSession::<Counter>::new(0);
    }
}
