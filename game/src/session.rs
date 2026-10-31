//! Local 2-player: both inputs come from one keyboard, nothing is predicted
//! or rolled back.
//!
//! It is shaped like rewind's P2PSession (add inputs, then tick) so that
//! switching to online play later changes the session, not the sim or the
//! main loop's structure.

use crate::sim::{GameState, Input};

#[derive(Default)]
pub struct LocalSession {
    pending: [Input; 2],
}

impl LocalSession {
    /// Set `player`'s (0 or 1) input for the next tick.
    pub fn add_local_input(&mut self, player: usize, input: Input) {
        self.pending[player] = input;
    }

    /// Run one tick with the inputs given since the last tick.
    pub fn tick(&mut self, game: &mut GameState) {
        game.advance(self.pending);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{fighting_state, HEAVY, LEFT};

    #[test]
    fn inputs_reach_the_right_player() {
        let mut session = LocalSession::default();
        let mut game = fighting_state();
        session.add_local_input(0, LEFT);
        session.add_local_input(1, HEAVY);
        session.tick(&mut game);
        assert_eq!(game.inputs, [LEFT, HEAVY]);
        assert_eq!(game.frame, 1);
    }

    #[test]
    fn input_persists_until_replaced() {
        let mut session = LocalSession::default();
        let mut game = fighting_state();
        session.add_local_input(0, LEFT);
        session.tick(&mut game);
        session.tick(&mut game);
        assert_eq!(game.inputs, [LEFT, 0]);
        assert_eq!(game.frame, 2);
    }
}
