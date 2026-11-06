//! The built-in template engine (Notebook Navigator's
//! `src/utils/templateRenderer.ts`), with the numbered file names and the
//! folder templates of `src/utils/fileCreationUtils.ts`.
//!
//! A template is text with `{{token}}` placeholders; everything else is
//! copied as it is, `{{…}}` whose name is no token too, so templates
//! shared with other tools keep working. A token is
//! `{{ name [+|-amount unit] [:format] }}`:
//!
//! - text: `{{title}}`, `{{folder}}`, `{{path}}`;
//! - the note's date: `{{date}}`, `{{yesterday}}`, `{{tomorrow}}`,
//!   `{{monday}}` … `{{sunday}}`;
//! - the clock: `{{time}}`, `{{today}}`, `{{now}}` (an ISO timestamp);
//! - `{{cursor}}`, where the editor's cursor goes;
//! - `{{prompt:Label}}` (`{{value:Label}}`; the label is `Value` when
//!   none), asked before the note is made;
//! - `{{number}}`, a numbered name's number (`{{number:000}}` pads it);
//! - `{{!name}}` writes `{{name}}`.
//!
//! Offsets and formats are for dates and the clock (a number's format is
//! its padding). Offsets are Moment's units: `y`, `Q`, `M`, `w`, `d`, `h`,
//! `m`, `s`. Formats are Moment's ([`crate::pattern::Pattern`]), and hold
//! no braces. One pass: what a token writes is not read again.

use std::collections::BTreeMap;

use std::fmt::Write as _;

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime};
use facet::Facet;

use crate::pattern::Pattern;
use crate::period::WeekRules;

/// `{{now}}`'s format (`ISO_DATE_FORMAT`).
pub const ISO_FORMAT: &str = "YYYY-MM-DD[T]HH:mm:ssZ";

/// Offsets above this are refused (Moment makes no date of them).
const MAX_OFFSET: i64 = 100_000;

/// A prompt's label when it names none.
pub const DEFAULT_PROMPT: &str = "Value";

/// Which week `{{monday}}` … `{{sunday}}` fall in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WeekdayBase {
    /// The week (by the week rules) the note's date is in.
    #[default]
    LocaleWeek,
    /// The seven days from the note's date (a weekly note's).
    NoteDate,
}

/// What `{{number}}` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Number {
    /// Nothing to write: the token is reported as invalid (only a
    /// numbered name has a number).
    #[default]
    None,
    /// The token is taken out and its place reported
    /// ([`Rendered::number_slots`]), so a name can be matched against the
    /// notes in a folder before the number is chosen.
    Slot,
    Value(u64),
}

/// What a template is written for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context<'a> {
    /// The note's name, no extension.
    pub title: &'a str,
    /// The name of the folder it is in; empty at the root.
    pub folder: &'a str,
    /// Its vault path, with the extension.
    pub path: &'a str,
    /// The day `{{date}}` and the weekdays are of (a periodic note's
    /// first day); today when none.
    pub date: Option<NaiveDate>,
    /// `{{date}}`'s format, and its neighbours'.
    pub date_format: &'a str,
    /// `{{today}}`'s format, and the weekdays'.
    pub today_format: &'a str,
    /// `{{time}}`'s.
    pub time_format: &'a str,
    pub weekday_base: WeekdayBase,
    /// Now, in the person's time zone; one reading for every token.
    pub now: NaiveDateTime,
    /// The time zone, seconds east of UTC (`{{now}}`'s `Z`).
    pub offset: i32,
    pub weeks: WeekRules,
    /// The prompts' answers, by label.
    pub prompts: &'a BTreeMap<String, String>,
    pub number: Number,
}

/// A `{{number}}` taken out: where it was in the text, and its padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberSlot {
    pub offset: usize,
    pub padding: usize,
}

/// A template written.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rendered {
    /// The text, with the `{{cursor}}` tokens taken out.
    pub content: String,
    /// The first `{{cursor}}`'s line and column (in characters).
    pub cursor: Option<(usize, usize)>,
    pub number_slots: Vec<NumberSlot>,
    /// Known tokens that do not read or write, as written; they stay in
    /// the text.
    pub invalid: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Year,
    Quarter,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
}

impl Unit {
    /// Moment's letter: `M` is a month and `m` a minute; the others take
    /// either case.
    const fn of(c: char) -> Option<Self> {
        Some(match c {
            'y' | 'Y' => Self::Year,
            'q' | 'Q' => Self::Quarter,
            'M' => Self::Month,
            'm' => Self::Minute,
            'w' | 'W' => Self::Week,
            'd' | 'D' => Self::Day,
            'h' | 'H' => Self::Hour,
            's' | 'S' => Self::Second,
            _ => return None,
        })
    }

