//! The filter search: what the list's search box types, read and
//! matched against a note.
//!
//! Notebook Navigator's filter search (`src/utils/filterSearch.ts`
//! `tokenizeFilterSearchQuery`, `classifyRawTokens`, `parseFilterModeTokens`,
//! `parseFilterSearchTokens`, `getFileFilterSearchMatch`,
//! `findFilterSearchNameMatch`; `src/utils/filterSearchExpression.ts`
//! `buildTagExpression`, `evaluateTagExpression`, `propertyTokenMatches`;
//! `src/utils/filterSearchDate.ts`, here [`DateFilter`]):
//!
//! - A word matches a note whose name (its title) or one of its aliases
//!   holds it; every word must match, across the name and the aliases.
//! - `#tag` a note with the tag or one below it; `#` a note with any tag.
//! - `.key` a note with a key that starts with it; `.key=value` a note with
//!   that key whose value holds `value`; `.key=null` one with the key and
//!   no value (`.key="null"` is the text `null`).
//! - `@today`, `@yesterday`, `@last7d`, `@last30d`, `@thisweek`,
//!   `@thismonth`, `@2026`, `@2026-11`, `@2026-Q4`, `@2026-W45`,
//!   `@2026-11-05`, `@05/11/2026`, ranges `@2026-11-01..2026-11-07` (either
//!   end open); `@c:` and `@m:` for the created or modified time, else the
//!   one the list sorts by.
//! - `folder:name` a note in a folder whose name holds `name`;
//!   `folder:/a/b` one right in `a/b`; `folder:/` one at the root.
//! - `ext:pdf` a file with that extension; `has:task` a note with a task
//!   not done.
//! - `-` before any of them leaves out what it matches (`-#` the tagged
//!   notes); a term opened with `"` is text to find in names, as written
//!   (`".F"`, `-"#work"`).
//! - Text is matched in any case, Latin accents folded (`é` is `e`).
//! - `AND` and `OR` are operators only when the search is tags and
//!   properties alone (AND binding tighter, terms side by side ANDed): then
//!   it is an expression, as a ⌘/Ctrl-click writes ([`crate::Query`]).
//!   Anywhere else, and leading, trailing or doubled, they are words.

use unicode_normalization::UnicodeNormalization;

use crate::list::{self, Clock, value_parts};
use crate::period::{DateField, DateFilter, DayMonthOrder};
use crate::properties::value_display;
use crate::query::Join;
use crate::settings::NavigatorSettings;
use crate::source::{Millis, NoteFile};

/// A property clause (`PropertySearchToken`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyToken {
    /// `.key`: any key that starts with it.
    Key(String),
    /// `.key=null`: exactly `key`, with no value.
    Empty(String),
    /// `.key=value`: exactly `key`, a value that holds `value`.
    Value { key: String, value: String },
}

/// A folder clause (`FolderFilterToken`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderToken {
    /// `folder:/a/b`: right in `a/b` (`""` the root).
    Exact(String),
    /// `folder:name`: in a folder whose name holds `name`.
    Segment(String),
}

/// One operand of a tags-and-properties expression.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Operand {
    /// `#tag`, or `#` (any tag) for `None`.
    Tag(Option<String>),
    /// `-#tag`, or `-#` (no tag) for `None`.
    NotTag(Option<String>),
    Property(PropertyToken),
    NotProperty(PropertyToken),
}

/// A term, read.
#[derive(Debug, Clone, PartialEq)]
enum Term {
    Operator(Join),
    Operand(Operand),
    Name(String),
    NotName(String),
    Date(DateFilter),
    NotDate(DateFilter),
    Folder(FolderToken),
    NotFolder(FolderToken),
    Extension(String),
    NotExtension(String),
    Task,
    NotTask,
}

/// A raw term: its text, whether opened with a quote (and so text), and
/// whether a `-` before that quote leaves it out; whether a value's quote
/// opened right after `=`.
struct Raw {
    text: String,
    literal: bool,
    negated: bool,
    quoted_value: bool,
}

