//! Which template a day uses, and why: ordered rules (first match wins),
//! overrides that beat them, and the rules each past day had.
//!
//! - **Rules**, in order: by weekday (Mon–Fri → Workday), by an RRULE
//!   (every other Saturday → Studio day), by a date range (a tour week →
//!   Road day), by a calendar condition (a day with an event tagged
//!   `#gig` → Gig day).
//! - **Overrides**, for one date (beats everything) or a week or a range
//!   (beats every rule); `template: none` clears a day.
//! - **Only the future changes.** A change to the rules applies from today
//!   on: the rules they replace are kept until yesterday
//!   ([`RulesRevision`]), so a past day resolves as it did then. An
//!   override is the one thing that changes a past day.

use chrono::{Datelike as _, Duration, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

/// When a rule applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    /// On these weekdays.
    Weekdays(Vec<Weekday>),
    /// On the days an RRULE falls on, from `start`.
    Rrule { rule: String, start: NaiveDate },
    /// From `from` to `to`, both included.
    Dates { from: NaiveDate, to: NaiveDate },
    /// On a day with an event tagged `#tag` (or with it in its title).
    Tag(String),
}

/// One rule: a template, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RuleRow", into = "RuleRow")]
pub struct Rule {
    pub template: String,
    pub when: When,
}

/// A rule as written: one map, its condition's key beside `template`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RuleRow {
    template: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weekdays: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rrule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

/// A weekday's short name (`mon`).
#[must_use]
pub const fn weekday_id(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

/// `mon`, `Monday`, `MO`.
#[must_use]
pub fn weekday(text: &str) -> Option<Weekday> {
    let t = text.trim().to_ascii_lowercase();
    let t = t.get(..2)?;
    Some(match t {
        "mo" => Weekday::Mon,
        "tu" => Weekday::Tue,
        "we" => Weekday::Wed,
        "th" => Weekday::Thu,
        "fr" => Weekday::Fri,
        "sa" => Weekday::Sat,
        "su" => Weekday::Sun,
        _ => return None,
    })
}

impl TryFrom<RuleRow> for Rule {
    type Error = String;

    fn try_from(r: RuleRow) -> Result<Self, String> {
        let template = r.template.trim().to_owned();
        if template.is_empty() {
            return Err("a rule names no template".into());
        }
        let when = if let Some(days) = r.weekdays {
            let days: Option<Vec<Weekday>> = days.iter().map(|d| weekday(d)).collect();
            When::Weekdays(days.ok_or("a rule's weekdays are mon, tue, … sun")?)
        } else if let Some(rule) = r.rrule {
            architect_calendar::recur::Rule::parse(&rule)
                .ok_or_else(|| format!("not an RRULE: {rule}"))?;
            When::Rrule {
                rule,
                start: r.start.ok_or("an RRULE rule needs its start")?,
            }
        } else if let (Some(from), Some(to)) = (r.from, r.to) {
            When::Dates { from, to }
        } else if let Some(tag) = r.tag {
            When::Tag(tag.trim().trim_start_matches('#').to_owned())
        } else {
            return Err(format!(
                "rule for {template} says no weekdays, rrule, from/to or tag"
            ));
        };
        Ok(Self { template, when })
    }
}

impl From<Rule> for RuleRow {
    fn from(r: Rule) -> Self {
        let mut row = Self {
            template: r.template,
            ..Self::default()
        };
        match r.when {
            When::Weekdays(days) => {
                row.weekdays = Some(days.into_iter().map(|d| weekday_id(d).into()).collect());
            }
            When::Rrule { rule, start } => {
                row.rrule = Some(rule);
                row.start = Some(start);
            }
            When::Dates { from, to } => {
                row.from = Some(from);
                row.to = Some(to);
            }
            When::Tag(t) => row.tag = Some(t),
        }
        row
    }
}

/// Days in words: `Mon–Fri`, `Sat, Sun`, `every day`.
fn days_words(days: &[Weekday]) -> String {
    let short = |d: &Weekday| {
        let s = weekday_id(*d);
        let mut c = s.chars();
        c.next()
            .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
            .unwrap_or_default()
    };
    let mut nums: Vec<u32> = days.iter().map(Weekday::num_days_from_monday).collect();
    nums.sort_unstable();
    nums.dedup();
    if nums.len() == 7 {
        return "every day".into();
    }
    let run = nums.len() > 2 && nums.windows(2).all(|w| w[1] == w[0] + 1);
    let named: Vec<String> = nums
        .iter()
        .filter_map(|n| Weekday::try_from(u8::try_from(*n).ok()?).ok())
        .map(|d| short(&d))
        .collect();
    match (run, named.first(), named.last()) {
        (true, Some(a), Some(b)) => format!("{a}–{b}"),
        _ => named.join(", "),
    }
}

impl Rule {
    /// Whether it applies on `date`; `has_tag(tag, date)` says whether a
    /// day has an event tagged so.
    #[must_use]
    pub fn matches(&self, date: NaiveDate, has_tag: &dyn Fn(&str, NaiveDate) -> bool) -> bool {
        match &self.when {
            When::Weekdays(days) => days.contains(&date.weekday()),
            When::Rrule { rule, start } => {
                let Some(r) = architect_calendar::recur::Rule::parse(rule) else {
                    return false;
                };
                let (Some(s), Some(d)) = (start.and_hms_opt(0, 0, 0), date.and_hms_opt(0, 0, 0))
                else {
                    return false;
                };
                !r.between(s, d, d + Duration::days(1)).is_empty()
            }
            When::Dates { from, to } => (*from..=*to).contains(&date),
            When::Tag(tag) => has_tag(tag, date),
        }
    }

    /// When, in words: `Mon–Fri`, `every 2 weeks on Saturday`,
    /// `1 Dec – 7 Dec`, `a day with #gig`.
    #[must_use]
    pub fn words(&self) -> String {
        match &self.when {
            When::Weekdays(days) => days_words(days),
            When::Rrule { rule, .. } => architect_calendar::recur::describe(rule),
            When::Dates { from, to } => {
                format!("{} – {}", from.format("%-d %b"), to.format("%-d %b %Y"))
            }
            When::Tag(t) => format!("a day with #{t}"),
        }
    }
}

/// How far an override reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Span {
    /// One date: beats everything.
    Day,
    /// A week, from its first day.
    Week,
    /// A run of dates.
    Range,
}

