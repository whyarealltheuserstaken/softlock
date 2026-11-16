//! How long the spike may keep the keyboard and mouse blocked.

use std::time::Duration;

/// Credential passthrough never stays open longer than this.
pub const PASSTHROUGH_TIMEOUT: Duration = Duration::from_secs(60);

/// The keyboard and mouse block ends by itself after this long, even if
/// Windows Hello never returns.
pub const MAX_BLOCK: Duration = Duration::from_secs(3 * 60);

/// Caps a requested block at [`MAX_BLOCK`].
pub fn clamp_block_limit(requested: Duration) -> Duration {
    if requested > MAX_BLOCK {
        MAX_BLOCK
    } else {
        requested
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_limits_are_sixty_seconds_and_three_minutes() {
        assert_eq!(PASSTHROUGH_TIMEOUT, Duration::from_secs(60));
        assert_eq!(MAX_BLOCK, Duration::from_secs(180));
    }

    #[test]
    fn a_longer_request_is_cut_down_to_three_minutes() {
        assert_eq!(
            clamp_block_limit(Duration::from_secs(10)),
            Duration::from_secs(10)
        );
        assert_eq!(clamp_block_limit(MAX_BLOCK), MAX_BLOCK);
        assert_eq!(
            clamp_block_limit(Duration::from_secs(60 * 60)),
            Duration::from_secs(180)
        );
    }
}
