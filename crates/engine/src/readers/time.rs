//! RFC 3339 times as the engine writes them.

/// Milliseconds since the epoch of an RFC 3339 time as the engine writes them (`…Z`, seconds
/// optionally with a fraction; an offset is honoured). `None` for anything else.
pub(super) fn ms(text: &str) -> Option<u64> {
    let (date, rest) = text.split_once(['T', 't'])?;
    let mut d = date.split('-');
    let year = d.next()?.parse::<i64>().ok()?;
    let month = d.next()?.parse::<i64>().ok()?;
    let day = d.next()?.parse::<i64>().ok()?;
    if d.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let at = rest.find(['Z', 'z', '+', '-'])?;
    let (clock, zone) = rest.split_at(at);
    let mut c = clock.split(':');
    let hour = c.next()?.parse::<i64>().ok()?;
    let minute = c.next()?.parse::<i64>().ok()?;
    let second = c.next()?;
    if c.next().is_some() {
        return None;
    }
    let (whole, fraction) = second.split_once('.').unwrap_or((second, ""));
    let whole = whole.parse::<i64>().ok()?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..=60).contains(&whole) {
        return None;
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Milliseconds: the first three digits of the fraction, the rest dropped.
    let millis = format!("{:0<3}", &fraction[..fraction.len().min(3)]).parse::<i64>().ok()?;
    let offset_minutes = match zone {
        "Z" | "z" => 0,
        _ => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let (h, m) = zone[1..].split_once(':')?;
            sign * (h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?)
        }
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + whole - offset_minutes * 60;
    u64::try_from(seconds * 1000 + millis).ok()
}

/// Days from 1970-01-01 to the civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The day `ms` falls on at `offset_minutes` east of UTC, in days from 1970-01-01.
pub(super) fn day_of(ms: u64, offset_minutes: i32) -> i64 {
    (ms as i64 + i64::from(offset_minutes) * 60_000).div_euclid(86_400_000)
}

/// `YYYY-MM-DD` of a day counted from 1970-01-01 (proleptic Gregorian).
pub(super) fn date_of(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::{date_of, day_of, days_from_civil, ms};

    #[test]
    fn days_are_counted_in_the_offset_and_written_as_dates() {
        for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 11, 16), (2026, 12, 31), (1969, 12, 31)] {
            assert_eq!(date_of(days_from_civil(y, m, d)), format!("{y:04}-{m:02}-{d:02}"));
        }
        let late = ms("2026-11-15T23:30:00Z").unwrap();
        assert_eq!(date_of(day_of(late, 0)), "2026-11-15");
        assert_eq!(date_of(day_of(late, 60)), "2026-11-16", "an hour east it is the next day");
        assert_eq!(date_of(day_of(late, -8 * 60)), "2026-11-15");
        assert_eq!(date_of(day_of(ms("2026-11-16T03:00:00Z").unwrap(), -5 * 60)), "2026-11-15", "five hours west it is still yesterday");
    }

    #[test]
    fn times_the_engine_writes_are_read() {
        assert_eq!(ms("2025-11-15T09:46:40Z"), Some(1_763_200_000_000));
        assert_eq!(ms("2025-11-15T09:46:40.5Z"), Some(1_763_200_000_500));
        assert_eq!(ms("2025-11-15T09:46:40.123456Z"), Some(1_763_200_000_123));
        assert_eq!(ms("2025-11-15T17:46:40+08:00"), Some(1_763_200_000_000));
        assert_eq!(ms("2025-11-15T04:46:40-05:00"), Some(1_763_200_000_000));
        assert_eq!(ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(ms("1970-01-01T00:00:01.001Z"), Some(1001));
    }

    #[test]
    fn anything_else_is_none() {
        for bad in ["", "yesterday", "2026-11-15", "2026-13-15T09:46:40Z", "2026-11-15T09:46Z", "2025-11-15T09:46:40", "1969-12-31T23:59:59Z"] {
            assert_eq!(ms(bad), None, "{bad:?}");
        }
    }
}
