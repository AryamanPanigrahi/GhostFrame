//! A tiny deterministic random number generator (xorshift64).
//!
//! `thread_rng` and friends draw from the OS, so two machines would get
//! different numbers. This one is just a `u64` in `GameState`: the same seed
//! gives the same sequence everywhere, and saving/loading the state for
//! rollback saves and restores the generator too.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng {
    pub state: u64,
}

/// xorshift maps 0 to 0 forever, so a zero seed is swapped for this
/// (arbitrary, non-zero) constant.
const ZERO_SEED_REPLACEMENT: u64 = 0x9E37_79B9_7F4A_7C15;

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let state = if seed == 0 {
            ZERO_SEED_REPLACEMENT
        } else {
            seed
        };
        Rng { state }
    }

    /// Next pseudo-random number. Three shift-and-xor steps scramble the bits;
    /// 13, 7, 17 are the standard xorshift64 constants (Marsaglia, 2003),
    /// chosen so the sequence visits every non-zero u64 before repeating.
    // Nothing in the sim needs randomness yet, but the generator has to be in
    // the state from the start so checksums and rollback already cover it.
    #[allow(dead_code)]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn zero_seed_does_not_get_stuck() {
        let mut r = Rng::new(0);
        assert_ne!(r.next_u64(), 0);
        assert_ne!(r.next_u64(), 0);
    }

    #[test]
    fn known_first_value() {
        // Computed independently (outside Rust); pins the algorithm so a
        // "harmless" edit can't change every future match.
        let mut r = Rng::new(1);
        assert_eq!(r.next_u64(), 0x4082_2041);
    }
}
