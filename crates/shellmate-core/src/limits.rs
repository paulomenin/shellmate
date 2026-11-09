//! Internal time limits on helper programs (git, the shell's syntax check, man).
//!
//! The defaults suit an idle machine. `SHELLMATE_TIME_SCALE` multiplies them (1 to 100) for a slow
//! disk, a huge repository, or a test run under instrumentation. It never shortens a limit.

use std::time::Duration;

/// `base` times `SHELLMATE_TIME_SCALE`, which is 1 when unset or not a number.
pub fn scaled(base: Duration) -> Duration {
    base.mul_f64(scale(std::env::var("SHELLMATE_TIME_SCALE").ok().as_deref()))
}

fn scale(value: Option<&str>) -> f64 {
    value
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .map_or(1.0, |v| v.clamp(1.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_only_lengthens_and_is_bounded() {
        assert_eq!(scale(None), 1.0);
        assert_eq!(scale(Some("5")), 5.0);
        assert_eq!(scale(Some(" 2.5 ")), 2.5);
        assert_eq!(scale(Some("0.1")), 1.0);
        assert_eq!(scale(Some("-3")), 1.0);
        assert_eq!(scale(Some("1000")), 100.0);
        assert_eq!(scale(Some("fast")), 1.0);
        assert_eq!(scale(Some("NaN")), 1.0);
        assert_eq!(scale(Some("inf")), 1.0);
    }
}
