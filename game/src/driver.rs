//! Render frames → simulation ticks, keeping input between ticks. Every kind
//! of match (local, practice, online) runs its ticks through one of these.

use crate::clock::FixedClock;
use crate::sim::Input;

#[derive(Default)]
pub struct TickDriver {
    clock: FixedClock,
    /// Buttons seen on any frame since the last tick, per player.
    buffered: [Input; 2],
}

impl TickDriver {
    /// Bank this frame's elapsed time and inputs. Returns how many ticks are
    /// due and the inputs to use for them: everything seen since the last
    /// tick. On a 144 Hz screen most frames have no tick due, and a tap seen
    /// on one of those frames would otherwise never reach the sim, so with 0
    /// ticks due the inputs stay buffered for the next tick.
    pub fn frame(&mut self, elapsed_micros: u64, inputs: [Input; 2]) -> (u32, [Input; 2]) {
        self.buffered[0] |= inputs[0];
        self.buffered[1] |= inputs[1];
        let ticks = self.clock.advance(elapsed_micros);
        if ticks == 0 {
            return (0, [0, 0]);
        }
        let used = self.buffered;
        self.buffered = [0, 0];
        (ticks, used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{LEFT, LIGHT};

    /// 1/240 s: a quarter of a tick, like a frame on a 240 Hz screen.
    const QUARTER_TICK: u64 = 4_167;

    #[test]
    fn tap_on_a_frame_without_a_tick_is_kept() {
        let mut driver = TickDriver::default();
        // Pressed and released within one 240 Hz frame that has no tick.
        assert_eq!(driver.frame(QUARTER_TICK, [LIGHT, 0]), (0, [0, 0]));
        assert_eq!(driver.frame(QUARTER_TICK, [0, 0]).0, 0);
        assert_eq!(driver.frame(QUARTER_TICK, [0, 0]).0, 0);
        // The tick comes due: the tap is still there.
        assert_eq!(driver.frame(QUARTER_TICK, [0, LEFT]), (1, [LIGHT, LEFT]));
    }

    #[test]
    fn buffer_is_cleared_after_a_tick() {
        let mut driver = TickDriver::default();
        driver.frame(20_000, [LIGHT, 0]);
        assert_eq!(driver.frame(20_000, [0, 0]), (1, [0, 0]));
    }
}