/// The search, read (`FilterSearchTokens`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilterSearch {
    /// As typed.
    text: String,
    /// A tags-and-properties expression: OR between runs of AND.
    expression: Option<Vec<(Join, Operand)>>,
    names: Vec<String>,
    not_names: Vec<String>,
    tags: Vec<String>,
    any_tag: bool,
    not_tags: Vec<String>,
    no_tag: bool,
    properties: Vec<PropertyToken>,
    not_properties: Vec<PropertyToken>,
    dates: Vec<DateFilter>,
    not_dates: Vec<DateFilter>,
    folders: Vec<FolderToken>,
    not_folders: Vec<FolderToken>,
    extensions: Vec<String>,
    not_extensions: Vec<String>,
    task: bool,
    not_task: bool,
}

/// What a note is matched with beyond the search: the fields its name is
/// read from, which date `@…` means, the time zone.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchContext<'a> {
    /// [`NoteFile::title`]'s fields.
    pub name_fields: &'a [&'a str],
    /// `@…` without `c:` or `m:` is the created time.
    pub by_created: bool,
    /// The time zone's offset from UTC.
    pub offset: Millis,
    /// The settings and clock a note's dates are read with, from its
    /// frontmatter when they say so (`list::created`, `list::modified`);
    /// `None`, its file's times.
    pub dates: Option<(&'a NavigatorSettings, Clock)>,
}

impl FilterSearch {
    /// `text` read, `today` the person's (for `@today` and the like),
    /// `order` how `@05/11/2026` reads.
    #[must_use]
    pub fn parse(text: &str, today: chrono::NaiveDate, order: DayMonthOrder) -> Self {
        let mut out = Self {
            text: text.to_owned(),
            ..Self::default()
        };
        let raws = tokenize(text);
        if raws.is_empty() {
            return out;
        }
        let (terms, tag_mode) = classify(&raws, today, order);
        if tag_mode && let Some(expression) = expression(&terms) {
            out.expression = Some(expression);
            return out;
        }
        for term in terms {
            match term {
                Term::Operator(j) => out.names.push(match j {
                    Join::And => "and".to_owned(),
                    Join::Or => "or".to_owned(),
                }),
                Term::Name(n) => out.names.push(n),
                Term::NotName(n) => out.not_names.push(n),
                Term::Operand(Operand::Tag(Some(t))) => out.tags.push(t),
                Term::Operand(Operand::Tag(None)) => out.any_tag = true,
                Term::Operand(Operand::NotTag(Some(t))) => out.not_tags.push(t),
                Term::Operand(Operand::NotTag(None)) => out.no_tag = true,
                Term::Operand(Operand::Property(p)) => out.properties.push(p),
                Term::Operand(Operand::NotProperty(p)) => out.not_properties.push(p),
                Term::Date(d) => out.dates.push(d),
                Term::NotDate(d) => out.not_dates.push(d),
                Term::Folder(f) => out.folders.push(f),
                Term::NotFolder(f) => out.not_folders.push(f),
                Term::Extension(e) => out.extensions.push(e),
                Term::NotExtension(e) => out.not_extensions.push(e),
                Term::Task => out.task = true,
                Term::NotTask => out.not_task = true,
            }
        }
        out
    }

