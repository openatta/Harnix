//! When a schedule is due: its timing read in its zone, the next time after an instant, and the
//! latest time up to one.
//!
//! Wall-clock rules (daily, weekly, cron) are matched minute by minute in local time: a local
//! time a daylight-saving change skips is not due that day, one it repeats is due the first time
//! only (as DSH's scheduler and Vixie cron do). Elapsed rules (after, every) count real time. A
//! one-off wall-clock time that falls in a gap is moved past it.

use jiff::civil::{Date, DateTime};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::Timestamp;

use crate::wire::Timing;

/// How far ahead a wall-clock rule is searched: long enough for 29 February, twice.
const SEARCH_DAYS: u32 = 366 * 9;

/// Look up an IANA zone.
pub fn zone(name: &str) -> Result<TimeZone, String> {
    if name.trim().is_empty() {
        return Err("time_zone is empty".into());
    }
    TimeZone::get(name).map_err(|e| format!("time_zone {name:?}: {e}"))
}

/// This machine's zone by its IANA name, or `UTC` when the system does not say one.
pub fn local_zone_name() -> String {
    TimeZone::try_system()
        .ok()
        .and_then(|tz| tz.iana_name().map(str::to_string))
        .unwrap_or_else(|| "UTC".to_string())
}

/// `ms` as `YYYY-MM-DD HH:MM` in `tz`: what a new session's title says.
pub fn local_minute(ms: u64, tz: &TimeZone) -> String {
    match Timestamp::from_millisecond(ms as i64) {
        Ok(ts) => tz.to_datetime(ts).strftime("%Y-%m-%d %H:%M").to_string(),
        Err(_) => ms.to_string(),
    }
}

/// A timing, read: what the next and latest times are computed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Once, at this instant.
    At(u64),
    /// Every `period` ms of elapsed time after `from`.
    Every { from: u64, period: u64 },
    /// Wall-clock minutes in a zone.
    Rule(Rule),
}

impl Plan {
    /// Read `timing` in `tz`. `from` is what an elapsed rule counts from when it carries none.
    pub fn of(timing: &Timing, tz: &TimeZone, from: u64) -> Result<Plan, String> {
        Ok(match timing {
            Timing::Once { at } => Plan::At(instant(at, tz)?),
            Timing::After { seconds, from: anchor } => {
                if *seconds == 0 {
                    return Err("after: seconds must be at least 1".into());
                }
                Plan::At(anchor.unwrap_or(from).saturating_add(seconds.saturating_mul(1000)))
            }
            Timing::Every { minutes, from: anchor } => {
                if *minutes == 0 {
                    return Err("every: minutes must be at least 1".into());
                }
                Plan::Every { from: anchor.unwrap_or(from), period: u64::from(*minutes) * 60_000 }
            }
            Timing::Daily { time } => {
                let (h, m) = clock(time)?;
                Plan::Rule(Rule::at(h, m, ALL_WEEKDAYS))
            }
            Timing::Weekly { weekdays, time } => {
                let (h, m) = clock(time)?;
                if weekdays.is_empty() {
                    return Err("weekly: at least one weekday".into());
                }
                let mut mask = 0u8;
                for d in weekdays {
                    if !(1..=7).contains(d) {
                        return Err(format!("weekly: {d} is not a weekday (Monday 1 … Sunday 7)"));
                    }
                    mask |= 1 << (d % 7);
                }
                Plan::Rule(Rule::at(h, m, mask))
            }
            Timing::Cron { expr } => Plan::Rule(Rule::cron(expr)?),
        })
    }

    /// The first time due strictly after `after`; `None`: never again.
    pub fn next_after(&self, tz: &TimeZone, after: u64) -> Option<u64> {
        match self {
            Plan::At(at) => (*at > after).then_some(*at),
            Plan::Every { from, period } => {
                let k = if after < *from { 0 } else { (after - from) / period };
                Some(from + period * (k + 1))
            }
            Plan::Rule(rule) => rule.next_after(tz, after),
        }
    }

