//! Turns real elapsed time into a whole number of 1/60 s simulation ticks.
//!
//! Monitors refresh at 30, 60, 144 Hz or anything else, and frames can hitch.
//! The sim must still advance exactly 60 times per real second, so we bank
//! elapsed time and spend it in fixed 1/60 s chunks.

pub const TICKS_PER_SECOND: u64 = 60;
const MICROS_PER_SECOND: u64 = 1_000_000;

/// After a long pause (window dragged, laptop asleep) we'd otherwise run
/// hundreds of ticks at once and the game would fast-forward. Past this cap
/// the extra time is dropped and the game simply resumes.
pub const MAX_TICKS_PER_FRAME: u32 = 4;

#[derive(Default)]
pub struct FixedClock {
    /// Banked time in units of 1/60,000,000 s (microseconds times 60).
    /// One tick is 16_666.67 microseconds, which isn't a whole number; scaled
    /// by 60, one tick is exactly 1_000_000 units, so no time is lost to
    /// rounding and no floats are needed.
    banked: u64,
}

impl FixedClock {
    /// Bank `elapsed_micros` of real time and return how many ticks to run now.
    pub fn advance(&mut self, elapsed_micros: u64) -> u32 {
        self.banked = self
            .banked
            .saturating_add(elapsed_micros.saturating_mul(TICKS_PER_SECOND));
        let ticks = self.banked / MICROS_PER_SECOND;
        self.banked %= MICROS_PER_SECOND;
        // Clamping drops the excess ticks; the sub-tick remainder is kept.
        ticks.min(MAX_TICKS_PER_FRAME as u64) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_real_second_is_sixty_ticks_at_any_frame_rate() {
        for fps in [30u64, 60, 75, 144, 240] {
            let mut clock = FixedClock::default();
            let mut ticks = 0;
            // Split one second into `fps` frames. 1_000_000 doesn't always
            // divide evenly, so each frame ends at the exact i/fps mark.
            let mut spent = 0;
            for i in 1..=fps {
                let until = MICROS_PER_SECOND * i / fps;
                ticks += clock.advance(until - spent);
                spent = until;
            }
            assert_eq!(ticks, 60, "at {fps} fps");
        }
    }

    #[test]
    fn short_frames_bank_time_until_a_tick_is_due() {
        let mut clock = FixedClock::default();
        assert_eq!(clock.advance(10_000), 0);
        assert_eq!(clock.advance(10_000), 1); // 20 ms banked ≥ 16.67 ms
    }

    #[test]
    fn long_pause_is_capped() {
        let mut clock = FixedClock::default();
        assert_eq!(clock.advance(5 * MICROS_PER_SECOND), MAX_TICKS_PER_FRAME);
        // The dropped time doesn't come back on the next frame.
        assert_eq!(clock.advance(0), 0);
    }
}
