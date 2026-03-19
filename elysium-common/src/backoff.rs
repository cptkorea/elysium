//! Simple exponential backoff for retry loops.

use std::time::Duration;

/// Computes an exponentially increasing delay for retry attempt `attempt`,
/// clamped to `max`.
///
/// The returned duration is `base * 2^attempt`, saturating at `max`.
/// `attempt` values above 30 are clamped to avoid overflow in the shift.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use elysium_common::backoff::exponential;
///
/// let base = Duration::from_millis(100);
/// let max  = Duration::from_secs(5);
///
/// assert_eq!(exponential(base, 0, max), Duration::from_millis(100));
/// assert_eq!(exponential(base, 1, max), Duration::from_millis(200));
/// assert_eq!(exponential(base, 2, max), Duration::from_millis(400));
/// assert_eq!(exponential(base, 3, max), Duration::from_millis(800));
/// assert_eq!(exponential(base, 20, max), max); // clamped
/// ```
pub fn exponential(base: Duration, attempt: u32, max: Duration) -> Duration {
    base.saturating_mul(1 << attempt.min(30)).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_each_attempt() {
        let base = Duration::from_millis(50);
        let max = Duration::from_secs(10);

        assert_eq!(exponential(base, 0, max), Duration::from_millis(50));
        assert_eq!(exponential(base, 1, max), Duration::from_millis(100));
        assert_eq!(exponential(base, 2, max), Duration::from_millis(200));
        assert_eq!(exponential(base, 3, max), Duration::from_millis(400));
    }

    #[test]
    fn clamps_to_max() {
        let base = Duration::from_millis(100);
        let max = Duration::from_millis(500);

        assert_eq!(exponential(base, 0, max), Duration::from_millis(100));
        assert_eq!(exponential(base, 3, max), max);
        assert_eq!(exponential(base, 10, max), max);
    }

    #[test]
    fn high_attempt_does_not_overflow() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(60);

        let result = exponential(base, u32::MAX, max);
        assert_eq!(result, max);
    }
}
