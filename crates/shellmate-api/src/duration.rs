//! Durations as the config writes them: a number and a unit, `90s`, `30m`, `2h`, `7d`.

use crate::Error;
use std::time::Duration;

/// `90s`, `30m`, `2h`, `7d`. No bare numbers: the unit is always explicit.
pub fn parse_duration(s: &str) -> Result<Duration, Error> {
    let bad = || {
        Error::new("config.invalid", format!("invalid duration `{s}`"))
            .with_hint("use a number and a unit: 90s, 30m, 2h, 7d")
    };
    let s = s.trim();
    let (num, unit) = s.split_at(s.len().checked_sub(1).ok_or_else(bad)?);
    let n: u64 = num.parse().map_err(|_| bad())?;
    let secs = match unit {
        "s" => n,
        "m" => n.checked_mul(60).ok_or_else(bad)?,
        "h" => n.checked_mul(3600).ok_or_else(bad)?,
        "d" => n.checked_mul(86400).ok_or_else(bad)?,
        _ => return Err(bad()),
    };
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_and_a_unit() {
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration(" 30m ").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("7d").unwrap(), Duration::from_secs(604_800));
        assert_eq!(parse_duration("0s").unwrap(), Duration::ZERO);
    }

    #[test]
    fn anything_else_is_refused_with_a_hint() {
        for bad in [
            "",
            "s",
            "5",
            "5x",
            "-5s",
            "1.5s",
            "ten s",
            "99999999999999999999s",
            &format!("{}d", u64::MAX),
        ] {
            let e = parse_duration(bad).unwrap_err();
            assert_eq!(e.code, "config.invalid", "{bad:?}");
            assert!(e.hint.is_some());
        }
    }
}