    /// The latest time due in `[first, until]`, `first` being one that is due. Elapsed and
    /// one-off times are worked out; a wall-clock rule steps forward from `first`.
    pub fn latest_until(&self, tz: &TimeZone, first: u64, until: u64) -> u64 {
        match self {
            Plan::At(_) => first,
            Plan::Every { from, period } if until >= *from + period => {
                let k = (until - from) / period;
                (from + period * k).max(first)
            }
            Plan::Every { .. } => first,
            Plan::Rule(rule) => {
                let mut latest = first;
                // A minute rule missed for a year is half a million steps: past that, start
                // from a day before `until` instead.
                if until.saturating_sub(first) > 40 * 86_400_000 {
                    if let Some(t) = rule.next_after(tz, until - 86_400_000).filter(|t| *t <= until) {
                        latest = t;
                    }
                }
                while let Some(next) = rule.next_after(tz, latest).filter(|t| *t <= until) {
                    latest = next;
                }
                latest
            }
        }
    }
}

/// `HH:MM` → (hour, minute).
fn clock(time: &str) -> Result<(u8, u8), String> {
    let bad = || format!("time {time:?} is not HH:MM");
    let (h, m) = time.trim().split_once(':').ok_or_else(bad)?;
    let h: u8 = h.parse().map_err(|_| bad())?;
    let m: u8 = m.parse().map_err(|_| bad())?;
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok((h, m))
}

/// A one-off time: an instant with its offset, or a wall-clock time in `tz` (a gap moves it on).
fn instant(at: &str, tz: &TimeZone) -> Result<u64, String> {
    let at = at.trim();
    if let Ok(ts) = at.parse::<Timestamp>() {
        return millis(ts);
    }
    let dt: DateTime = at
        .parse()
        .map_err(|_| format!("at {at:?} is neither RFC 3339 nor YYYY-MM-DDTHH:MM[:SS]"))?;
    let zoned = tz.to_ambiguous_zoned(dt).compatible().map_err(|e| format!("at {at:?}: {e}"))?;
    millis(zoned.timestamp())
}

fn millis(ts: Timestamp) -> Result<u64, String> {
    u64::try_from(ts.as_millisecond()).map_err(|_| format!("{ts} is before 1970"))
}

const ALL_WEEKDAYS: u8 = 0b111_1111;

/// Wall-clock minutes, as cron's five fields say them. Bit `n` of each mask is the value `n`;
/// weekdays count from Sunday 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    /// Vixie's rule: when either day field is `*` (or starts with it) a day must match both, when
    /// neither is it must match one.
    any_day: bool,
}

impl Rule {
    /// `hour:minute` on these weekdays, every month: cron's `m h * * <weekdays>`.
    fn at(hour: u8, minute: u8, weekdays: u8) -> Rule {
        Rule {
            minutes: 1 << minute,
            hours: 1 << hour,
            days: field_mask(1, 31) as u32,
            months: field_mask(1, 12) as u16,
            weekdays,
            any_day: true,
        }
    }