    /// Moves whole days: applied before the clock's time is put on.
    const fn calendar(self) -> bool {
        matches!(
            self,
            Self::Year | Self::Quarter | Self::Month | Self::Week | Self::Day
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    name: String,
    offset: Option<(i64, Unit)>,
    format: Option<String>,
}

const TEXT: &[&str] = &["title", "folder", "path", "cursor"];
const WEEKDAYS: &[&str] = &[
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];
const DATES: &[&str] = &["date", "yesterday", "tomorrow", "time", "today", "now"];
const PROMPTS: &[&str] = &["prompt", "value"];
const NUMBER: &str = "number";

fn known(name: &str) -> bool {
    TEXT.contains(&name)
        || DATES.contains(&name)
        || WEEKDAYS.contains(&name)
        || PROMPTS.contains(&name)
        || name == NUMBER
}

/// The name a token starts with, when it ends where a space, a format, an
/// offset or the token does (so `{{date_created}}` and `{{title-x}}` are
/// unknown, left alone, not known and malformed).
// Byte offsets inside one text: none can overflow.
#[allow(clippy::arithmetic_side_effects)]
fn name_of(body: &str) -> String {
    let end = body
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(body.len());
    let (name, rest) = body.split_at(end);
    let ends = rest.is_empty()
        || rest.starts_with(char::is_whitespace)
        || rest.starts_with(':')
        || ((rest.starts_with('+') || rest.starts_with('-'))
            && rest
                .get(1..)
                .unwrap_or("")
                .trim_start()
                .starts_with(|c: char| c.is_ascii_digit()));
    if ends && !name.is_empty() {
        name.to_ascii_lowercase()
    } else {
        String::new()
    }
}

/// A token's body (`TOKEN_BODY_PATTERN`): a name, a signed offset with its
/// unit, a format after the first colon. `None` when it does not read, an
/// offset is out of range, or a text token has an offset or a format.
// Byte offsets inside one text: none can overflow.
#[allow(clippy::arithmetic_side_effects)]
fn parse_body(body: &str) -> Option<Token> {
    let end = body
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(body.len());
    let (name, mut rest) = body.split_at(end);
    if name.is_empty() {
        return None;
    }
    let name = name.to_ascii_lowercase();
    rest = rest.trim_start();
    let mut offset = None;
    if let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        let after = rest.get(1..).unwrap_or("").trim_start();
        let digits = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        let (num, after) = after.split_at(digits);
        let after = after.trim_start();
        let unit_char = after.chars().next()?;
        if num.is_empty() || !unit_char.is_ascii_alphabetic() {
            return None;
        }
        let unit = Unit::of(unit_char)?;
        let amount: i64 = num.parse().ok()?;
        if amount > MAX_OFFSET {
            return None;
        }
        offset = Some((
            if sign == '-' {
                amount.saturating_neg()
            } else {
                amount
            },
            unit,
        ));
        rest = after.get(1..).unwrap_or("").trim_start();
    }
    let mut format = None;
    if let Some(f) = rest.strip_prefix(':') {
        let f = f.trim();
        if f.is_empty() {
            return None;
        }
        format = Some(f.to_owned());
    } else if !rest.is_empty() {
        return None;
    }
    if TEXT.contains(&name.as_str()) && (offset.is_some() || format.is_some()) {
        return None;
    }
    if PROMPTS.contains(&name.as_str()) {
        if offset.is_some() {
            return None;
        }
        format = Some(format.unwrap_or_else(|| DEFAULT_PROMPT.to_owned()));
    }
    if name == NUMBER
        && (offset.is_some()
            || format
                .as_ref()
                .is_some_and(|f| !f.chars().all(|c| c == '0')))
    {
        return None;
    }
    Some(Token {
        name,
        offset,
        format,
    })
}

/// Each `{{…}}` in `text` with no brace inside (`/\{\{([^{}]*)\}\}/g`):
/// its start, its end, and what is inside.
// Byte offsets inside one text: none can overflow.
#[allow(clippy::arithmetic_side_effects)]
fn tokens(text: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    let mut from = 0;
    std::iter::from_fn(move || {
        while let Some(found) = text.get(from..).and_then(|r| r.find("{{")) {
            let start = from + found;
            let inner_from = start + 2;
            let rest = text.get(inner_from..).unwrap_or("");
            let len = rest.find(['{', '}']).unwrap_or(rest.len());
            if rest.get(len..).is_some_and(|r| r.starts_with("}}")) {
                let end = inner_from + len + 2;
                from = end;
                return Some((start, end, rest.get(..len).unwrap_or("")));
            }
            from = start + 1;
        }
        None
    })
}

/// The labels of a template's prompts, in order, once each
/// (`collectTemplatePrompts`).
#[must_use]
pub fn prompts(template: &str) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for (_, _, inner) in tokens(template) {
        let body = inner.trim();
        if body.starts_with('!') {
            continue;
        }
        if let Some(t) = parse_body(body)
            && PROMPTS.contains(&t.name.as_str())
            && let Some(label) = t.format
            && !labels.contains(&label)
        {
            labels.push(label);
        }
    }
    labels
}

/// A Moment format and a day offset from Templater's arguments
/// (`"YYYY-MM-DD", 7`): `None` when they are not plain ones.
fn tp_args(args: &str) -> Option<(Option<String>, i64)> {
    let args = args.trim();
    if args.is_empty() {
        return Some((None, 0));
    }
    let mut parts = args.splitn(2, ',');
    let first = parts.next()?.trim();
    let format = if first.is_empty() || first == "undefined" {
        None
    } else {
        Some(
            first
                .strip_prefix(['"', '\''])
                .and_then(|f| f.strip_suffix(['"', '\'']))
                .filter(|f| !f.contains(['{', '}']))?
                .to_owned(),
        )
    };
    let offset = match parts.next().map(str::trim) {
        None | Some("") => 0,
        Some(n) => n.parse().ok()?,
    };
    Some((format, offset))
}

