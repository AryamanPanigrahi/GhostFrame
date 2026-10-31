//! Game feel (G6): hit flash and screen shake, driven purely by watching the
//! sim's state. Nothing here is read by the sim, so it can never change a hit,
//! a position or a checksum. Floats are fine: this is render-only.
//!
//! How it works: each time we draw, we compare every fighter's health with what
//! we saw last time. A drop means a hit landed, so we start a flash on that
//! fighter and a shake whose size grows with the damage. Both then count down in
//! sim ticks (not wall-clock time), so they last the same however fast the
//! window redraws.

use crate::sim::GameState;

/// Ticks the defender stays tinted after being hit.
const FLASH_TICKS: u8 = 6;
/// Shake lasts damage / SHAKE_TICKS_DIVISOR ticks (light 50 -> 3, heavy 120 -> 8).
const SHAKE_TICKS_DIVISOR: i16 = 15;
/// Shake starts at damage / SHAKE_PIXELS_DIVISOR logical pixels (light 5, heavy 12).
const SHAKE_PIXELS_DIVISOR: i16 = 10;

#[derive(Default)]
pub struct Effects {
    /// Sim frame seen on the previous `observe`, or `None` before the first.
    last_frame: Option<u32>,
    last_health: [i16; 2],
    flash_left: [u8; 2],
    shake_left: u8,
    shake_total: u8,
    shake_pixels: f32,
}

impl Effects {
    /// Call once per draw with the current sim state.
    pub fn observe(&mut self, game: &GameState) {
        let health = [game.fighters[0].health, game.fighters[1].health];

        let ticks_passed = match self.last_frame {
            Some(last) if game.frame >= last => game.frame - last,
            // First call, or the frame went backwards (restart, or later a
            // rollback). Take the new health as the baseline without firing
            // effects, so a reset never looks like damage.
            _ => {
                self.last_frame = Some(game.frame);
                self.last_health = health;
                self.flash_left = [0, 0];
                self.shake_left = 0;
                return;
            }
        };

        let mut hit_this_call = false;
        for (player, &now) in health.iter().enumerate() {
            let damage = self.last_health[player] - now;
            if damage > 0 {
                self.flash_left[player] = FLASH_TICKS;
                self.start_shake(damage);
                hit_this_call = true;
            }
        }

        // A hit seen now gets its full duration; only later calls count down.
        // Counting down in the same call would shorten a hit that landed
        // during a frame that ran several ticks.
        if ticks_passed > 0 && !hit_this_call {
            let n = ticks_passed.min(u32::from(u8::MAX)) as u8;
            for left in &mut self.flash_left {
                *left = left.saturating_sub(n);
            }
            self.shake_left = self.shake_left.saturating_sub(n);
        }

        self.last_frame = Some(game.frame);
        self.last_health = health;
    }
    fn start_shake(&mut self, damage: i16) {
        // A bigger hit replaces a smaller shake, but a small hit never cuts a
        // big one short.
        let ticks = (damage / SHAKE_TICKS_DIVISOR).clamp(1, 20) as u8;
        let pixels = f32::from((damage / SHAKE_PIXELS_DIVISOR).max(1));
        if pixels >= self.current_shake_pixels() {
            self.shake_left = ticks;
            self.shake_total = ticks;
            self.shake_pixels = pixels;
        }
    }

    fn current_shake_pixels(&self) -> f32 {
        if self.shake_total == 0 {
            return 0.0;
        }
        self.shake_pixels * f32::from(self.shake_left) / f32::from(self.shake_total)
    }

    /// 0.0 = no flash, 1.0 = just hit. Fades linearly over `FLASH_TICKS`.
    pub fn flash(&self, player: usize) -> f32 {
        f32::from(self.flash_left[player]) / f32::from(FLASH_TICKS)
    }

