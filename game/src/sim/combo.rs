//! Hit streaks and the special moves they unlock.
//!
//! Landing normal attacks in a row builds a streak: Light hits are "punches",
//! Heavy hits are "kicks". Enough of them in a row earns a special move,
//! which the player then performs with the next button press:
//!
//! | Streak | Special | Performed with |
//! |---|---|---|
//! | 3 or more punches, no kicks | Dynamite punch | Light |
//! | 4 or more kicks, no punches | Super kick | Heavy |
//! | 3 or more hits, at least one punch and one kick | Nado | either button |
//!
//! The streak ends when its timer runs out (`STREAK_WINDOW` ticks since the
//! last landed hit) or when the fighter is hit. Performing a special uses the
//! streak up.

/// Ticks a streak survives without a new hit: 1.5 s at 60 ticks per second.
/// Heavy attacks recover slowly, so one second would make four kicks in a
/// row nearly impossible.
pub const STREAK_WINDOW: u16 = 90;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    DynamitePunch,
    SuperKick,
    Nado,
}

/// The special that a streak of `punches` Light hits and `kicks` Heavy hits
/// has earned, if any. Mixed streaks are checked first, so a 3-punch streak
/// that picks up a kick becomes a nado.
pub fn earned(punches: u8, kicks: u8) -> Option<Special> {
    if punches >= 1 && kicks >= 1 && punches.saturating_add(kicks) >= 3 {
        Some(Special::Nado)
    } else if kicks == 0 && punches >= 3 {
        Some(Special::DynamitePunch)
    } else if punches == 0 && kicks >= 4 {
        Some(Special::SuperKick)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_punches_earn_dynamite_at_three() {
        assert_eq!(earned(2, 0), None);
        assert_eq!(earned(3, 0), Some(Special::DynamitePunch));
        assert_eq!(earned(5, 0), Some(Special::DynamitePunch));
    }

    #[test]
    fn pure_kicks_earn_super_kick_at_four() {
        assert_eq!(earned(0, 3), None);
        assert_eq!(earned(0, 4), Some(Special::SuperKick));
    }

    #[test]
    fn mixed_streaks_earn_nado_at_three_hits() {
        assert_eq!(earned(1, 1), None, "only two hits");
        assert_eq!(earned(2, 1), Some(Special::Nado));
        assert_eq!(earned(1, 2), Some(Special::Nado));
        assert_eq!(
            earned(3, 1),
            Some(Special::Nado),
            "dynamite upgrades to nado"
        );
        assert_eq!(earned(1, 4), Some(Special::Nado), "super kick upgrades too");
    }

    #[test]
    fn empty_streak_earns_nothing() {
        assert_eq!(earned(0, 0), None);
    }

    #[test]
    fn counts_do_not_overflow() {
        assert_eq!(earned(255, 255), Some(Special::Nado));
    }
}