/// A template chosen for some days by hand (`template: None` clears
/// them: no template).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Override {
    pub from: NaiveDate,
    /// Included.
    pub to: NaiveDate,
    pub span: Span,
    #[serde(default)]
    pub template: Option<String>,
}

impl Override {
    /// For one day.
    #[must_use]
    pub const fn day(date: NaiveDate, template: Option<String>) -> Self {
        Self {
            from: date,
            to: date,
            span: Span::Day,
            template,
        }
    }

    /// For the week from `start`.
    #[must_use]
    pub fn week(start: NaiveDate, template: Option<String>) -> Self {
        Self {
            from: start,
            to: start + Duration::days(6),
            span: Span::Week,
            template,
        }
    }

    fn covers(&self, date: NaiveDate) -> bool {
        (self.from..=self.to).contains(&date)
    }
}

/// The rules as they were up to and including `until`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesRevision {
    pub until: NaiveDate,
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub fallback: Option<String>,
}

/// The rules note (`Calendar/Day templates/Rules.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// The template a day no rule matches uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// The rules before each change, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<RulesRevision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<Override>,
}

/// Why a day uses its template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    /// Chosen by hand for it.
    Override {
        span: Span,
        from: NaiveDate,
        to: NaiveDate,
    },
    /// Rule `number` (from 1) of the rules in effect that day.
    Rule { number: usize, words: String },
    /// No rule matched: the fallback.
    Fallback,
    /// Nothing says: no template.
    Nothing,
    /// Its plan was saved with it (an edited day keeps its template).
    Saved,
}

impl Reason {
    /// In words, for the badge's tooltip: `Rule 1: Mon–Fri`, `Override
    /// for this day`, `Override for the week of 9 Nov`.
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            Self::Override {
                span: Span::Day, ..
            } => "Override for this day".into(),
            Self::Override {
                span: Span::Week,
                from,
                ..
            } => format!("Override for the week of {}", from.format("%-d %b")),
            Self::Override { from, to, .. } => format!(
                "Override for {} – {}",
                from.format("%-d %b"),
                to.format("%-d %b")
            ),
            Self::Rule { number, words } => format!("Rule {number}: {words}"),
            Self::Fallback => "No rule matched: the default".into(),
            Self::Nothing => "No rule matched".into(),
            Self::Saved => "Kept with its edited plan".into(),
        }
    }

    /// Whether it was chosen by hand.
    #[must_use]
    pub const fn is_override(&self) -> bool {
        matches!(self, Self::Override { .. })
    }
}