/// A date token for an offset (`today+7d`) and a format.
fn tp_date(base: &str, format: Option<&str>, days: i64) -> String {
    let off = match days {
        0 => String::new(),
        d if d > 0 => format!("+{d}d"),
        d => format!("{d}d"),
    };
    let fmt = format
        .map(|f| format!(":{f}"))
        .unwrap_or_else(|| ":YYYY-MM-DD".to_owned());
    format!("{{{{{base}{off}{fmt}}}}}")
}

/// One Templater command's token, when it is one this reads.
fn tp_command(cmd: &str) -> Option<String> {
    let c = cmd.trim();
    let call = |name: &str| -> Option<&str> {
        c.strip_prefix(name)?
            .trim()
            .strip_prefix('(')?
            .strip_suffix(')')
    };
    Some(match c {
        "tp.file.title" => "{{title}}".to_owned(),
        _ if call("tp.file.folder").is_some() => {
            if call("tp.file.folder").is_some_and(|a| a.trim() == "true") {
                "{{path}}".to_owned()
            } else {
                "{{folder}}".to_owned()
            }
        }
        _ if call("tp.file.path").is_some() => "{{path}}".to_owned(),
        _ if call("tp.file.cursor").is_some() => "{{cursor}}".to_owned(),
        _ => {
            if let Some(a) = call("tp.date.now") {
                let (f, d) = tp_args(a)?;
                tp_date("today", f.as_deref(), d)
            } else if let Some(a) = call("tp.date.tomorrow") {
                tp_date("today", tp_args(a)?.0.as_deref(), 1)
            } else if let Some(a) = call("tp.date.yesterday") {
                tp_date("today", tp_args(a)?.0.as_deref(), -1)
            } else if let Some(a) = call("tp.date.weekday") {
                // Days from this week's Monday.
                let (f, d) = tp_args(a)?;
                tp_date("monday", f.as_deref(), d)
            } else if let Some(a) =
                call("tp.file.creation_date").or_else(|| call("tp.file.last_modified_date"))
            {
                let (f, _) = tp_args(a)?;
                format!(
                    "{{{{now:{}}}}}",
                    f.unwrap_or_else(|| "YYYY-MM-DD HH:mm".to_owned())
                )
            } else if let Some(a) = call("tp.system.prompt") {
                let label = a.split(',').next()?.trim().trim_matches(['"', '\'']);
                format!("{{{{prompt:{label}}}}}")
            } else {
                return None;
            }
        }
    })
}