    /// As typed.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether it narrows the list at all (`filterSearchHasActiveCriteria`).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.expression.as_ref().is_some_and(|e| !e.is_empty())
            || !self.names.is_empty()
            || !self.not_names.is_empty()
            || !self.tags.is_empty()
            || self.any_tag
            || !self.not_tags.is_empty()
            || self.no_tag
            || !self.properties.is_empty()
            || !self.not_properties.is_empty()
            || !self.dates.is_empty()
            || !self.not_dates.is_empty()
            || !self.folders.is_empty()
            || !self.not_folders.is_empty()
            || !self.extensions.is_empty()
            || !self.not_extensions.is_empty()
            || self.task
            || self.not_task
    }

    /// Whether it is a tags-and-properties expression (`mode: 'tag'`).
    #[must_use]
    pub const fn is_expression(&self) -> bool {
        self.expression.is_some()
    }

    /// The words matched against names (and the full text, with that
    /// provider).
    #[must_use]
    pub fn words(&self) -> &[String] {
        &self.names
    }

    /// A search with the words taken out: what still narrows the list
    /// when the words go to the full text.
    #[must_use]
    pub fn without_words(&self) -> Self {
        Self {
            names: Vec::new(),
            ..self.clone()
        }
    }

    /// Whether `f` matches.
    #[must_use]
    pub fn matches(&self, f: &NoteFile, cx: &SearchContext<'_>) -> bool {
        let unfinished = f.tasks.is_some_and(|t| t.unfinished > 0);
        if (self.not_task && unfinished) || (self.task && !unfinished) {
            return false;
        }
        let tags: Vec<String> = f.tags.iter().map(|t| fold_tag(t)).collect();
        if let Some(expression) = &self.expression {
            return evaluate(expression, &tags, f);
        }
        let name = fold(f.title(cx.name_fields));
        let aliases = aliases(f);
        let folder = f.folder();
        let ext = fold(f.extension());
        let has = |want: &str| name.contains(want) || aliases.iter().any(|a| a.contains(want));
        if self.not_names.iter().any(|n| has(n))
            || self.not_folders.iter().any(|t| folder_matches(folder, t))
            || self.not_extensions.contains(&ext)
            || (self.no_tag && !tags.is_empty())
            || (!self.no_tag
                && self
                    .not_tags
                    .iter()
                    .any(|t| tags.iter().any(|g| tag_matches(g, t))))
            || self.not_properties.iter().any(|p| property_matches(f, p))
        {
            return false;
        }
        if !names_covered(&name, &aliases, &self.names) {
            return false;
        }
        if !self.folders.iter().all(|t| folder_matches(folder, t))
            || !self.extensions.iter().all(|e| *e == ext)
            || !self.properties.iter().all(|p| property_matches(f, p))
        {
            return false;
        }
        if (self.any_tag || !self.tags.is_empty())
            && (tags.is_empty()
                || !self
                    .tags
                    .iter()
                    .all(|t| tags.iter().any(|g| tag_matches(g, t))))
        {
            return false;
        }
        // `fileMatchesDateFilterTokens`.
        let created = || cx.dates.map_or(f.created, |(s, c)| list::created(f, s, c));
        let modified = || {
            cx.dates
                .map_or(f.modified, |(s, c)| list::modified(f, s, c))
        };
        let at = |d: &DateFilter| match d.field {
            DateField::Created => created(),
            DateField::Default if cx.by_created => created(),
            DateField::Modified | DateField::Default => modified(),
        };
        self.dates.iter().all(|d| d.matches(at(d), cx.offset))
            && !self.not_dates.iter().any(|d| d.matches(at(d), cx.offset))
    }
}

/// How a tag stands in the search (`SearchMatchState`): taken in by a
/// `#tag` on it or above it, left out by a `-#tag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMatch {
    Include,
    Exclude,
}

impl FilterSearch {
    /// How tag `tag` stands in the search, for its row and its pills
    /// (`TagTreeItem`'s search classes); `None` when no term names it.
    #[must_use]
    pub fn tag_match(&self, tag: &str) -> Option<SearchMatch> {
        let tag = fold_tag(tag);
        let named = |want: &str| tag_matches(&tag, want);
        let (mut inc, mut exc) = (
            self.tags.iter().any(|t| named(t)),
            self.not_tags.iter().any(|t| named(t)),
        );
        for (_, o) in self.expression.iter().flatten() {
            match o {
                Operand::Tag(Some(t)) => inc |= named(t),
                Operand::NotTag(Some(t)) => exc |= named(t),
                _ => {}
            }
        }
        if exc {
            Some(SearchMatch::Exclude)
        } else if inc {
            Some(SearchMatch::Include)
        } else {
            None
        }
    }
}