/// The template a day resolves to, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub template: Option<String>,
    pub reason: Reason,
}

impl Rules {
    /// The rules and fallback in effect on `date`.
    #[must_use]
    pub fn in_effect(&self, date: NaiveDate) -> (&[Rule], Option<&str>) {
        self.history
            .iter()
            .filter(|r| r.until >= date)
            .min_by_key(|r| r.until)
            .map_or((&self.rules, self.fallback.as_deref()), |r| {
                (&r.rules, r.fallback.as_deref())
            })
    }

    /// The override for `date`, if any: one for the day beats one for a
    /// week or a range; of two alike, the one made last.
    #[must_use]
    pub fn override_on(&self, date: NaiveDate) -> Option<&Override> {
        let on = || self.overrides.iter().rev().filter(|o| o.covers(date));
        on().find(|o| o.span == Span::Day).or_else(|| on().next())
    }

    /// Which template `date` uses, and why.
    #[must_use]
    pub fn resolve(
        &self,
        date: NaiveDate,
        has_tag: &dyn Fn(&str, NaiveDate) -> bool,
    ) -> Resolution {
        if let Some(o) = self.override_on(date) {
            return Resolution {
                template: o.template.clone(),
                reason: Reason::Override {
                    span: o.span,
                    from: o.from,
                    to: o.to,
                },
            };
        }
        let (rules, fallback) = self.in_effect(date);
        if let Some((i, r)) = rules
            .iter()
            .enumerate()
            .find(|(_, r)| r.matches(date, has_tag))
        {
            return Resolution {
                template: Some(r.template.clone()),
                reason: Reason::Rule {
                    number: i + 1,
                    words: r.words(),
                },
            };
        }
        fallback.map_or(
            Resolution {
                template: None,
                reason: Reason::Nothing,
            },
            |f| Resolution {
                template: Some(f.to_owned()),
                reason: Reason::Fallback,
            },
        )
    }

    /// The rules from `today` on are `rules` (and `fallback`): the ones
    /// they replace stay in effect until yesterday, unless a change
    /// earlier today already kept them.
    pub fn change(&mut self, rules: Vec<Rule>, fallback: Option<String>, today: NaiveDate) {
        if rules == self.rules && fallback == self.fallback {
            return;
        }
        if let Some(yesterday) = today.pred_opt()
            && !self.history.iter().any(|r| r.until >= yesterday)
        {
            self.history.push(RulesRevision {
                until: yesterday,
                rules: std::mem::take(&mut self.rules),
                fallback: self.fallback.take(),
            });
        }
        self.rules = rules;
        self.fallback = fallback;
    }

    /// Sets `template` (or none) on `from`..=`to` by hand, replacing the
    /// overrides of the same reach it covers exactly; `clear` takes them
    /// away instead.
    pub fn set_override(&mut self, o: Override, clear: bool) {
        self.overrides
            .retain(|x| !(x.from == o.from && x.to == o.to && x.span == o.span));
        if !clear {
            self.overrides.push(o);
        }
    }

    /// Renames template `from` to `to` in the rules, the history and the
    /// overrides.
    pub fn rename_template(&mut self, from: &str, to: &str) {
        let fix = |t: &mut String| {
            if t.eq_ignore_ascii_case(from) {
                to.clone_into(t);
            }
        };
        let fix_opt = |t: &mut Option<String>| {
            if let Some(t) = t {
                fix(t);
            }
        };
        self.rules.iter_mut().for_each(|r| fix(&mut r.template));
        fix_opt(&mut self.fallback);
        for h in &mut self.history {
            h.rules.iter_mut().for_each(|r| fix(&mut r.template));
            fix_opt(&mut h.fallback);
        }
        self.overrides
            .iter_mut()
            .for_each(|o| fix_opt(&mut o.template));
    }
}