/// Templater's common commands (`<% tp.… %>`) as this engine's tokens
/// (`templateEngine` automatic): `tp.file.title`, `folder`, `path`,
/// `cursor`; `tp.date.now`, `tomorrow`, `yesterday`, `weekday` with a
/// format and an offset in days; `tp.file.creation_date`;
/// `tp.system.prompt`. Scripts (`<%* … %>`) and commands this does not
/// read are left as written.
#[must_use]
pub fn templater(template: &str) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(at) = rest.find("<%") {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        let Some(end) = after.find("%>") else {
            out.push_str(after);
            return out;
        };
        let whole = &after[..end + 2];
        let inner = whole.trim_start_matches("<%").trim_end_matches("%>");
        let inner = inner
            .trim_start_matches(['-', '_'])
            .trim_end_matches(['-', '_']);
        match (inner.starts_with('*'), tp_command(inner)) {
            (false, Some(token)) => out.push_str(&token),
            _ => out.push_str(whole),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Whether a template has Templater's commands (`<%`), which architect
/// does not run.
#[must_use]
pub fn has_templater_commands(template: &str) -> bool {
    template.contains("<%")
}

fn add_months(at: NaiveDateTime, months: i64) -> Option<NaiveDateTime> {
    let m = Months::new(u32::try_from(months.unsigned_abs()).ok()?);
    if months < 0 {
        at.checked_sub_months(m)
    } else {
        at.checked_add_months(m)
    }
}

fn add(at: NaiveDateTime, amount: i64, unit: Unit) -> Option<NaiveDateTime> {
    match unit {
        Unit::Year => add_months(at, amount.checked_mul(12)?),
        Unit::Quarter => add_months(at, amount.checked_mul(3)?),
        Unit::Month => add_months(at, amount),
        Unit::Week => at.checked_add_signed(Duration::try_weeks(amount)?),
        Unit::Day => at.checked_add_signed(Duration::try_days(amount)?),
        Unit::Hour => at.checked_add_signed(Duration::try_hours(amount)?),
        Unit::Minute => at.checked_add_signed(Duration::try_minutes(amount)?),
        Unit::Second => at.checked_add_signed(Duration::try_seconds(amount)?),
    }
}

/// A date or clock token written; `None` when the offset leaves the
/// dates there are.
fn date_token(t: &Token, cx: &Context<'_>) -> Option<String> {
    let note_day = cx.date.unwrap_or_else(|| cx.now.date());
    let midnight = |d: NaiveDate| d.and_time(NaiveTime::MIN);
    let (mut base, default) = match t.name.as_str() {
        "time" => (cx.now, cx.time_format),
        "today" => (cx.now, cx.today_format),
        "now" => (cx.now, ISO_FORMAT),
        "date" => (midnight(note_day), cx.date_format),
        "yesterday" => (midnight(note_day.pred_opt()?), cx.date_format),
        "tomorrow" => (midnight(note_day.succ_opt()?), cx.date_format),
        day => {
            let want = u32::try_from(WEEKDAYS.iter().position(|w| *w == day)?).ok()?;
            let start = match cx.weekday_base {
                WeekdayBase::NoteDate => note_day,
                WeekdayBase::LocaleWeek => cx.weeks.week_start(note_day),
            };
            let d = (0..7)
                .filter_map(|i| start.checked_add_signed(Duration::days(i)))
                .find(|d| d.weekday().num_days_from_sunday() == want)
                .unwrap_or(start);
            (midnight(d), cx.today_format)
        }
    };
    let clock = matches!(t.name.as_str(), "time" | "today" | "now");
    if clock {
        if let Some((n, u)) = t.offset {
            base = add(base, n, u)?;
        }
    } else {
        // Days move first, then the clock's time goes on, so a time that
        // is not on the note's day (a DST gap) cannot move the day; the
        // clock's units after.
        if let Some((n, u)) = t.offset.filter(|(_, u)| u.calendar()) {
            base = add(base, n, u)?;
        }
        base = base.date().and_time(cx.now.time());
        if let Some((n, u)) = t.offset.filter(|(_, u)| !u.calendar()) {
            base = add(base, n, u)?;
        }
    }
    let format = t.format.as_deref().unwrap_or(default);
    Some(Pattern::new(format).format_at(base, cx.offset, cx.weeks))
}

/// The quoted YAML scalars of the template's frontmatter, as byte ranges
/// inside the quotes (`getQuotedFrontmatterRanges`): what a token writes in
/// one is escaped for its quote. Tokens are opaque, so a quote in a label
/// or a format does not end one.
// Byte offsets inside one text: none can overflow.
#[allow(clippy::arithmetic_side_effects)]
fn quoted_ranges(template: &str) -> Vec<(usize, usize, char)> {
    let body_from = match template
        .strip_prefix('\u{feff}')
        .map_or(template, |t| t)
        .find('\n')
    {
        Some(nl) => {
            let first = template.get(..nl).unwrap_or("");
            let first = first.trim_start_matches('\u{feff}').trim_end_matches('\r');
            if first.trim_end_matches([' ', '\t']) != "---" {
                return Vec::new();
            }
            nl + 1
        }
        None => return Vec::new(),
    };
    // The closing `---` or `...` line.
    let mut close = None;
    let mut at = body_from;
    for line in template
        .get(body_from..)
        .unwrap_or("")
        .split_inclusive('\n')
    {
        let l = line
            .trim_end_matches(['\n', '\r'])
            .trim_end_matches([' ', '\t']);
        if l == "---" || l == "..." {
            close = Some(at);
            break;
        }
        at += line.len();
    }
    let Some(close) = close else {
        return Vec::new();
    };
    let bytes = template.as_bytes();
    let token_end = |i: usize| -> Option<usize> {
        let rest = template.get(i..close)?;
        let rest = rest.strip_prefix("{{")?;
        let end = rest.find("}}")?;
        let inner = rest.get(..end)?;
        (!inner.contains('{') && !inner.contains('}')).then_some(i + 2 + end + 2)
    };
    let ws = |i: usize| bytes.get(i).is_none_or(u8::is_ascii_whitespace);
    let mut ranges = Vec::new();
    let mut quote: Option<char> = None;
    let mut quote_start = 0;
    let mut line_start = body_from;
    let mut block_indent: Option<usize> = None;
    let mut plain_indent: Option<usize> = None;
    let mut node_indent = 0;
    let mut value_indent = 0;
    let mut flow = 0usize;
    let mut scalar_start = true;
    let mut after_quoted = false;
    let mut i = body_from;
    while i < close {
        if i == line_start {
            let line_end = template
                .get(i..)
                .and_then(|r| r.find('\n'))
                .map_or(template.len(), |n| i + n);
            let mut content = i;
            while matches!(bytes.get(content), Some(b' ' | b'\t')) {
                content += 1;
            }
            let indent = content - line_start;
            let cont = block_indent.or(if flow == 0 { plain_indent } else { None });
            if quote.is_none()
                && let Some(c) = cont
            {
                if indent > c
                    || template
                        .get(content..line_end)
                        .unwrap_or("")
                        .trim()
                        .is_empty()
                {
                    i = line_end;
                    line_start = line_end + 1;
                    i += 1;
                    continue;
                }
                block_indent = None;
                plain_indent = None;
            }
            value_indent = indent;
        }
        let c = bytes.get(i).copied().unwrap_or(0);
        if c == b'{'
            && let Some(end) = token_end(i)
        {
            if quote.is_none() && scalar_start {
                node_indent = i - line_start;
                plain_indent = Some(value_indent);
            }
            i = end;
            scalar_start = false;
            continue;
        }
        if let Some(q) = quote {
            if (q == '"' && c == b'\\')
                || (q == '\'' && c == b'\'' && bytes.get(i + 1) == Some(&b'\''))
            {
                i += 2;
                continue;
            }
            if u32::from(c) == u32::from(q) {
                ranges.push((quote_start, i, q));
                quote = None;
                after_quoted = true;
            }
            i += 1;
            continue;
        }
        if c == b'\n' {
            line_start = i + 1;
            scalar_start = flow == 0 || scalar_start;
            after_quoted = false;
        } else if c == b'#' && i > 0 && ws(i - 1) {
            let nl = template
                .get(i..)
                .and_then(|r| r.find('\n'))
                .map_or(close, |n| i + n);
            i = nl;
            line_start = nl + 1;
            scalar_start = flow == 0 || scalar_start;
        } else if scalar_start && (c == b'"' || c == b'\'') {
            node_indent = i - line_start;
            quote = Some(char::from(c));
            quote_start = i + 1;
            scalar_start = false;
        } else if scalar_start && flow == 0 && (c == b'|' || c == b'>') {
            block_indent = Some(value_indent);
            let nl = template
                .get(i..)
                .and_then(|r| r.find('\n'))
                .map_or(close, |n| i + n);
            i = nl;
            line_start = nl + 1;
        } else if scalar_start && (c == b'[' || c == b'{') {
            flow += 1;
        } else if flow > 0 && (c == b']' || c == b'}') {
            flow -= 1;
            scalar_start = false;
        } else if c == b':' && (after_quoted || ws(i + 1)) {
            value_indent = node_indent;
            plain_indent = None;
            scalar_start = true;
            after_quoted = false;
        } else if (flow > 0 && c == b',') || (scalar_start && (c == b'-' || c == b'?') && ws(i + 1))
        {
            scalar_start = true;
            after_quoted = false;
        } else if !c.is_ascii_whitespace() {
            if scalar_start {
                node_indent = i - line_start;
                plain_indent = Some(value_indent);
            }
            scalar_start = false;
            after_quoted = false;
        }
        i += 1;
    }
    ranges
}

/// `template` written for `cx` (`renderNoteTemplate`).
///
/// Unknown `{{…}}` is copied; known tokens that do not read or write are
/// copied and listed in [`Rendered::invalid`]. `{{cursor}}` goes, and the
/// first one's place is kept; `{{number}}` writes the number, or goes and
/// is kept as a slot. In a quoted frontmatter scalar what a token writes
/// is escaped for the quote.
#[must_use]
// Byte offsets inside one text: none can overflow.
#[allow(clippy::arithmetic_side_effects)]
pub fn render(template: &str, cx: &Context<'_>) -> Rendered {
    let quoted = quoted_ranges(template);
    let mut out = String::new();
    let mut r = Rendered::default();
    let mut cursor_at: Option<usize> = None;
    let mut last = 0;
    let value = |out: &mut String, v: &str, at: usize| match quoted
        .iter()
        .find(|(s, e, _)| at >= *s && at <= *e)
    {
        Some((_, _, '"')) => {
            for ch in v.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if c.is_control() => {
                        let _ = write!(out, "\\u{:04x}", u32::from(c));
                    }
                    c => out.push(c),
                }
            }
        }
        Some(_) => out.push_str(&v.replace('\'', "''")),
        None => out.push_str(v),
    };
    for (start, end, inner) in tokens(template) {
        out.push_str(template.get(last..start).unwrap_or(""));
        last = end;
        let raw = template.get(start..end).unwrap_or("");
        let body = inner.trim();
        if let Some(lit) = body.strip_prefix('!') {
            out.push_str("{{");
            out.push_str(lit);
            out.push_str("}}");
            continue;
        }
        let name = name_of(body);
        if !known(&name) {
            out.push_str(raw);
            continue;
        }
        let Some(t) = parse_body(body) else {
            r.invalid.push(raw.to_owned());
            out.push_str(raw);
            continue;
        };
        match t.name.as_str() {
            "title" => value(&mut out, cx.title, start),
            "folder" => value(&mut out, cx.folder, start),
            "path" => value(&mut out, cx.path, start),
            "cursor" => {
                cursor_at.get_or_insert(out.len());
            }
            "prompt" | "value" => {
                if let Some(v) = t.format.as_ref().and_then(|l| cx.prompts.get(l)) {
                    value(&mut out, v, start);
                } else {
                    r.invalid.push(raw.to_owned());
                    out.push_str(raw);
                }
            }
            NUMBER => {
                let padding = t.format.as_ref().map_or(1, String::len);
                match cx.number {
                    Number::Slot => r.number_slots.push(NumberSlot {
                        offset: out.len(),
                        padding,
                    }),
                    Number::Value(n) => value(&mut out, &format!("{n:0padding$}"), start),
                    Number::None => {
                        r.invalid.push(raw.to_owned());
                        out.push_str(raw);
                    }
                }
            }
            _ => {
                if let Some(v) = date_token(&t, cx) {
                    value(&mut out, &v, start);
                } else {
                    r.invalid.push(raw.to_owned());
                    out.push_str(raw);
                }
            }
        }
    }
    out.push_str(template.get(last..).unwrap_or(""));
    r.cursor = cursor_at.map(|at| {
        let before = out.get(..at).unwrap_or("");
        let line = before.matches('\n').count();
        let ch = before.rsplit('\n').next().unwrap_or("").chars().count();
        (line, ch)
    });
    r.content = out;
    r
}

