//! Time formatting without a date crate.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch; times before it count as 0.
pub fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Formats a time as RFC 3339 in UTC with second precision, e.g.
/// `2026-10-05T12:00:00Z`. Times before 1970 print as the epoch.
pub fn rfc3339_utc(t: SystemTime) -> String {
    let secs = unix_secs(t);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian
/// (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn known_dates() {
        assert_eq!(rfc3339_utc(at(0)), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(at(951_782_400)), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(at(1_791_201_600)), "2026-10-05T12:00:00Z");
        assert_eq!(rfc3339_utc(at(4_102_444_799)), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn before_epoch_is_epoch() {
        assert_eq!(
            rfc3339_utc(UNIX_EPOCH - Duration::from_secs(5)),
            "1970-01-01T00:00:00Z"
        );
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use proptest::prelude::*;
    use std::time::Duration;

    proptest! {
        /// Never panics for any second up to year 9999; always the same
        /// 20-byte shape with fields in range.
        #[test]
        fn rfc3339_has_one_shape(secs in 0u64..253_402_300_799) {
            let s = rfc3339_utc(UNIX_EPOCH + Duration::from_secs(secs));
            prop_assert_eq!(s.len(), 20, "{}", s);
            let b = s.as_bytes();
            prop_assert!(b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':'
                && b[16] == b':' && b[19] == b'Z');
            let n = |r: std::ops::Range<usize>| s[r].parse::<u32>().unwrap();
            prop_assert!((1..=12).contains(&n(5..7)) && (1..=31).contains(&n(8..10)));
            prop_assert!(n(11..13) < 24 && n(14..16) < 60 && n(17..19) < 60);
        }
    }

    #[test]
    fn the_far_future_does_not_panic() {
        let _ = rfc3339_utc(UNIX_EPOCH + Duration::from_secs(u64::MAX / 2));
        let _ = rfc3339_utc(SystemTime::now());
    }
}
