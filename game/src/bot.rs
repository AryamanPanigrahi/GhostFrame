//! A crude computer opponent: every few frames it picks something to do,
//! biased toward walking in and attacking. Used by the G5 sync test, the R4
//! network test and the "Practice vs lag" mode.
//!
//! The bot is a *player*, not part of the simulation: it only produces
//! inputs, like a keyboard would. It reads the game to decide which way is
//! forward, but nothing it does needs to be deterministic across machines,
//! because its inputs travel over the network like a human's.

use crate::sim::rng::Rng;
use crate::sim::{GameState, Input, DOWN, HEAVY, LEFT, LIGHT, RIGHT, UP};

pub struct Bot {
    rng: Rng,
    held: Input,
    frames_left: u32,
}

impl Bot {
    pub fn new(seed: u64) -> Bot {
        Bot {
            rng: Rng::new(seed),
            held: 0,
            frames_left: 0,
        }
    }

    /// This tick's input for player `me`, given the game as the bot sees it.
    pub fn input(&mut self, me: usize, game: &GameState) -> Input {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_each_choice_for_a_few_frames() {
        let game = GameState::default();
        let mut bot = Bot::new(3);
        let inputs: Vec<Input> = (0..200).map(|_| bot.input(0, &game)).collect();
        // Count how often the input changes: at least every 12 frames, at
        // most every 3 (a repeat choice can make a segment look longer).
        let changes = inputs.windows(2).filter(|w| w[0] != w[1]).count();
        assert!((10..=66).contains(&changes), "{changes} changes");
    }

    #[test]
    fn same_seed_same_inputs() {
        let game = GameState::default();
        let (mut a, mut b) = (Bot::new(8), Bot::new(8));
        for _ in 0..100 {
            assert_eq!(a.input(1, &game), b.input(1, &game));
        }
    }
}
