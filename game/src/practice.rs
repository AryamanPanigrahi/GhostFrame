//! "Practice vs lag": play the bot through real rollback netcode over the
//! fake network (R4), with the lag switchable mid-match. Both sides run in
//! this one program: your `OnlineMatch` as player 0, and the bot's own
//! session and game copy as player 1, joined by a `FakeNetwork`.
//!
//! Nothing here exists for gameplay; it's for feeling and testing rollback
//! before real online play (I1) is possible.

use rewind::{FakeConfig, FakeNetwork, FakeTransport, P2PSession};

use crate::bot::Bot;
use crate::driver::TickDriver;
use crate::online::OnlineMatch;
use crate::sim::{GameState, Input};

/// Any fixed seed works: both sides are in this program.
const PRACTICE_SEED: u64 = 2;

/// Lag settings, picked with keys 1-4: (one-way latency, jitter, loss %).
/// Delays are in ticks (1 tick ≈ 17 ms). The worst case, 6 + 1 ticks plus
/// one lost message, stays inside what the session absorbs without a stall
/// (R4, "the prediction envelope"); R5's stall is needed for more.
pub const LAG_PRESETS: [(u32, u32, u32); 4] = [(0, 0, 0), (2, 1, 0), (4, 1, 5), (6, 1, 10)];

fn network_config(preset: usize) -> FakeConfig {
    let (latency, jitter, loss_percent) = LAG_PRESETS[preset];
    FakeConfig {
        latency,
        jitter,
        loss_percent,
        max_loss_streak: 1,
        // Like the WebSocket we'll use for real: delayed, never reordered.
        in_order: true,
        seed: PRACTICE_SEED,
    }
}

pub struct Practice {
    /// Your side: what's drawn on screen.
    pub you: OnlineMatch<FakeTransport>,
    bot_session: P2PSession<GameState, FakeTransport>,
    bot_game: GameState,
    bot: Bot,
    net: FakeNetwork,
    driver: TickDriver,
    preset: usize,
}

impl Practice {
    pub fn new() -> Practice {
        let preset = 2;
        let (net, yours, bots) = FakeNetwork::new(network_config(preset));
        Practice {
            you: OnlineMatch::new(0, PRACTICE_SEED, yours),
            bot_session: P2PSession::new(1, bots, rewind::Config::default()),
            bot_game: GameState::new(PRACTICE_SEED),
            bot: Bot::new(PRACTICE_SEED),
            net,
            driver: TickDriver::default(),
            preset,
        }
    }

    /// One render frame. Each tick: you play, the bot plays on its own copy
    /// of the game, then one tick of network time passes.
    pub fn update(&mut self, elapsed_micros: u64, your_input: Input) {
        let (ticks, inputs) = self.driver.frame(elapsed_micros, [your_input, 0]);
        for _ in 0..ticks {
            self.you.tick(inputs[0]);
            let bot_input = self.bot.input(1, &self.bot_game);
            self.bot_session.add_local_input(bot_input);
            self.bot_session.tick(&mut self.bot_game);
            self.net.advance();
        }
    }

    /// Switch to lag preset `preset` (0-3) mid-match.
    pub fn set_preset(&mut self, preset: usize) {
        if preset < LAG_PRESETS.len() {
            self.preset = preset;
            self.net.set_config(network_config(preset));
        }
    }

    /// Debug text for the screen.
    pub fn overlay(&self) -> Vec<String> {
        let (latency, jitter, loss) = LAG_PRESETS[self.preset];
        let ms = |ticks: u32| ticks * 1000 / 60;
        vec![
            format!(
                "PRACTICE VS LAG  preset {}: one-way {} ms (+{} ms jitter, {}% loss)",
                self.preset + 1,
                ms(latency),
                ms(jitter),
                loss
            ),
            format!(
                "rolled back {} frames so far, deepest {}   keys 1-4: lag   Esc: menu",
                self.you.rollback_frames(),
                self.you.deepest_rollback()
            ),
            "You: either key set (WASD+F/G or arrows+K/L)".to_owned(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{LIGHT, RIGHT};

    /// One tick of real time, so each `update` runs exactly one tick.
    const TICK: u64 = 16_667;

    #[test]
    fn worst_preset_rolls_back_without_crashing() {
        let mut practice = Practice::new();
        practice.set_preset(3);
        // A minute of you walking in and pressing light now and then.
        for t in 0..3600 {
            let input = if t % 20 < 3 { LIGHT } else { RIGHT };
            practice.update(TICK, input);
        }
        assert!(practice.you.rollback_frames() > 0);
        assert!(practice.you.deepest_rollback() <= 8);
        assert_eq!(practice.you.game.frame, practice.bot_game.frame);
    }

    #[test]
    fn without_lag_both_sides_always_agree() {
        let mut practice = Practice::new();
        practice.set_preset(0);
        for t in 0..1200 {
            practice.update(TICK, if t % 30 < 2 { LIGHT } else { RIGHT });
            assert_eq!(practice.you.game.checksum(), practice.bot_game.checksum());
        }
    }

    #[test]
    fn switching_lag_mid_match_is_safe() {
        let mut practice = Practice::new();
        for t in 0..2400 {
            if t % 300 == 0 {
                practice.set_preset((t / 300) % 4);
            }
            practice.update(TICK, if t % 25 < 2 { LIGHT } else { RIGHT });
        }
        assert_eq!(practice.you.game.frame, 2400);
    }
}
