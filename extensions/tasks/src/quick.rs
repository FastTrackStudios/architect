//! Quick add: a line typed into "New task" read into a task, ported from
//! Task's capture (`task:features/task/task-proto/src/capture.rs`
//! `capture`), with the same rules.
//!
//! `Call the venue tomorrow [[Album]] #gigs @phone !high every week`:
//!
//! - `#tag` → `tags`; `@context` → `contexts`; `[[Link]]` → `projects`;
//! - `!low|normal|high|critical` → `priority`;
//! - `today`, `tomorrow`, a weekday, `next <weekday>`, `YYYY-MM-DD` →
//!   `due` (a word that only introduced it, `for` / `by` / `on`, goes too);
//! - `daily`, `weekly`, `every week`, `every 3 days`, `every other friday`,
//!   `every weekday` → `recurrence` (an RRULE) and its anchor: a named day
//!   is `scheduled` (the calendar decides), a bare interval `completion`
//!   (counted from when it was done);
//! - the rest is the title.
//!
//! When architect's capture (#352) lands, these become the `task` capture
//! target's tokens; until then the Tasks list's "New task" field and
//! `TASKS_ADD` read them.

// Token indices within one line, and a day or a week on a date.
#![allow(clippy::arithmetic_side_effects)]

use chrono::{Datelike, Duration, NaiveDate, Weekday};

use crate::model::{Priority, Status, Task};

/// Words that only introduce a date ("for Friday", "by tomorrow").
const DATE_CONNECTORS: &[&str] = &["for", "on", "by", "due", "until", "before"];

/// Reads `input` into a new task, dates counted from `today`.
#[must_use]
pub fn parse(input: &str, today: NaiveDate) -> Task {
    let ymd = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    let (tokens, recur) = recurrence(&tokenize(input), today);
    let (tokens, phrase_due) = next_phrase(tokens, today);
    let mut task = Task {
        status: Status::Open.as_str().to_owned(),
        priority: Priority::Normal.as_str().to_owned(),
        ..Task::default()
    };
    let mut due = None;
    let mut title: Vec<String> = Vec::new();
    for tok in tokens {
        if let Some(tag) = tok.strip_prefix('#').filter(|t| !t.is_empty()) {
            task.tags.push(tag.to_owned());
        } else if let Some(ctx) = tok.strip_prefix('@').filter(|c| !c.is_empty()) {
            task.contexts.push(format!("@{ctx}"));
        } else if let Some(link) = tok
            .strip_prefix("[[")
            .and_then(|s| s.strip_suffix("]]"))
            .filter(|l| !l.is_empty())
        {
            task.projects.push(format!("[[{link}]]"));
        } else if let Some(p) = tok.strip_prefix('!').and_then(Priority::parse) {
            p.as_str().clone_into(&mut task.priority);
        } else if let Some(d) = date(&tok, today) {
            due = Some(ymd(d));
            if title
                .last()
                .is_some_and(|w| DATE_CONNECTORS.contains(&w.to_lowercase().as_str()))
            {
                title.pop();
            }
        } else {
            title.push(tok);
        }
    }
    // A date typed beats the start a repeat implies.
    task.due = due
        .or(phrase_due)
        .or_else(|| recur.as_ref().and_then(|r| r.first.map(ymd)));
    if let Some(r) = recur {
        task.recurrence = Some(r.rule);
        task.recurrence_anchor = Some(r.anchor.to_owned());
    }
    let title = title.join(" ");
    task.title = if title.trim().is_empty() {
        "Untitled task".to_owned()
    } else {
        title.trim().to_owned()
    };
    task
}