    /// Five fields, Vixie's way. Refused when it is never due.
    pub fn cron(expr: &str) -> Result<Rule, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!("cron {expr:?}: five fields (minute hour day month weekday), not {}", fields.len()));
        }
        let minutes = parse_field(fields[0], 0, 59).map_err(|e| format!("cron minute: {e}"))?;
        let hours = parse_field(fields[1], 0, 23).map_err(|e| format!("cron hour: {e}"))?;
        let days = parse_field(fields[2], 1, 31).map_err(|e| format!("cron day of month: {e}"))?;
        let months = parse_field(fields[3], 1, 12).map_err(|e| format!("cron month: {e}"))?;
        let weekdays = parse_field(fields[4], 0, 7).map_err(|e| format!("cron day of week: {e}"))?;
        // 7 is Sunday too.
        let weekdays = ((weekdays & 0x7f) | (weekdays >> 7)) as u8;
        let any_day = fields[2].starts_with('*') || fields[4].starts_with('*');
        let rule = Rule { minutes, hours: hours as u32, days: days as u32, months: months as u16, weekdays, any_day };
        if !rule.can_fall() {
            return Err(format!("cron {expr:?} is never due"));
        }
        Ok(rule)
    }

    /// Whether some day of some month matches, leap years counted.
    fn can_fall(&self) -> bool {
        if !self.any_day && self.weekdays != 0 {
            return true;
        }
        if self.any_day && self.days == field_mask(1, 31) as u32 {
            return true;
        }
        const LONGEST: [u8; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        (1..=12).any(|m| self.months & (1 << m) != 0 && (1..=LONGEST[m - 1]).any(|d| self.days & (1 << d) != 0))
    }

    fn day_matches(&self, date: Date) -> bool {
        if self.months & (1 << date.month()) == 0 {
            return false;
        }
        let dom = self.days & (1 << date.day()) != 0;
        let dow = self.weekdays & (1 << (date.weekday().to_monday_one_offset() % 7)) != 0;
        if self.any_day {
            dom && dow
        } else {
            dom || dow
        }
    }

    /// The first matching minute strictly after `after`, in `tz`.
    pub fn next_after(&self, tz: &TimeZone, after: u64) -> Option<u64> {
        let start = tz.to_datetime(Timestamp::from_millisecond(after as i64).ok()?);
        // The first minute that can come after `after`: the one after its own.
        let (mut date, mut first_hour, mut first_minute) = (start.date(), start.hour(), start.minute() + 1);
        if first_minute == 60 {
            first_minute = 0;
            first_hour += 1;
        }
        if first_hour == 24 {
            date = date.tomorrow().ok()?;
            first_hour = 0;
        }
        for day in 0..SEARCH_DAYS {
            if self.day_matches(date) {
                for h in 0..24i8 {
                    if self.hours & (1 << h) == 0 || (day == 0 && h < first_hour) {
                        continue;
                    }
                    for m in 0..60i8 {
                        if self.minutes & (1 << m) == 0 || (day == 0 && h == first_hour && m < first_minute) {
                            continue;
                        }
                        let ambiguous = tz.to_ambiguous_zoned(date.at(h, m, 0, 0));
                        if matches!(ambiguous.offset(), AmbiguousOffset::Gap { .. }) {
                            continue;
                        }
                        let Ok(zoned) = ambiguous.earlier() else { continue };
                        let at = zoned.timestamp().as_millisecond();
                        if at > after as i64 {
                            return u64::try_from(at).ok();
                        }
                    }
                }
            }
            date = date.tomorrow().ok()?;
        }
        None
    }
}

/// Bits `lo..=hi` set.
fn field_mask(lo: u32, hi: u32) -> u64 {
    (lo..=hi).fold(0u64, |m, b| m | (1 << b))
}