// ── Names ──────────────────────────────────────────────────────────────

/// One part of a numbered name: text, or a `{{number}}`'s place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamePart {
    Text(String),
    /// Its padding (`{{number:000}}` is 3).
    Number(usize),
}

/// A name with the characters a note's name cannot have taken out:
/// control characters, `: / \ < > " | ? *`, runs of white space as one.
fn clean_name_text(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in text.chars() {
        if c.is_control() || matches!(c, ':' | '/' | '\\' | '<' | '>' | '"' | '|' | '?' | '*') {
            continue;
        }
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    if space {
        out.push(' ');
    }
    out
}

/// A name format written ([`Number::Slot`]) cut at its numbers and
/// cleaned into a note's name (`sanitizeNumberedBaseName`): the ends
/// trimmed and trailing periods gone; empty parts dropped.
#[must_use]
// Byte offsets inside one text: none can overflow.
#[allow(clippy::arithmetic_side_effects)]
pub fn numbered_name(content: &str, slots: &[NumberSlot]) -> Vec<NamePart> {
    let mut texts: Vec<String> = Vec::new();
    let mut from = 0;
    for s in slots {
        texts.push(clean_name_text(content.get(from..s.offset).unwrap_or("")));
        from = s.offset;
    }
    texts.push(clean_name_text(content.get(from..).unwrap_or("")));
    if let Some(first) = texts.first_mut() {
        *first = first.trim_start().to_owned();
    }
    if let Some(last) = texts.last_mut() {
        *last = last.trim_end().trim_end_matches('.').trim_end().to_owned();
    }
    let mut parts = Vec::new();
    for (i, t) in texts.into_iter().enumerate() {
        if !t.is_empty() {
            parts.push(NamePart::Text(t));
        }
        if let Some(s) = slots.get(i) {
            parts.push(NamePart::Number(s.padding));
        }
    }
    parts
}

/// The number a numbered name takes in `folder` (`allocateNoteNumber`).
///
/// One more than the highest a note directly in it has in each place, in
/// any case and any width (`Note 7`, `note 007`); 1 when none. `None` for
/// a name with no number.
#[must_use]
pub fn next_number<'a>(
    folder: &str,
    name: &[NamePart],
    paths: impl IntoIterator<Item = &'a str>,
) -> Option<u64> {
    if !name.iter().any(|p| matches!(p, NamePart::Number(_))) {
        return None;
    }
    let mut highest = 0u64;
    for path in paths {
        let Some(stem) = path.strip_suffix(".md") else {
            continue;
        };
        let (dir, base) = stem.rsplit_once('/').unwrap_or(("", stem));
        if !dir.eq_ignore_ascii_case(folder) {
            continue;
        }
        if let Some(n) = match_numbered(&base.to_lowercase(), name) {
            highest = highest.max(n);
        }
    }
    highest.checked_add(1)
}

