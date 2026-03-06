//! Reusable random number generation utilities.
//!
//! This module provides a [`LevelGenerator`] trait for producing random levels
//! (used by probabilistic data structures like skiplists) along with a
//! built-in [`XorShift`] implementation that requires no external dependencies.
//!
//! # Using a Custom Generator
//!
//! Implement [`LevelGenerator`] to plug in any randomization strategy:
//!
//! ```
//! use common::rng::LevelGenerator;
//!
//! struct ConstantLevel(usize);
//!
//! impl LevelGenerator for ConstantLevel {
//!     fn random_level(&mut self, max_level: usize) -> usize {
//!         self.0.min(max_level - 1)
//!     }
//! }
//! ```

/// Default probability used by [`XorShift`] for promoting a node to the next
/// level.
///
/// A probability of 0.5 means each successive level contains roughly half
/// the nodes of the level below, yielding a balanced distribution similar to
/// a binary tree.
pub const DEFAULT_PROBABILITY: f64 = 0.5;

// ---------------------------------------------------------------------------
// LevelGenerator trait
// ---------------------------------------------------------------------------

/// Strategy trait for generating random levels for new nodes in a
/// probabilistic data structure.
///
/// Implementations control how nodes are distributed across levels, which
/// directly affects performance. A good generator produces geometrically
/// distributed levels: most nodes appear only at level 0, with exponentially
/// fewer at each higher level.
pub trait LevelGenerator {
    /// Returns a random level in the range `0..max_level` for a newly
    /// inserted node.
    ///
    /// - Level 0 is the bottom (densest) level where every node appears.
    /// - Higher levels are sparser and serve as express lanes.
    /// - The returned value **must** satisfy `result < max_level`.
    fn random_level(&mut self, max_level: usize) -> usize;
}

// ---------------------------------------------------------------------------
// XorShift PRNG
// ---------------------------------------------------------------------------

/// A minimal xorshift64-based level generator.
///
/// Uses the xorshift64 algorithm (Marsaglia, 2003) to produce pseudo-random
/// numbers with no external dependencies. Each call to
/// [`LevelGenerator::random_level`] performs repeated "coin flips" — comparing
/// a random value against a probability threshold — to decide how many levels
/// a node participates in.
///
/// # Defaults
///
/// - **probability:** 0.5 — 50% chance of promotion to the next level
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

impl LevelGenerator for XorShift {
    fn random_level(&mut self, max_level: usize) -> usize {
        let threshold = (self.probability * u64::MAX as f64) as u64;
        let mut level = 0;
        while level < max_level - 1 && self.next_u64() < threshold {
            level += 1;
        }
        level
    }
}