/// One cron field: `*`, `n`, `a-b`, each with an optional `/step`, in a comma list.
fn parse_field(text: &str, lo: u32, hi: u32) -> Result<u64, String> {
    let mut mask = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s.parse().map_err(|_| format!("{part:?}: the step is not a number"))?;
                if step == 0 {
                    return Err(format!("{part:?}: a step of 0"));
                }
                (r, step)
            }
            None => (part, 1),
        };
        let number = |s: &str| -> Result<u32, String> {
            let n: u32 = s.parse().map_err(|_| format!("{s:?} is not a number (names are not read)"))?;
            if n < lo || n > hi {
                return Err(format!("{n} is outside {lo}-{hi}"));
            }
            Ok(n)
        };
        let (from, to) = if range == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            let (a, b) = (number(a)?, number(b)?);
            if a > b {
                return Err(format!("{part:?}: the range runs backwards"));
            }
            (a, b)
        } else {
            let a = number(range)?;
            // `a/n` runs from a to the end, as Vixie cron reads it.
            (a, if part.contains('/') { hi } else { a })
        };
        let mut v = from;
        while v <= to {
            mask |= 1 << v;
            v += step;
        }
    }
    Ok(mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ny() -> TimeZone {
        zone("America/New_York").unwrap()
    }

    /// Epoch ms of a wall-clock time in New York (unambiguous ones only).
    fn at(s: &str) -> u64 {
        let dt: DateTime = s.parse().unwrap();
        ny().to_ambiguous_zoned(dt).unambiguous().unwrap().timestamp().as_millisecond() as u64
    }

    fn utc(s: &str) -> u64 {
        s.parse::<Timestamp>().unwrap().as_millisecond() as u64
    }

    fn plan(t: Timing) -> Plan {
        Plan::of(&t, &ny(), 0).unwrap()
    }

    fn local(ms: u64) -> String {
        Timestamp::from_millisecond(ms as i64).unwrap().to_zoned(ny()).strftime("%Y-%m-%dT%H:%M %Z").to_string()
    }

    #[test]
    fn daily_is_the_next_such_minute_strictly_after() {
        let p = plan(Timing::Daily { time: "09:00".into() });
        assert_eq!(p.next_after(&ny(), at("2026-06-01T08:59:59")), Some(at("2026-06-01T09:00")));
        assert_eq!(p.next_after(&ny(), at("2026-06-01T09:00")), Some(at("2026-06-02T09:00")));
        assert_eq!(p.next_after(&ny(), at("2026-06-01T23:59:30")), Some(at("2026-06-02T09:00")));
    }

    #[test]
    fn a_time_the_spring_change_skips_is_not_due_that_day() {
        // 2026-03-08: New York goes from 02:00 EST to 03:00 EDT.
        let p = plan(Timing::Daily { time: "02:30".into() });
        let next = p.next_after(&ny(), at("2026-03-07T03:00")).unwrap();
        assert_eq!(local(next), "2026-03-09T02:30 EDT");
        // The hours either side are where they were.
        let p = plan(Timing::Daily { time: "03:30".into() });
        assert_eq!(local(p.next_after(&ny(), at("2026-03-07T04:00")).unwrap()), "2026-03-08T03:30 EDT");
    }

    #[test]
    fn a_time_the_autumn_change_repeats_is_due_the_first_time_only() {
        // 2026-11-01: 01:00–01:59 comes twice, EDT then EST.
        let p = plan(Timing::Daily { time: "01:30".into() });
        let first = p.next_after(&ny(), at("2026-10-31T12:00")).unwrap();
        assert_eq!(local(first), "2026-11-01T01:30 EDT");
        assert_eq!(first, utc("2026-11-01T05:30:00Z"));
        // An hour later the same wall-clock time comes again: not due.
        assert_eq!(local(p.next_after(&ny(), first).unwrap()), "2026-11-02T01:30 EST");
        // Every quarter of an hour: through the repeated hour once, on through the second one.
        let quarter = plan(Timing::Cron { expr: "*/15 * * * *".into() });
        let after_first = quarter.next_after(&ny(), utc("2026-11-01T05:50:00Z")).unwrap();
        assert_eq!(after_first, utc("2026-11-01T07:00:00Z"), "{}", local(after_first)); // 02:00 EST
    }

    #[test]
    fn elapsed_time_does_not_care_about_the_clock() {
        let p = plan(Timing::Every { minutes: 90, from: Some(at("2026-03-08T00:00")) });
        let next = p.next_after(&ny(), at("2026-03-08T01:00")).unwrap();
        assert_eq!(next - at("2026-03-08T00:00"), 90 * 60_000);
        assert_eq!(local(next), "2026-03-08T01:30 EST");
        let after = p.next_after(&ny(), next).unwrap();
        assert_eq!(local(after), "2026-03-08T04:00 EDT");
        // Before its start, the first one.
        assert_eq!(p.next_after(&ny(), 0), Some(at("2026-03-08T01:30")));
    }

    #[test]
    fn once_is_once() {
        let p = plan(Timing::Once { at: "2026-06-01T09:00".into() });
        assert_eq!(p, Plan::At(at("2026-06-01T09:00")));
        assert_eq!(p.next_after(&ny(), at("2026-06-01T08:00")), Some(at("2026-06-01T09:00")));
        assert_eq!(p.next_after(&ny(), at("2026-06-01T09:00")), None);
        // With an offset it is that instant, whatever the zone.
        assert_eq!(plan(Timing::Once { at: "2026-06-01T09:00:00Z".into() }), Plan::At(utc("2026-06-01T09:00:00Z")));
        // In the spring gap: moved past it.
        let gap = plan(Timing::Once { at: "2026-03-08T02:30".into() });
        assert_eq!(gap, Plan::At(at("2026-03-08T03:30")));
        // After: from when it was made.
        let after = Plan::of(&Timing::After { seconds: 90, from: None }, &ny(), 1_000).unwrap();
        assert_eq!(after, Plan::At(91_000));
    }

    #[test]
    fn weekly_counts_from_monday() {
        // 2026-06-01 is a Monday.
        let p = plan(Timing::Weekly { weekdays: vec![6, 7], time: "10:00".into() });
        assert_eq!(p.next_after(&ny(), at("2026-06-01T00:00")), Some(at("2026-06-06T10:00")));
        assert_eq!(p.next_after(&ny(), at("2026-06-06T10:00")), Some(at("2026-06-07T10:00")));
        assert_eq!(p.next_after(&ny(), at("2026-06-07T10:00")), Some(at("2026-06-13T10:00")));
        let workdays = plan(Timing::Weekly { weekdays: vec![1, 2, 3, 4, 5], time: "09:00".into() });
        assert_eq!(workdays.next_after(&ny(), at("2026-06-05T09:30")), Some(at("2026-06-08T09:00")));
        assert!(Plan::of(&Timing::Weekly { weekdays: vec![8], time: "10:00".into() }, &ny(), 0).is_err());
        assert!(Plan::of(&Timing::Weekly { weekdays: vec![], time: "10:00".into() }, &ny(), 0).is_err());
    }

    #[test]
    fn cron_reads_vixie_fields() {
        // The first of the month at 09:00.
        let monthly = plan(Timing::Cron { expr: "0 9 1 * *".into() });
        assert_eq!(monthly.next_after(&ny(), at("2026-06-02T00:00")), Some(at("2026-07-01T09:00")));
        // Both day fields restricted: either matches (the 13th, or a Friday).
        let either = plan(Timing::Cron { expr: "0 0 13 * 5".into() });
        assert_eq!(either.next_after(&ny(), at("2026-06-01T00:00")), Some(at("2026-06-05T00:00")));
        assert_eq!(either.next_after(&ny(), at("2026-06-12T00:00")), Some(at("2026-06-13T00:00")));
        // One of them `*`: both must.
        let fridays = plan(Timing::Cron { expr: "0 0 * * 5".into() });
        assert_eq!(fridays.next_after(&ny(), at("2026-06-06T00:00")), Some(at("2026-06-12T00:00")));
        // 7 is Sunday; ranges, lists and steps.
        let sundays = plan(Timing::Cron { expr: "30 8-10/2,23 * * 7".into() });
        assert_eq!(sundays.next_after(&ny(), at("2026-06-06T00:00")), Some(at("2026-06-07T08:30")));
        assert_eq!(sundays.next_after(&ny(), at("2026-06-07T08:30")), Some(at("2026-06-07T10:30")));
        assert_eq!(sundays.next_after(&ny(), at("2026-06-07T10:30")), Some(at("2026-06-07T23:30")));
        // 29 February: years ahead, still found.
        let leap = plan(Timing::Cron { expr: "0 12 29 2 *".into() });
        assert_eq!(leap.next_after(&ny(), at("2026-06-01T00:00")), Some(at("2028-02-29T12:00")));

        for bad in ["* * * *", "60 * * * *", "* * * * 8", "MON * * * *", "*/0 * * * *", "5-1 * * * *", "0 0 31 2 *", "0 0 30 2 *"] {
            assert!(Rule::cron(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_latest_due_up_to_now() {
        let p = plan(Timing::Daily { time: "09:00".into() });
        let first = at("2026-06-01T09:00");
        assert_eq!(p.latest_until(&ny(), first, at("2026-06-04T08:00")), at("2026-06-03T09:00"));
        assert_eq!(p.latest_until(&ny(), first, first), first);
        let every = plan(Timing::Every { minutes: 10, from: Some(0) });
        assert_eq!(every.latest_until(&ny(), 600_000, 3_599_999), 3_000_000);
        // A minute rule missed for a year comes out the same, without stepping through the year.
        let minutely = plan(Timing::Cron { expr: "* * * * *".into() });
        let now = at("2027-06-01T12:00") + 30_000;
        assert_eq!(minutely.latest_until(&ny(), at("2026-06-01T09:00"), now), at("2027-06-01T12:00"));
    }

    #[test]
    fn zones_are_iana_names() {
        assert!(zone("Asia/Shanghai").is_ok());
        assert!(zone("Mars/Olympus").is_err());
        assert!(zone("").is_err());
        assert!(zone(&local_zone_name()).is_ok());
        assert_eq!(local_minute(utc("2026-11-16T01:05:00Z"), &zone("Asia/Shanghai").unwrap()), "2026-11-16 09:05");
    }
}