/// The number `base` (lower case) has in each of `name`'s places, when
/// they are the same.
// Byte offsets inside one text: none can overflow.
#[allow(clippy::arithmetic_side_effects)]
fn match_numbered(base: &str, name: &[NamePart]) -> Option<u64> {
    // The number once the parts are read; none when they do not read
    // or the name has no number.
    fn go(rest: &str, parts: &[NamePart], n: Option<u64>) -> Option<u64> {
        let Some((first, more)) = parts.split_first() else {
            return if rest.is_empty() { n } else { None };
        };
        match first {
            NamePart::Text(t) => go(rest.strip_prefix(t.to_lowercase().as_str())?, more, n),
            NamePart::Number(_) => {
                let digits = rest
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len());
                // Shorter runs too: the text after may start with a digit.
                (1..=digits).rev().find_map(|len| {
                    let v: u64 = rest.get(..len)?.parse().ok()?;
                    if v >= (1 << 53) || n.is_some_and(|old| old != v) {
                        return None;
                    }
                    go(rest.get(len..)?, more, Some(v))
                })
            }
        }
    }
    go(base, name, None)
}

/// A numbered name with `number` in its places, padded.
#[must_use]
pub fn format_numbered(name: &[NamePart], number: Option<u64>) -> String {
    name.iter()
        .map(|p| match (p, number) {
            (NamePart::Text(t), _) => t.clone(),
            (NamePart::Number(w), Some(n)) => format!("{n:0w$}"),
            (NamePart::Number(_), None) => String::new(),
        })
        .collect()
}

/// `name` in `folder` with `ext`, or `name 1`, `name 2`, … the first that
/// `taken` says is free (`generateUniqueFilename`). The name, no
/// extension.
#[must_use]
pub fn unique_name(folder: &str, name: &str, ext: &str, taken: impl Fn(&str) -> bool) -> String {
    (0..=u32::MAX)
        .map(|n| {
            if n == 0 {
                name.to_owned()
            } else {
                format!("{name} {n}")
            }
        })
        .find(|candidate| !taken(&in_folder(folder, candidate, ext)))
        .unwrap_or_else(|| name.to_owned())
}

/// `name.ext` in `folder` (the root when empty).
#[must_use]
pub fn in_folder(folder: &str, name: &str, ext: &str) -> String {
    let file = if ext.is_empty() {
        name.to_owned()
    } else {
        format!("{name}.{ext}")
    };
    if folder.is_empty() {
        file
    } else {
        format!("{folder}/{file}")
    }
}

// ── Folder templates and template commands ──────────────────────────────

/// A folder's template, set from its menu (`FolderTemplateMapping`).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct FolderTemplate {
    /// The folder; empty is the vault's root.
    pub folder: String,
    /// The template's vault path.
    pub template: String,
    /// Notes made in its subfolders take it too, unless a nearer folder
    /// has its own.
    pub subfolders: bool,
}