/// Whitespace-split, a `[[multi word link]]` one token.
fn tokenize(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_link = false;
    for ch in src.chars() {
        if in_link {
            cur.push(ch);
            if cur.ends_with("]]") {
                out.push(std::mem::take(&mut cur));
                in_link = false;
            }
            continue;
        }
        if ch == '[' && cur.is_empty() {
            cur.push(ch);
            in_link = true;
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn weekday(tok: &str) -> Option<Weekday> {
    Some(match tok.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "weds" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

fn date(tok: &str, today: NaiveDate) -> Option<NaiveDate> {
    match tok.to_ascii_lowercase().as_str() {
        "today" => Some(today),
        "tomorrow" | "tmrw" => Some(today + Duration::days(1)),
        _ => weekday(tok)
            .map(|w| next_weekday(today, w))
            .or_else(|| NaiveDate::parse_from_str(tok, "%Y-%m-%d").ok()),
    }
}

/// `next <weekday>` (two tokens), before a lone weekday reads the second.
fn next_phrase(tokens: Vec<String>, today: NaiveDate) -> (Vec<String>, Option<String>) {
    let mut out = Vec::with_capacity(tokens.len());
    let mut due = None;
    let mut it = tokens.into_iter().peekable();
    while let Some(tok) = it.next() {
        if due.is_none()
            && tok.eq_ignore_ascii_case("next")
            && let Some(d) = it.peek().and_then(|n| date(n, today))
        {
            due = Some(d.format("%Y-%m-%d").to_string());
            let _ = it.next();
            continue;
        }
        out.push(tok);
    }
    (out, due)
}

struct Recurrence {
    rule: String,
    anchor: &'static str,
    first: Option<NaiveDate>,
}

const fn rrule_day(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

fn freq(tok: &str) -> Option<&'static str> {
    Some(match tok.to_ascii_lowercase().as_str() {
        "day" | "days" => "DAILY",
        "week" | "weeks" => "WEEKLY",
        "month" | "months" => "MONTHLY",
        "year" | "years" => "YEARLY",
        _ => return None,
    })
}

/// The first repeat phrase, taken out of the tokens (a second is left in
/// the title).
#[allow(clippy::indexing_slicing)]
fn recurrence(tokens: &[String], today: NaiveDate) -> (Vec<String>, Option<Recurrence>) {
    let mut out = Vec::with_capacity(tokens.len());
    let mut found: Option<Recurrence> = None;
    let mut i = 0;
    while i < tokens.len() {
        if found.is_some() {
            out.push(tokens[i].clone());
            i += 1;
            continue;
        }
        let bare = match tokens[i].to_ascii_lowercase().as_str() {
            "daily" => Some("DAILY"),
            "weekly" => Some("WEEKLY"),
            "monthly" => Some("MONTHLY"),
            "yearly" | "annually" => Some("YEARLY"),
            _ => None,
        };
        if let Some(f) = bare {
            found = Some(Recurrence {
                rule: format!("FREQ={f}"),
                anchor: "completion",
                first: Some(today),
            });
            i += 1;
            continue;
        }
        if !tokens[i].eq_ignore_ascii_case("every") || i + 1 >= tokens.len() {
            out.push(tokens[i].clone());
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut interval = 1_u32;
        if tokens[j].eq_ignore_ascii_case("other") {
            interval = 2;
            j += 1;
        } else if let Ok(n) = tokens[j].parse::<u32>()
            && n > 0
        {
            interval = n;
            j += 1;
        }
        let parsed = tokens.get(j).and_then(|unit| {
            if unit.eq_ignore_ascii_case("weekday") || unit.eq_ignore_ascii_case("weekdays") {
                let first = match today.weekday() {
                    Weekday::Sat | Weekday::Sun => next_weekday(today, Weekday::Mon),
                    _ => today,
                };
                Some((
                    "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_owned(),
                    "scheduled",
                    first,
                ))
            } else if let Some(w) = weekday(unit) {
                Some((
                    format!("FREQ=WEEKLY;BYDAY={}", rrule_day(w)),
                    "scheduled",
                    next_weekday(today, w),
                ))
            } else {
                freq(unit).map(|f| (format!("FREQ={f}"), "completion", today))
            }
        });
        let Some((rule_base, anchor, first)) = parsed else {
            out.push(tokens[i].clone());
            i += 1;
            continue;
        };
        let rule = if interval > 1 {
            format!("{rule_base};INTERVAL={interval}")
        } else {
            rule_base
        };
        found = Some(Recurrence {
            rule,
            anchor,
            first: Some(first),
        });
        i = j + 1;
    }
    (out, found)
}

/// The next `target` after `today` (a week on when it is today).
fn next_weekday(today: NaiveDate, target: Weekday) -> NaiveDate {
    let now = i64::from(today.weekday().num_days_from_monday());
    let want = i64::from(target.num_days_from_monday());
    let delta = (want - now).rem_euclid(7);
    today + Duration::days(if delta == 0 { 7 } else { delta })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A Thursday.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()
    }

    #[test]
    fn tokens_become_properties_and_the_rest_the_title() {
        let t = parse(
            "Call the venue [[Album]] #gigs @phone !high tomorrow",
            today(),
        );
        assert_eq!(t.title, "Call the venue");
        assert_eq!(t.projects, ["[[Album]]"]);
        assert_eq!(t.tags, ["gigs"]);
        assert_eq!(t.contexts, ["@phone"]);
        assert_eq!(t.priority, "high");
        assert_eq!(t.due.as_deref(), Some("2026-11-06"));
        assert_eq!(t.status, "open");
    }

    #[test]
    fn the_word_that_introduced_a_date_leaves_with_it() {
        let t = parse("Book the booth for friday", today());
        assert_eq!(t.title, "Book the booth");
        assert_eq!(t.due.as_deref(), Some("2026-11-06"));
        assert_eq!(
            parse("Plan next monday", today()).due.as_deref(),
            Some("2026-11-09")
        );
    }

    #[test]
    fn repeats_and_their_anchors() {
        let t = parse("Standup every monday", today());
        assert_eq!(t.title, "Standup");
        assert_eq!(t.recurrence.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
        assert_eq!(t.recurrence_anchor.as_deref(), Some("scheduled"));
        assert_eq!(t.due.as_deref(), Some("2026-11-09"));
        let t = parse("Water plants every 3 days", today());
        assert_eq!(t.recurrence.as_deref(), Some("FREQ=DAILY;INTERVAL=3"));
        assert_eq!(t.recurrence_anchor.as_deref(), Some("completion"));
        assert_eq!(t.due.as_deref(), Some("2026-11-05"), "starts today");
        let t = parse("Review every weekday", today());
        assert_eq!(
            t.recurrence.as_deref(),
            Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR")
        );
        assert_eq!(
            parse("Read every so often", today()).title,
            "Read every so often"
        );
        assert!(crate::model::Repeat::parse(t.recurrence.as_deref().unwrap()).is_some());
    }

    #[test]
    fn a_typed_date_beats_the_repeat_start() {
        let t = parse("Gym 2026-12-01 weekly", today());
        assert_eq!(t.due.as_deref(), Some("2026-12-01"));
        assert_eq!(parse("", today()).title, "Untitled task");
    }
}
