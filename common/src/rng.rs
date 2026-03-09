//! Reusable random number generation utilities.
//!
//! This module provides a [`RandomN`] trait for producing bounded random
//! values along with a built-in [`XorShift`] implementation that requires no
//! external dependencies.
//!
//! # Using a Custom Generator
//!
//! Implement [`RandomN`] to plug in any randomization strategy:
//!
//! ```
//! use common::rng::RandomN;
//!
//! struct Fixed(usize);
//!
//! impl RandomN for Fixed {
//!     fn random_n(&mut self, max: usize) -> usize {
//!         self.0.min(max - 1)
//!     }
//! }
//! ```

/// Default probability used by [`XorShift`] for promoting to the next level.
///
/// A probability of 0.5 means each successive level contains roughly half
/// the nodes of the level below, yielding a balanced distribution similar to
/// a binary tree.
pub const DEFAULT_PROBABILITY: f64 = 0.5;

// ---------------------------------------------------------------------------
// RandomN trait
// ---------------------------------------------------------------------------

/// Strategy trait for generating a bounded random number.
///
/// Implementations produce a value in `0..max`, controlling how values are
/// distributed. A good generator produces geometrically distributed results:
/// most values are 0, with exponentially fewer at each higher value.
pub trait RandomN {
    /// Returns a random value in the range `0..max`.
    ///
    /// The returned value **must** satisfy `result < max`.
    fn random_n(&mut self, max: usize) -> usize;
}

// ---------------------------------------------------------------------------
// XorShift PRNG
// ---------------------------------------------------------------------------

/// A minimal xorshift64-based random number generator.
///
/// Uses the xorshift64 algorithm (Marsaglia, 2003) to produce pseudo-random
/// numbers with no external dependencies. Each call to [`RandomN::random_n`]
/// performs repeated "coin flips" — comparing a random value against a
/// probability threshold — to produce a geometrically distributed result.
///
/// # Defaults
///
/// - **probability:** 0.5 — 50% chance of incrementing to the next value
/// - **state:** automatically seeded from the system clock
///
/// # Deterministic Seeding
///
/// For reproducible behavior (e.g. in tests), use [`XorShift::with_seed`]:
///
/// ```
/// use common::rng::XorShift;
///
/// let rng = XorShift::with_seed(0.5, 42);
/// ```
pub struct XorShift {
    /// Internal PRNG state. Must never be zero (xorshift invariant).
    state: u64,
    /// Promotion probability in the range (0.0, 1.0).
    probability: f64,
}

impl XorShift {
    /// Creates a new generator with the given promotion probability,
    /// automatically seeded from the system clock.
    ///
    /// # Panics
    ///
    /// Panics if `probability` is not in the open interval (0.0, 1.0).
    pub fn new(probability: f64) -> Self {
        assert!(
            probability > 0.0 && probability < 1.0,
            "probability must be in (0.0, 1.0), got {probability}"
        );
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let state = if seed == 0 { 0xDEAD_BEEF_CAFE_BABE } else { seed };
        Self { state, probability }
    }

    /// Creates a new generator with a fixed seed for deterministic behavior.
    ///
    /// Useful in tests where reproducibility matters.
    ///
    /// # Panics
    ///
    /// Panics if `probability` is not in the open interval (0.0, 1.0).
    pub fn with_seed(probability: f64, seed: u64) -> Self {
        assert!(
            probability > 0.0 && probability < 1.0,
            "probability must be in (0.0, 1.0), got {probability}"
        );
        let state = if seed == 0 { 0xDEAD_BEEF_CAFE_BABE } else { seed };
        Self { state, probability }
    }

    /// Advances the xorshift64 state and returns the next pseudo-random u64.
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

impl Default for XorShift {
    /// Creates a generator with the recommended probability of 0.5,
    /// automatically seeded from the system clock.
    fn default() -> Self {
        Self::new(DEFAULT_PROBABILITY)
    }
}

impl RandomN for XorShift {
    fn random_n(&mut self, max: usize) -> usize {
        let threshold = (self.probability * u64::MAX as f64) as u64;
        let mut n = 0;
        while n < max - 1 && self.next_u64() < threshold {
            n += 1;
        }
        n
    }
}