/// The template notes made in `folder` take (`getFolderTemplatePath`): its
/// own, else the nearest folder above's that includes subfolders.
#[must_use]
pub fn folder_template<'a>(list: &'a [FolderTemplate], folder: &str) -> Option<&'a str> {
    let mut at = folder.trim_matches('/').to_owned();
    let mut own = true;
    loop {
        if let Some(t) = list
            .iter()
            .find(|t| t.folder.trim_matches('/') == at && !t.template.trim().is_empty())
            .filter(|t| own || t.subfolders)
        {
            return Some(t.template.as_str());
        }
        if at.is_empty() {
            return None;
        }
        at = at
            .rsplit_once('/')
            .map_or(String::new(), |(p, _)| p.to_owned());
        own = false;
    }
}

/// Where a template command makes its note (`TemplateCommandLocation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum CommandLocation {
    /// The folder selected, else the open note's, else the root.
    #[default]
    Current,
    /// [`TemplateCommand::folder`].
    Folder,
}

/// A "create note" command a person made (`TemplateCommand`): a name, a
/// template, a file-name format and where. Its id is stable, so its
/// action (`NAVIGATOR_TEMPLATE_COMMAND`'s `id`) survives a rename.
#[derive(Debug, Clone, PartialEq, Eq, Facet, Default)]
pub struct TemplateCommand {
    pub id: String,
    pub name: String,
    /// The template's vault path; empty makes an empty note.
    pub template: String,
    /// The note's name, no extension, with tokens; empty is `Untitled`.
    pub name_format: String,
    pub location: CommandLocation,
    pub folder: String,
    /// Its button's icon (`templateCommands[].icon`): a Lucide name, an
    /// emoji, … as an item's icon is written; `file-plus` when empty.
    #[facet(default)]
    pub icon: String,
    /// Where its button is (`templateCommands[].placement`): none, or the
    /// navigator's header (NN's tab bar).
    #[facet(default)]
    pub placement: CommandPlacement,
}

/// Where a "create note" command's button is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum CommandPlacement {
    /// Nowhere: the palette and the menus only
    #[default]
    None,
    /// The navigator's header
    Header,
}

/// What a person keeps for templates (Notebook Navigator's
/// `folderTemplates`, `templateCommands`): kept with the view's state.
#[derive(Debug, Clone, PartialEq, Eq, Facet, Default)]
pub struct Templates {
    pub folders: Vec<FolderTemplate>,
    pub commands: Vec<TemplateCommand>,
}

impl Templates {
    /// Sets (or with an empty `template`, clears) `folder`'s template.
    pub fn set_folder(&mut self, folder: &str, template: &str, subfolders: bool) {
        let folder = folder.trim_matches('/');
        self.folders
            .retain(|t| t.folder.trim_matches('/') != folder);
        if !template.trim().is_empty() {
            self.folders.push(FolderTemplate {
                folder: folder.to_owned(),
                template: template.trim().to_owned(),
                subfolders,
            });
        }
    }

    /// The template paths and the commands' folders follow a file or a
    /// folder moved (`renameTemplateReferences`); `to` none: it went.
    pub fn follow(&mut self, from: &str, to: Option<&str>) {
        let moved = |p: &str| -> Option<Option<String>> {
            if p == from {
                return Some(to.map(str::to_owned));
            }
            p.strip_prefix(from)
                .filter(|rest| rest.starts_with('/'))
                .map(|rest| to.map(|to| format!("{to}{rest}")))
        };
        self.folders.retain_mut(|t| {
            if let Some(m) = moved(&t.folder) {
                match m {
                    Some(new) => t.folder = new,
                    None => return false,
                }
            }
            if let Some(m) = moved(&t.template) {
                match m {
                    Some(new) => t.template = new,
                    None => return false,
                }
            }
            true
        });
        for c in &mut self.commands {
            if let Some(Some(new)) = moved(&c.template) {
                c.template = new;
            }
            if let Some(Some(new)) = moved(&c.folder) {
                c.folder = new;
            }
        }
    }

    /// The command `which`: its id, its name (any case), or its number
    /// from 1.
    #[must_use]
    pub fn command(&self, which: &str) -> Option<&TemplateCommand> {
        let which = which.trim();
        self.commands
            .iter()
            .find(|c| c.id == which)
            .or_else(|| {
                self.commands
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(which))
            })
            .or_else(|| {
                let n: usize = which.parse().ok()?;
                self.commands.get(n.checked_sub(1)?)
            })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn templater_s_common_commands_are_tokens() {
        assert_eq!(
            templater(
                "# <% tp.file.title %>\n<% tp.date.now(\"YYYY-MM-DD\") %> <% tp.date.now(\"ddd\", 7) %> <%- tp.date.yesterday() -%>\n<% tp.file.cursor() %> <% tp.system.prompt(\"Who\") %>"
            ),
            "# {{title}}\n{{today:YYYY-MM-DD}} {{today+7d:ddd}} {{today-1d:YYYY-MM-DD}}\n{{cursor}} {{prompt:Who}}"
        );
        // Scripts, and commands it does not read, stay as written.
        let script = "<%* tR += \"x\" %> <% tp.web.daily_quote() %>";
        assert_eq!(templater(script), script);
        assert_eq!(templater("no commands"), "no commands");
    }

    use super::*;