/// Where `words` (folded, as [`FilterSearch::words`]) are in `text`: byte
/// ranges of `text`, in order, overlaps merged
/// (`getFoldedSearchHighlightRanges`). A folded-out accent belongs to the
/// letter before it.
#[must_use]
pub fn highlights(text: &str, words: &[String]) -> Vec<std::ops::Range<usize>> {
    // Each byte of the folded text, and the source character it came from.
    let mut folded = String::new();
    let mut from: Vec<std::ops::Range<usize>> = Vec::new();
    for (at, c) in text.char_indices() {
        let end = at.saturating_add(c.len_utf8());
        let piece = fold(&c.to_string());
        if piece.is_empty() {
            if let Some(last) = from.last_mut() {
                let start = last.start;
                for r in from.iter_mut().rev().take_while(|r| r.start == start) {
                    r.end = end;
                }
            }
            continue;
        }
        for _ in 0..piece.len() {
            from.push(at..end);
        }
        folded.push_str(&piece);
    }
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for w in words.iter().filter(|w| !w.is_empty()) {
        if !seen.insert(w.as_str()) {
            continue;
        }
        let mut at = 0;
        while let Some(i) = folded.get(at..).and_then(|r| r.find(w.as_str())) {
            let fs = at.saturating_add(i);
            let fe = fs.saturating_add(w.len());
            if let (Some(a), Some(b)) = (from.get(fs), from.get(fe.saturating_sub(1))) {
                ranges.push(a.start..b.end);
            }
            at = fe;
        }
    }
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// Text in any case, Latin accents folded (`foldSearchText`).
#[must_use]
pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .nfd()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .nfc()
        .collect()
}

fn fold_tag(tag: &str) -> String {
    fold(tag.trim().trim_start_matches('#').trim_matches('/'))
}

/// `tagMatchesToken`: the tag or one below it.
fn tag_matches(tag: &str, want: &str) -> bool {
    !tag.is_empty()
        && !want.is_empty()
        && (tag == want
            || tag
                .strip_prefix(want)
                .is_some_and(|rest| rest.starts_with('/')))
}

/// A note's aliases, folded (`aliases`, `alias`).
fn aliases(f: &NoteFile) -> Vec<String> {
    f.properties
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("aliases") || k.eq_ignore_ascii_case("alias"))
        .flat_map(|(_, v)| value_parts(v))
        .map(|a| fold(&a))
        .collect()
}

/// `findFilterSearchNameMatch`: every word in the name or in an alias.
fn names_covered(name: &str, aliases: &[String], words: &[String]) -> bool {
    words
        .iter()
        .all(|w| name.contains(w.as_str()) || aliases.iter().any(|a| a.contains(w.as_str())))
}

/// `folderMatchesTokenWithNormalizedPath`.
fn folder_matches(folder: &str, token: &FolderToken) -> bool {
    let folder = fold(folder.trim_matches('/'));
    match token {
        FolderToken::Exact(want) => folder == *want,
        FolderToken::Segment(want) => {
            !want.is_empty()
                && folder
                    .split('/')
                    .any(|s| !s.is_empty() && s.contains(want.as_str()))
        }
    }
}

/// `propertyTokenMatches`, over the note's keys and values, folded.
fn property_matches(f: &NoteFile, token: &PropertyToken) -> bool {
    let values = |key: &str| -> Option<Vec<String>> {
        f.properties
            .iter()
            .find(|(k, _)| fold(k.trim()) == key)
            .map(|(_, v)| {
                value_parts(v)
                    .iter()
                    .map(|p| fold(&value_display(p)))
                    .filter(|p| !p.is_empty())
                    .collect()
            })
    };
    match token {
        PropertyToken::Key(want) => f
            .properties
            .iter()
            .any(|(k, _)| fold(k.trim()).starts_with(want.as_str())),
        PropertyToken::Empty(key) => values(key).is_some_and(|v| v.is_empty()),
        PropertyToken::Value { key, value } => {
            values(key).is_some_and(|v| v.iter().any(|p| p.contains(value.as_str())))
        }
    }
}

/// `evaluateTagExpression`: OR between runs of AND.
fn evaluate(expression: &[(Join, Operand)], tags: &[String], f: &NoteFile) -> bool {
    let has = |want: &str| tags.iter().any(|t| tag_matches(t, want));
    let one = |o: &Operand| match o {
        Operand::Tag(Some(t)) => has(t),
        Operand::Tag(None) => !tags.is_empty(),
        Operand::NotTag(Some(t)) => !has(t),
        Operand::NotTag(None) => tags.is_empty(),
        Operand::Property(p) => property_matches(f, p),
        Operand::NotProperty(p) => !property_matches(f, p),
    };
    let mut any = false;
    let mut run = true;
    for (i, (join, o)) in expression.iter().enumerate() {
        if i > 0 && *join == Join::Or {
            any |= run;
            run = true;
        }
        run = run && one(o);
    }
    any || run
}

