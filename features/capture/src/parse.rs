//! The quick-add parser: a line as typed read into a [`Draft`], the one
//! parser every capture surface and target shares (the composer, the
//! global window, `CAPTURE_ADD`, Tasks' `TASKS_ADD`). Ported from Task's
//! capture (`task:features/task/task-proto/src/capture.rs` `capture`),
//! with the same rules.
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
//! - a token an extension adds (`[[capture_targets]] tokens`, a
//!   [`TokenRule`]: Keyflow's `key:D`) → its property;
//! - the rest is the title.
//!
//! A chip set by hand beats the text: [`merge`] lays the composer's
//! properties over the parsed ones.

// Token indices within one line, and a day or a week on a date.
#![allow(clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Words that only introduce a date ("for Friday", "by tomorrow").
const DATE_CONNECTORS: &[&str] = &["for", "on", "by", "due", "until", "before"];

/// The frontmatter keys a draft writes: `TaskNotes`' names, so a capture
/// turned into a task keeps them as they are.
pub mod keys {
    pub const TAGS: &str = "tags";
    pub const CONTEXTS: &str = "contexts";
    pub const PROJECTS: &str = "projects";
    pub const PRIORITY: &str = "priority";
    pub const DUE: &str = "due";
    pub const RECURRENCE: &str = "recurrence";
    pub const RECURRENCE_ANCHOR: &str = "recurrence_anchor";
}

/// A line read: its title and the properties its tokens gave.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// What is left once the tokens are out; empty when nothing is.
    pub title: String,
    /// Without the `#`.
    pub tags: Vec<String>,
    /// With the `@` (`TaskNotes`' shape).
    pub contexts: Vec<String>,
    /// As links: `[[Album]]`.
    pub projects: Vec<String>,
    /// `low`, `normal`, `high` or `critical`.
    pub priority: Option<String>,
    pub due: Option<NaiveDate>,
    pub repeat: Option<Repeat>,
    /// What extensions' tokens gave: property → value.
    pub extra: BTreeMap<String, String>,
}

/// A repeat: an RRULE and what its next time is counted from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repeat {
    /// `FREQ=WEEKLY;BYDAY=MO`.
    pub rule: String,
    /// `scheduled` (the calendar decides) or `completion` (from when it
    /// was done).
    pub anchor: String,
}

/// A token an extension adds to quick add (`[[capture_targets]]
/// tokens`): a word starting with `prefix` gives property `key` the rest.
///
/// ```toml
/// tokens = [{ prefix = "key:", key = "key" }]   # `key:D` → key: D
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TokenRule {
    pub prefix: String,
    pub key: String,
}

/// Reads `input`, dates counted from `today`.
#[must_use]
pub fn parse(input: &str, today: NaiveDate) -> Draft {
    parse_with(input, today, &[])
}

/// [`parse`], extensions' `rules` read too (the first that matches a
/// word takes it; the core's tokens come first).
#[must_use]
pub fn parse_with(input: &str, today: NaiveDate, rules: &[TokenRule]) -> Draft {
    let (tokens, recur) = recurrence(&tokenize(input), today);
    let (tokens, phrase_due) = next_phrase(tokens, today);
    let mut draft = Draft::default();
    let mut due = None;
    let mut title: Vec<String> = Vec::new();
    for tok in tokens {
        if let Some(tag) = tok.strip_prefix('#').filter(|t| !t.is_empty()) {
            draft.tags.push(tag.to_owned());
        } else if let Some(ctx) = tok.strip_prefix('@').filter(|c| !c.is_empty()) {
            draft.contexts.push(format!("@{ctx}"));
        } else if let Some(link) = tok
            .strip_prefix("[[")
            .and_then(|s| s.strip_suffix("]]"))
            .filter(|l| !l.is_empty())
        {
            draft.projects.push(format!("[[{link}]]"));
        } else if let Some(p) = tok.strip_prefix('!').and_then(priority) {
            draft.priority = Some(p.to_owned());
        } else if let Some(d) = date(&tok, today) {
            due = Some(d);
            if title
                .last()
                .is_some_and(|w| DATE_CONNECTORS.contains(&w.to_lowercase().as_str()))
            {
                title.pop();
            }
        } else if let Some((key, value)) = rules.iter().find_map(|r| {
            (!r.prefix.is_empty() && !r.key.is_empty())
                .then(|| tok.strip_prefix(r.prefix.as_str()))
                .flatten()
                .filter(|v| !v.is_empty())
                .map(|v| (r.key.clone(), v.to_owned()))
        }) {
            draft.extra.insert(key, value);
        } else {
            title.push(tok);
        }
    }
    // A date typed beats the start a repeat implies.
    draft.due = due
        .or(phrase_due)
        .or_else(|| recur.as_ref().and_then(|r| r.first));
    draft.repeat = recur.map(|r| Repeat {
        rule: r.rule,
        anchor: r.anchor.to_owned(),
    });
    draft.title = title.join(" ").trim().to_owned();
    draft
}

/// A priority word, as `!word` says it: its canonical name.
#[must_use]
pub fn priority(word: &str) -> Option<&'static str> {
    Some(match word.trim().to_ascii_lowercase().as_str() {
        "low" | "4" => "low",
        "normal" | "medium" | "med" | "3" => "normal",
        "high" | "2" => "high",
        "critical" | "urgent" | "1" => "critical",
        _ => return None,
    })
}

