//! Quick add for events: what a line says as an event.
//!
//! What a capture or the calendar's own field says. architect's capture parser (`architect_capture::parse`)
//! reads the day (`tomorrow`, `fri`, `2026-11-20`), the links, tags and
//! a repeat (`every week`); this reads the times it leaves in the title:
//! `1pm`, `13:30`, `9:30am`, `noon`, a range (`2-3pm`, `14:00–15:30`),
//! and a length (`for 45m`, `for:1h30`, or the `duration` a capture's
//! `for:` token gives).

use chrono::{Duration, NaiveDate, NaiveTime};

/// What a line says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quick {
    pub title: String,
    /// When it starts; none: all day.
    pub start: Option<NaiveTime>,
    /// How long, when said (or by a range).
    pub length: Option<Duration>,
}

/// A time of day: `1pm`, `1:30pm`, `13:30`, `9am`, `noon`, `midnight`.
#[must_use]
pub fn time_of_day(word: &str) -> Option<NaiveTime> {
    let w = word
        .trim()
        .trim_end_matches([',', '.'])
        .to_ascii_lowercase();
    match w.as_str() {
        "noon" | "midday" => return NaiveTime::from_hms_opt(12, 0, 0),
        "midnight" => return NaiveTime::from_hms_opt(0, 0, 0),
        _ => {}
    }
    let am = w
        .strip_suffix("am")
        .or_else(|| w.strip_suffix('a'))
        .map(|d| (d, Some(false)));
    let pm = || {
        w.strip_suffix("pm")
            .or_else(|| w.strip_suffix('p'))
            .map(|d| (d, Some(true)))
    };
    let (digits, meridiem) = am.or_else(pm).unwrap_or((w.as_str(), None));
    let (h, m) = match digits.split_once([':', '.']) {
        Some((h, m)) if m.len() == 2 => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        // A bare number is a time only with am or pm ("3pm"): `2026` is a
        // year.
        None if meridiem.is_some() => (digits.parse::<u32>().ok()?, 0),
        _ => return None,
    };
    if m > 59 {
        return None;
    }
    let h = match meridiem {
        Some(pm) if (1..=12).contains(&h) => (h % 12) + if pm { 12 } else { 0 },
        None if h <= 23 => h,
        _ => return None,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// A length: `45m`, `1h`, `1h30m`, `1h30`, `1:30`, `90` (minutes),
/// `1.5h`.
#[must_use]
pub fn length(text: &str) -> Option<Duration> {
    let t = text.trim().to_ascii_lowercase();
    let t = t.trim_start_matches("for:").trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(m) = t.parse::<i64>() {
        return (m > 0).then(|| Duration::minutes(m));
    }
    if let Some((h, m)) = t.split_once(':') {
        let (h, m) = (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?);
        return (h * 60 + m > 0).then(|| Duration::minutes(h * 60 + m));
    }
    let mut minutes = 0.0_f64;
    let mut num = String::new();
    let mut any = false;
    for c in t.chars() {
        match c {
            '0'..='9' | '.' => num.push(c),
            'h' | 'm' => {
                let n: f64 = num.parse().ok()?;
                minutes += if c == 'h' { n * 60.0 } else { n };
                num.clear();
                any = true;
            }
            'i' | 'n' | 'r' | 's' => {} // `min`, `hrs`, `mins`
            _ => return None,
        }
    }
    if !num.is_empty() {
        // `1h30`: the rest is minutes.
        minutes += num.parse::<f64>().ok()?;
    }
    // A length a person types: far inside i64.
    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
    let m = minutes.round() as i64;
    (any && m > 0).then(|| Duration::minutes(m))
}

/// A range, `2-3pm`, `14:00-15:30`, `9am–10:30am`: its start and end (the
/// end's am or pm carried to the start when it has none).
fn range(word: &str) -> Option<(NaiveTime, NaiveTime)> {
    let (a, b) = word.split_once(['-', '–'])?;
    let end = time_of_day(b)?;
    let b_lower = b.to_ascii_lowercase();
    let suffix = if b_lower.ends_with("pm") {
        "pm"
    } else if b_lower.ends_with("am") {
        "am"
    } else {
        ""
    };
    let start = time_of_day(a).or_else(|| time_of_day(&format!("{a}{suffix}")))?;
    // `11-1pm`: 11 is the morning's.
    let start = if start > end && !suffix.is_empty() {
        time_of_day(&format!("{a}{}", if suffix == "pm" { "am" } else { "pm" })).unwrap_or(start)
    } else {
        start
    };
    (end > start).then_some((start, end))
}

/// Reads the times out of `title`; what is left is its title.
#[must_use]
pub fn parse(title: &str) -> Quick {
    let words: Vec<&str> = title.split_whitespace().collect();
    let mut keep = Vec::new();
    let mut start = None;
    let mut length_said = None;
    let mut i = 0;
    while let Some(w) = words.get(i).copied() {
        let lower = w.to_ascii_lowercase();
        let next = words.get(i + 1).copied();
        if start.is_none()
            && let Some((s, e)) = range(w)
        {
            start = Some(s);
            length_said = Some(e - s);
            i += 1;
            continue;
        }
        // `at 3pm`, `@ 3pm`.
        if start.is_none()
            && (lower == "at" || lower == "@")
            && let Some(t) = next.and_then(time_of_day)
        {
            start = Some(t);
            i += 2;
            continue;
        }
        if start.is_none()
            && let Some(t) = time_of_day(w)
        {
            // `3 pm`.
            start = Some(t);
            i += 1;
            continue;
        }
        if start.is_none()
            && let (Ok(_), Some(n)) = (w.parse::<u32>(), next)
            && matches!(n.to_ascii_lowercase().as_str(), "am" | "pm")
            && let Some(t) = time_of_day(&format!("{w}{n}"))
        {
            start = Some(t);
            i += 2;
            continue;
        }
        if length_said.is_none() {
            if let Some(rest) = lower.strip_prefix("for:")
                && let Some(l) = length(rest)
            {
                length_said = Some(l);
                i += 1;
                continue;
            }
            if lower == "for"
                && let Some(l) = next.and_then(length)
            {
                length_said = Some(l);
                i += 2;
                continue;
            }
        }
        keep.push(w);
        i += 1;
    }
    Quick {
        title: keep.join(" "),
        start,
        length: length_said,
    }
}

/// The start and end a line makes on `day`: at its time for its length
/// (`default` without one), or all `day` with no time.
#[must_use]
pub fn span(
    q: &Quick,
    day: NaiveDate,
    default: Duration,
) -> (chrono::NaiveDateTime, chrono::NaiveDateTime, bool) {
    q.start.map_or_else(
        || {
            let s = day.and_time(NaiveTime::MIN);
            let days = q.length.map_or(1, |l| (l.num_hours() / 24).max(1));
            (s, s + Duration::days(days), true)
        },
        |t| {
            let s = day.and_time(t);
            (s, s + q.length.unwrap_or(default), false)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> Option<NaiveTime> {
        NaiveTime::from_hms_opt(h, m, 0)
    }

    #[test]
    fn times_of_day() {
        assert_eq!(time_of_day("1pm"), t(13, 0));
        assert_eq!(time_of_day("12am"), t(0, 0));
        assert_eq!(time_of_day("12pm"), t(12, 0));
        assert_eq!(time_of_day("9:30am"), t(9, 30));
        assert_eq!(time_of_day("13:30"), t(13, 30));
        assert_eq!(time_of_day("noon"), t(12, 0));
        for no in ["13pm", "2026", "3", "9:5", "25:00", "Sam"] {
            assert_eq!(time_of_day(no), None, "{no}");
        }
    }

    #[test]
    fn lengths() {
        assert_eq!(length("45m"), Some(Duration::minutes(45)));
        assert_eq!(length("1h30"), Some(Duration::minutes(90)));
        assert_eq!(length("1h30m"), Some(Duration::minutes(90)));
        assert_eq!(length("1:15"), Some(Duration::minutes(75)));
        assert_eq!(length("1.5h"), Some(Duration::minutes(90)));
        assert_eq!(length("2hrs"), Some(Duration::minutes(120)));
        assert_eq!(length("for:20"), Some(Duration::minutes(20)));
        assert_eq!(length("Sam"), None);
    }

    #[test]
    fn lines() {
        let q = parse("Lunch with Sam 1pm for 90m");
        assert_eq!(
            (q.title.as_str(), q.start, q.length),
            ("Lunch with Sam", t(13, 0), Some(Duration::minutes(90)))
        );
        let q = parse("Standup 9:30-9:45");
        assert_eq!((q.start, q.length), (t(9, 30), Some(Duration::minutes(15))));
        let q = parse("Mix review 2-3pm");
        assert_eq!((q.start, q.length), (t(14, 0), Some(Duration::hours(1))));
        let q = parse("Brunch 11-1pm");
        assert_eq!((q.start, q.length), (t(11, 0), Some(Duration::hours(2))));
        let q = parse("Call at 4 pm");
        assert_eq!((q.title.as_str(), q.start), ("Call at", t(16, 0)));
        let q = parse("Call at 4pm");
        assert_eq!((q.title.as_str(), q.start), ("Call", t(16, 0)));
        let q = parse("Dentist");
        assert_eq!(
            (q.title.as_str(), q.start, q.length),
            ("Dentist", None, None)
        );
        // Only the first time is the start.
        assert_eq!(parse("Train 9am, arrives 11am").title, "Train arrives 11am");
    }

    #[test]
    fn spans() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        let (s, e, all) = span(&parse("Gig 8pm"), day, Duration::hours(1));
        assert_eq!(
            (s.to_string(), e.to_string(), all),
            (
                "2026-11-06 20:00:00".into(),
                "2026-11-06 21:00:00".into(),
                false
            )
        );
        let (s, e, all) = span(&parse("Holiday"), day, Duration::hours(1));
        assert_eq!((s.date(), (e - s).num_days(), all), (day, 1, true));
    }
}