/// `buildTagExpression`: operands joined by operators (side by side, by
/// AND); `None` when an operator leads, trails or follows another.
fn expression(terms: &[Term]) -> Option<Vec<(Join, Operand)>> {
    let mut out: Vec<(Join, Operand)> = Vec::new();
    let mut pending: Option<Join> = None;
    for term in terms {
        match term {
            Term::Operator(j) => {
                if out.is_empty() || pending.is_some() {
                    return None;
                }
                pending = Some(*j);
            }
            Term::Operand(o) => out.push((pending.take().unwrap_or(Join::And), o.clone())),
            _ => return None,
        }
    }
    (pending.is_none() && !out.is_empty()).then_some(out)
}

/// `tokenizeFilterSearchQuery`: terms split on spaces outside quotes.
fn tokenize(query: &str) -> Vec<Raw> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut literal, mut negated, mut quoted_value, mut in_quotes) = (false, false, false, false);
    let mut chars = query.trim().chars().peekable();
    let mut push = |cur: &mut String, literal: &mut bool, negated: &mut bool, qv: &mut bool| {
        if !cur.is_empty() {
            out.push(Raw {
                text: std::mem::take(cur),
                literal: *literal,
                negated: *negated,
                quoted_value: *qv,
            });
        }
        (*literal, *negated, *qv) = (false, false, false);
    };
    while let Some(c) = chars.next() {
        if in_quotes && c == '\\' && matches!(chars.peek(), Some('"' | '\\')) {
            cur.extend(chars.next());
            continue;
        }
        if c == '"' {
            if !in_quotes && !literal {
                if cur.is_empty() {
                    literal = true;
                } else if cur == "-" {
                    literal = true;
                    negated = true;
                    cur.clear();
                } else if cur.ends_with('=') {
                    quoted_value = true;
                }
            }
            in_quotes = !in_quotes;
            continue;
        }
        if !in_quotes && c.is_whitespace() {
            push(&mut cur, &mut literal, &mut negated, &mut quoted_value);
            continue;
        }
        cur.push(c);
    }
    push(&mut cur, &mut literal, &mut negated, &mut quoted_value);
    out
}

/// `classifyRawTokens`; and whether the terms may be an expression
/// (`canUseTagMode`: a tag or property, nothing else, nothing invalid).
fn classify(raws: &[Raw], today: chrono::NaiveDate, order: DayMonthOrder) -> (Vec<Term>, bool) {
    let mut out = Vec::new();
    let (mut tag_operand, mut other, mut invalid) = (false, false, false);
    for raw in raws {
        if raw.text.is_empty() {
            continue;
        }
        if raw.literal {
            other = true;
            let n = fold(&raw.text);
            out.push(if raw.negated {
                Term::NotName(n)
            } else {
                Term::Name(n)
            });
            continue;
        }
        let lower = raw.text.to_lowercase();
        match lower.as_str() {
            "and" => {
                out.push(Term::Operator(Join::And));
                continue;
            }
            "or" => {
                out.push(Term::Operator(Join::Or));
                continue;
            }
            _ => {}
        }
        let (negate, body) = lower
            .strip_prefix('-')
            .map_or((false, lower.as_str()), |rest| (true, rest));
        if body.is_empty() {
            invalid = true;
            continue;
        }
        let pick = |yes: Term, no: Term| if negate { no } else { yes };
        if body == "has:task" || body == "has:tasks" {
            other = true;
            out.push(pick(Term::Task, Term::NotTask));
        } else if body.starts_with('@') {
            other = true;
            if DateFilter::is_candidate(body) {
                if let Some(d) = DateFilter::parse(body, today, order) {
                    out.push(pick(Term::Date(d), Term::NotDate(d)));
                }
            } else {
                let n = fold(body);
                out.push(pick(Term::Name(n.clone()), Term::NotName(n)));
            }
        } else if let Some(rest) = body.strip_prefix("folder:") {
            if let Some(t) = folder_token(rest) {
                other = true;
                out.push(pick(Term::Folder(t.clone()), Term::NotFolder(t)));
            }
        } else if let Some(rest) = body.strip_prefix("ext:") {
            if let Some(e) = extension_token(rest) {
                other = true;
                out.push(pick(Term::Extension(e.clone()), Term::NotExtension(e)));
            }
        } else if let Some(rest) = body.strip_prefix('.') {
            if let Some(p) = property_token(rest, raw.quoted_value) {
                tag_operand = true;
                out.push(Term::Operand(if negate {
                    Operand::NotProperty(p)
                } else {
                    Operand::Property(p)
                }));
            }
        } else if let Some(rest) = body.strip_prefix('#') {
            tag_operand = true;
            let t = (!rest.is_empty()).then(|| fold_tag(rest));
            out.push(Term::Operand(if negate {
                Operand::NotTag(t)
            } else {
                Operand::Tag(t)
            }));
        } else {
            other = true;
            let n = fold(body);
            out.push(pick(Term::Name(n.clone()), Term::NotName(n)));
        }
    }
    (out, tag_operand && !other && !invalid)
}