    fn cx(prompts: &BTreeMap<String, String>) -> Context<'_> {
        Context {
            title: "Plan",
            folder: "Projects",
            path: "Projects/Plan.md",
            date: None,
            date_format: "YYYY-MM-DD",
            today_format: "YYYY-MM-DD",
            time_format: "HH:mm",
            weekday_base: WeekdayBase::LocaleWeek,
            now: NaiveDate::from_ymd_opt(2026, 11, 5)
                .and_then(|d| d.and_hms_opt(14, 30, 5))
                .unwrap_or_default(),
            offset: 3600,
            weeks: WeekRules::ISO,
            prompts,
            number: Number::None,
        }
    }

    #[test]
    fn text_dates_and_the_clock() {
        let p = BTreeMap::new();
        let r = render(
            "# {{title}} in {{folder}} ({{path}})\n{{date}} {{yesterday}} {{tomorrow}} {{time}} {{now}}",
            &cx(&p),
        );
        assert_eq!(
            r.content,
            "# Plan in Projects (Projects/Plan.md)\n2026-11-05 2026-11-04 2026-11-06 14:30 2026-11-05T14:30:05+01:00"
        );
        assert!(r.invalid.is_empty());
    }

    #[test]
    fn offsets_formats_and_weekdays() {
        let p = BTreeMap::new();
        let r = render(
            "{{date+1M:MMMM D}} {{date-2d}} {{time+90m}} {{monday:ddd D}} {{sunday:ddd D}} {{date:YYYY-MM-DD HH:mm}}",
            &cx(&p),
        );
        assert_eq!(
            r.content,
            "December 5 2026-11-03 16:00 Mon 2 Sun 8 2026-11-05 14:30"
        );
    }

    #[test]
    fn unknown_tokens_stay_and_bad_known_ones_are_reported() {
        let p = BTreeMap::new();
        let r = render(
            "{{date_created}} {{title-x}} {{title:YYYY}} {{!date}}",
            &cx(&p),
        );
        assert_eq!(
            r.content,
            "{{date_created}} {{title-x}} {{title:YYYY}} {{date}}"
        );
        assert_eq!(r.invalid, vec!["{{title:YYYY}}".to_owned()]);
    }

    #[test]
    fn cursor_prompts_and_numbers() {
        let mut p = BTreeMap::new();
        p.insert("Client".to_owned(), "Acme".to_owned());
        let mut c = cx(&p);
        c.number = Number::Value(7);
        let r = render(
            "a\nb {{cursor}}c {{prompt:Client}} {{value}} {{number:000}}",
            &c,
        );
        assert_eq!(r.content, "a\nb c Acme {{value}} 007");
        assert_eq!(r.cursor, Some((1, 2)));
        assert_eq!(r.invalid, vec!["{{value}}".to_owned()]);
        assert_eq!(
            prompts("{{prompt:Client}} {{value}} {{prompt:Client}} {{!prompt:X}}"),
            vec!["Client".to_owned(), "Value".to_owned()]
        );
    }

    #[test]
    fn quoted_frontmatter_values_are_escaped() {
        let mut p = BTreeMap::new();
        p.insert("Q".to_owned(), "say \"hi\" it's".to_owned());
        let r = render(
            "---\na: \"{{prompt:Q}}\"\nb: '{{prompt:Q}}'\nc: {{prompt:Q}}\n---\n{{prompt:Q}}",
            &cx(&p),
        );
        assert_eq!(
            r.content,
            "---\na: \"say \\\"hi\\\" it's\"\nb: 'say \"hi\" it''s'\nc: say \"hi\" it's\n---\nsay \"hi\" it's"
        );
    }

    #[test]
    fn numbered_names_count_up() {
        let p = BTreeMap::new();
        let mut c = cx(&p);
        c.number = Number::Slot;
        let r = render("Meeting {{number:00}}: {{date}}", &c);
        let name = numbered_name(&r.content, &r.number_slots);
        assert_eq!(
            name,
            vec![
                NamePart::Text("Meeting ".to_owned()),
                NamePart::Number(2),
                NamePart::Text(" 2026-11-05".to_owned()),
            ]
        );
        let paths = [
            "Meetings/meeting 07 2026-11-05.md",
            "Meetings/Meeting 3 2026-11-05.md",
            "Other/Meeting 9 2026-11-05.md",
        ];
        assert_eq!(next_number("Meetings", &name, paths), Some(8));
        assert_eq!(format_numbered(&name, Some(8)), "Meeting 08 2026-11-05");
        assert_eq!(next_number("", &[NamePart::Text("x".into())], paths), None);
    }

    #[test]
    fn folder_templates_are_inherited() {
        let mut t = Templates::default();
        t.set_folder("Projects", "Templates/Project.md", true);
        t.set_folder("Projects/Archive", "Templates/Old.md", false);
        assert_eq!(
            folder_template(&t.folders, "Projects/A/B"),
            Some("Templates/Project.md")
        );
        assert_eq!(
            folder_template(&t.folders, "Projects/Archive"),
            Some("Templates/Old.md")
        );
        assert_eq!(
            folder_template(&t.folders, "Projects/Archive/X"),
            Some("Templates/Project.md")
        );
        assert_eq!(folder_template(&t.folders, "Inbox"), None);
        t.follow("Templates", Some("Tpl"));
        assert_eq!(
            folder_template(&t.folders, "Projects"),
            Some("Tpl/Project.md")
        );
    }

    #[test]
    fn unique_names() {
        let taken = ["A/Note.md", "A/Note 1.md"];
        assert_eq!(
            unique_name("A", "Note", "md", |p| taken.contains(&p)),
            "Note 2"
        );
        assert_eq!(
            unique_name("", "Note", "md", |p| taken.contains(&p)),
            "Note"
        );
    }
}