    /// Camera offset in logical pixels. The sign flips every tick so the view
    /// jitters side to side, and the size fades to zero as the shake ends.
    pub fn shake_offset(&self) -> (f32, f32) {
        let size = self.current_shake_pixels();
        if size <= 0.0 {
            return (0.0, 0.0);
        }
        let sign = if self.shake_left.is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        (size * sign, size * 0.5 * -sign)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state at `frame` where P2 has taken `damage`.
    fn state(frame: u32, damage_to_p2: i16) -> GameState {
        let mut game = GameState::default();
        game.frame = frame;
        game.fighters[1].health -= damage_to_p2;
        game
    }

    #[test]
    fn no_effects_without_a_hit() {
        let mut fx = Effects::default();
        fx.observe(&state(0, 0));
        fx.observe(&state(1, 0));
        assert_eq!(fx.flash(0), 0.0);
        assert_eq!(fx.flash(1), 0.0);
        assert_eq!(fx.shake_offset(), (0.0, 0.0));
    }

    #[test]
    fn health_drop_flashes_only_the_defender() {
        let mut fx = Effects::default();
        fx.observe(&state(0, 0));
        fx.observe(&state(1, 50));
        assert_eq!(fx.flash(0), 0.0);
        assert_eq!(fx.flash(1), 1.0);
    }

    #[test]
    fn flash_fades_in_sim_ticks() {
        let mut fx = Effects::default();
        fx.observe(&state(0, 0));
        fx.observe(&state(1, 50));
        // Three ticks later (even if drawn as one frame): half faded.
        fx.observe(&state(4, 50));
        assert!((fx.flash(1) - 0.5).abs() < 1e-6);
        fx.observe(&state(10, 50));
        assert_eq!(fx.flash(1), 0.0);
    }

    #[test]
    fn redrawing_the_same_tick_does_not_fade_effects() {
        let mut fx = Effects::default();
        fx.observe(&state(0, 0));
        fx.observe(&state(1, 50));
        fx.observe(&state(1, 50));
        fx.observe(&state(1, 50));
        assert_eq!(fx.flash(1), 1.0);
    }

    #[test]
    fn heavy_hit_shakes_harder_and_longer_than_light() {
        let mut light = Effects::default();
        light.observe(&state(0, 0));
        light.observe(&state(1, 50));
        let mut heavy = Effects::default();
        heavy.observe(&state(0, 0));
        heavy.observe(&state(1, 120));
        assert!(heavy.shake_left > light.shake_left);
        assert!(heavy.shake_offset().0.abs() > light.shake_offset().0.abs());
    }

    #[test]
    fn shake_ends_and_jitters_while_active() {
        let mut fx = Effects::default();
        fx.observe(&state(0, 0));
        fx.observe(&state(1, 120));
        let first = fx.shake_offset();
        fx.observe(&state(2, 120));
        let second = fx.shake_offset();
        assert_ne!(first.0.signum(), second.0.signum());
        fx.observe(&state(60, 120));
        assert_eq!(fx.shake_offset(), (0.0, 0.0));
    }

    #[test]
    fn small_hit_does_not_cut_a_big_shake_short() {
        let mut fx = Effects::default();
        fx.observe(&state(0, 0));
        fx.observe(&state(1, 120));
        let big = fx.shake_left;
        fx.observe(&state(2, 170));
        assert!(fx.shake_left >= big - 1);
    }

    #[test]
    fn restart_is_not_mistaken_for_damage() {
        let mut fx = Effects::default();
        fx.observe(&state(500, 0));
        // Frame goes backwards with lower health, as if the match restarted
        // mid-fight: no effects.
        fx.observe(&state(0, 300));
        assert_eq!(fx.flash(1), 0.0);
        assert_eq!(fx.shake_offset(), (0.0, 0.0));
    }

    #[test]
    fn healing_back_to_full_is_not_damage() {
        let mut fx = Effects::default();
        fx.observe(&state(0, 0));
        fx.observe(&state(1, 100));
        fx.observe(&state(2, 0));
        // Health went up (round reset): still just the old flash fading.
        assert!(fx.flash(1) <= 1.0);
        assert_eq!(fx.flash(0), 0.0);
    }
}