/// `normalizeFolderFilterToken`.
fn folder_token(value: &str) -> Option<FolderToken> {
    let mut v = value.trim().replace('\\', "/");
    while v.contains("//") {
        v = v.replace("//", "/");
    }
    if v.is_empty() {
        return None;
    }
    if v.starts_with('/') {
        return Some(FolderToken::Exact(fold(v.trim_matches('/'))));
    }
    let seg = v.trim_matches('/');
    (!seg.is_empty() && !seg.contains('/')).then(|| FolderToken::Segment(fold(seg)))
}

/// `parseExtensionFilterToken`: the last `.` part.
fn extension_token(value: &str) -> Option<String> {
    let v = value.trim().trim_start_matches('.');
    if v.is_empty() || v.contains(['/', '\\']) {
        return None;
    }
    v.split('.').rfind(|s| !s.is_empty()).map(fold)
}

/// `parsePropertyFilterToken`.
fn property_token(content: &str, quoted_value: bool) -> Option<PropertyToken> {
    if content.is_empty() {
        return None;
    }
    let Some(at) = separator(content) else {
        let key = fold(unescape(&unquote(content)).trim());
        return (!key.is_empty()).then_some(PropertyToken::Key(key));
    };
    let key = fold(unescape(&unquote(content.get(..at)?)).trim());
    if key.is_empty() {
        return None;
    }
    let raw_value = content.get(at.checked_add(1)?..)?;
    let unquoted = unquote(raw_value);
    let value = unescape(&unquoted);
    if !quoted_value && unquoted == raw_value && value.trim().eq_ignore_ascii_case("null") {
        return Some(PropertyToken::Empty(key));
    }
    let value = fold(value_display(&value).trim());
    Some(if value.is_empty() {
        PropertyToken::Key(key)
    } else {
        PropertyToken::Value { key, value }
    })
}

/// The first `=` outside quotes and not escaped.
fn separator(content: &str) -> Option<usize> {
    let (mut escaped, mut quoted) = (false, false);
    for (i, c) in content.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            '=' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

/// `tryUnquotePropertyFilterPart`.
fn unquote(value: &str) -> String {
    let t = value.trim();
    let Some(inner) = t
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .filter(|_| t.len() >= 2)
    else {
        return value.to_owned();
    };
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && matches!(chars.peek(), Some('"' | '\\')) {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
}

/// `unescapePropertyFilterPart`.
fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && matches!(chars.peek(), Some('=' | '\\' | '"')) {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
}

/// A tag or property written into `text`, or taken out when it is there
/// (`updateFilterQueryWithTag` on a search that is not tags and
/// properties alone): the term appended, or every word equal to it
/// removed.
#[must_use]
pub fn toggle_term(text: &str, term: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.iter().any(|w| w.eq_ignore_ascii_case(term)) {
        words
            .into_iter()
            .filter(|w| !w.eq_ignore_ascii_case(term))
            .collect::<Vec<_>>()
            .join(" ")
    } else if text.trim().is_empty() {
        term.to_owned()
    } else {
        format!("{} {term}", text.trim_end())
    }
}