impl Draft {
    /// The frontmatter its tokens give ([`keys`]): only what was typed.
    #[must_use]
    pub fn properties(&self) -> Map<String, Value> {
        let mut out = Map::new();
        let list = |v: &[String]| Value::Array(v.iter().cloned().map(Value::String).collect());
        if !self.tags.is_empty() {
            out.insert(keys::TAGS.to_owned(), list(&self.tags));
        }
        if !self.contexts.is_empty() {
            out.insert(keys::CONTEXTS.to_owned(), list(&self.contexts));
        }
        if !self.projects.is_empty() {
            out.insert(keys::PROJECTS.to_owned(), list(&self.projects));
        }
        if let Some(p) = &self.priority {
            out.insert(keys::PRIORITY.to_owned(), Value::String(p.clone()));
        }
        if let Some(d) = self.due {
            out.insert(
                keys::DUE.to_owned(),
                Value::String(d.format("%Y-%m-%d").to_string()),
            );
        }
        if let Some(r) = &self.repeat {
            out.insert(keys::RECURRENCE.to_owned(), Value::String(r.rule.clone()));
            out.insert(
                keys::RECURRENCE_ANCHOR.to_owned(),
                Value::String(r.anchor.clone()),
            );
        }
        for (k, v) in &self.extra {
            out.insert(k.clone(), Value::String(v.clone()));
        }
        out
    }
}

/// The text's properties with the hand-set ones laid over them: a chip
/// set by hand beats the text, and a hand-set `null` takes one away.
#[must_use]
pub fn merge(text: Map<String, Value>, hand: &Map<String, Value>) -> Map<String, Value> {
    let mut out = text;
    for (k, v) in hand {
        if v.is_null() {
            out.remove(k);
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    out
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
fn next_phrase(tokens: Vec<String>, today: NaiveDate) -> (Vec<String>, Option<NaiveDate>) {
    let mut out = Vec::with_capacity(tokens.len());
    let mut due = None;
    let mut it = tokens.into_iter().peekable();
    while let Some(tok) = it.next() {
        if due.is_none()
            && tok.eq_ignore_ascii_case("next")
            && let Some(d) = it.peek().and_then(|n| date(n, today))
        {
            due = Some(d);
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
    use serde_json::json;

    use super::*;

    /// A Thursday.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()
    }

    fn ymd(d: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()
    }

    #[test]
    fn tokens_become_properties_and_the_rest_the_title() {
        let d = parse(
            "Call the venue [[Album]] #gigs @phone !high tomorrow",
            today(),
        );
        assert_eq!(d.title, "Call the venue");
        assert_eq!(d.projects, ["[[Album]]"]);
        assert_eq!(d.tags, ["gigs"]);
        assert_eq!(d.contexts, ["@phone"]);
        assert_eq!(d.priority.as_deref(), Some("high"));
        assert_eq!(d.due, ymd("2026-11-06"));
    }

    #[test]
    fn the_issue_s_line_turns_into_chips() {
        let d = parse("Sketch the toolbar [[Lab]] tomorrow !high #design", today());
        assert_eq!(d.title, "Sketch the toolbar");
        let p = d.properties();
        assert_eq!(p["projects"], json!(["[[Lab]]"]));
        assert_eq!(p["due"], json!("2026-11-06"));
        assert_eq!(p["priority"], json!("high"));
        assert_eq!(p["tags"], json!(["design"]));
        assert!(!p.contains_key("recurrence"), "only what was typed");
    }

    #[test]
    fn the_word_that_introduced_a_date_leaves_with_it() {
        let d = parse("Book the booth for friday", today());
        assert_eq!(d.title, "Book the booth");
        assert_eq!(d.due, ymd("2026-11-06"));
        assert_eq!(parse("Plan next monday", today()).due, ymd("2026-11-09"));
    }

    #[test]
    fn repeats_and_their_anchors() {
        let d = parse("Standup every monday", today());
        assert_eq!(d.title, "Standup");
        let r = d.repeat.unwrap();
        assert_eq!(r.rule, "FREQ=WEEKLY;BYDAY=MO");
        assert_eq!(r.anchor, "scheduled");
        assert_eq!(d.due, ymd("2026-11-09"));
        let d = parse("Water plants every 3 days", today());
        let r = d.repeat.unwrap();
        assert_eq!(r.rule, "FREQ=DAILY;INTERVAL=3");
        assert_eq!(r.anchor, "completion");
        assert_eq!(d.due, ymd("2026-11-05"), "starts today");
        let d = parse("Review every weekday", today());
        assert_eq!(d.repeat.unwrap().rule, "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR");
        assert_eq!(
            parse("Read every so often", today()).title,
            "Read every so often"
        );
    }

    #[test]
    fn a_typed_date_beats_the_repeat_start() {
        let d = parse("Gym 2026-12-01 weekly", today());
        assert_eq!(d.due, ymd("2026-12-01"));
        assert_eq!(parse("  ", today()).title, "");
    }

    #[test]
    fn an_extension_s_token_gives_its_property() {
        let rules = [TokenRule {
            prefix: "key:".to_owned(),
            key: "key".to_owned(),
        }];
        let d = parse_with("Bridge idea key:D #song", today(), &rules);
        assert_eq!(d.title, "Bridge idea");
        assert_eq!(d.extra.get("key").map(String::as_str), Some("D"));
        assert_eq!(d.properties()["key"], json!("D"));
        assert_eq!(parse("Bridge idea key:D", today()).title, "Bridge idea key:D");
    }

    #[test]
    fn a_chip_set_by_hand_beats_the_text() {
        let text = parse("Mix [[Album]] !low", today()).properties();
        let hand = json!({ "priority": "critical", "projects": null, "status": "waiting" });
        let out = merge(text, hand.as_object().unwrap());
        assert_eq!(out["priority"], json!("critical"));
        assert!(!out.contains_key("projects"), "a hand-set null takes it away");
        assert_eq!(out["status"], json!("waiting"));
    }
}
