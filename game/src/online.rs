//! An online match: this machine's game plus rewind's `P2PSession`, over
//! any `Transport`. Practice mode runs it over the fake network today; I1
//! runs it over the WebSocket transport (R6) once that exists, with
//! `local_player` and `seed` from the server's `MatchStart`.

use rewind::{Config, Event, P2PSession, Transport};

use crate::render::effects::Effects;
use crate::sim::{GameState, Input};

pub struct OnlineMatch<T: Transport> {
    pub game: GameState,
    session: P2PSession<GameState, T>,
    /// Hit flash and screen shake for this machine's view of the game.
    pub effects: Effects,
}

impl<T: Transport> OnlineMatch<T> {
    /// `local_player` (0 or 1) and `seed` must come from the same
    /// `MatchStart` on both machines, so both games start identical.
    pub fn new(local_player: usize, seed: u64, transport: T) -> OnlineMatch<T> {
        OnlineMatch {
            game: GameState::new(seed),
            session: P2PSession::new(local_player, transport, Config::default()),
            effects: Effects::default(),
        }
    }

    /// One tick with this machine's input. The session may load and re-run
    /// earlier frames inside this call; afterwards `game` is the best view of
    /// the current frame, ready to draw.
    pub fn tick(&mut self, local_input: Input) -> Vec<Event> {
        self.session.add_local_input(local_input);
        self.session.tick(&mut self.game)
    }

    pub fn rollback_frames(&self) -> u64 {
        self.session.rollback_frames()
    }

    pub fn deepest_rollback(&self) -> u32 {
        self.session.deepest_rollback()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::Bot;
    use rewind::memory_pair;

    #[test]
    fn two_online_matches_stay_identical_without_lag() {
        // With no delay and 2 frames of input delay, nothing is ever
        // guessed, so the two machines agree on every single tick.
        let (ta, tb) = memory_pair();
        let mut a = OnlineMatch::new(0, 7, ta);
        let mut b = OnlineMatch::new(1, 7, tb);
        let mut bots = [Bot::new(1), Bot::new(2)];
        for _ in 0..1200 {
            let ia = bots[0].input(0, &a.game);
            let ib = bots[1].input(1, &b.game);
            a.tick(ia);
            b.tick(ib);
            assert_eq!(a.game.checksum(), b.game.checksum());
        }
        assert_eq!(a.rollback_frames(), 0);
    }
}
