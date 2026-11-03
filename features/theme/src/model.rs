//! A theme: a small palette, a scheme, maybe a parent, and overrides —
//! what a theme file says ([`ThemeFile`]) and what the app keeps of it
//! ([`Theme`], in a [`Registry`]).

// The facet derive's generated code for data enums trips the arithmetic
// lint; the allowance stays in this module.
#![allow(clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::fmt;

use facet::Facet;

use crate::color::Rgba;

/// Whether a theme is drawn light (dark ink on a light ground) or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Facet)]
#[repr(u8)]
#[facet(rename_all = "lowercase")]
pub enum Scheme {
    Light,
    #[default]
    Dark,
}

impl Scheme {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    #[must_use]
    pub const fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

/// Which scheme the person wants: the system's, or one of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
#[facet(rename_all = "lowercase")]
pub enum Appearance {
    /// Follow the OS's light or dark ([`crate::ThemeStore::set_system`]).
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    /// The scheme it asks for, with the system's being `system`.
    #[must_use]
    pub const fn scheme(self, system: Scheme) -> Scheme {
        match self {
            Self::System => system,
            Self::Light => Scheme::Light,
            Self::Dark => Scheme::Dark,
        }
    }
}

/// Which theme shows: always one, or one for light and one for dark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeChoice {
    One(String),
    Pair { light: String, dark: String },
}

impl ThemeChoice {
    /// The pair `light` / `dark`.
    #[must_use]
    pub fn pair(light: impl Into<String>, dark: impl Into<String>) -> Self {
        Self::Pair {
            light: light.into(),
            dark: dark.into(),
        }
    }

    /// The theme it shows in `scheme`.
    #[must_use]
    pub fn pick(&self, scheme: Scheme) -> &str {
        match self {
            Self::One(id) => id,
            Self::Pair { light, dark } => match scheme {
                Scheme::Light => light,
                Scheme::Dark => dark,
            },
        }
    }

    /// This choice with `id` (a `scheme` theme) picked: in a pair it
    /// replaces only the half of its own lightness, so picking a dark
    /// theme keeps the light one. A single choice becomes a pair with the
    /// other half what it showed.
    #[must_use]
    pub fn with_pick(&self, id: &str, scheme: Scheme) -> Self {
        let (light, dark) = match self {
            Self::One(one) => (one.clone(), one.clone()),
            Self::Pair { light, dark } => (light.clone(), dark.clone()),
        };
        match scheme {
            Scheme::Light => Self::pair(id, dark),
            Scheme::Dark => Self::pair(light, id),
        }
    }
}

/// A theme file's palette: the few colours every token is made from.
///
/// Every one may be left out: a theme that `extends` another takes its
/// parent's, and anything still missing takes the built-in default's for
/// the scheme.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
#[facet(deny_unknown_fields)]
pub struct Palette {
    /// The ground under everything.
    #[facet(default)]
    pub background: Option<String>,
    /// The text.
    #[facet(default)]
    pub foreground: Option<String>,
    /// The brand colour: the primary button, the current tab, badges,
    /// the focus ring.
    #[facet(default)]
    pub accent: Option<String>,
    /// Cards and popovers, when they are not a step off the background.
    #[facet(default)]
    pub surface: Option<String>,
    /// Edges, when they are not a mix of the ground and the text.
    #[facet(default)]
    pub border: Option<String>,
    /// Good: a meter's green, a passing check.
    #[facet(default)]
    pub safe: Option<String>,
    /// Careful: a level near its ceiling, a warning.
    #[facet(default)]
    pub warn: Option<String>,
    /// Bad: clipping, a destructive action, an error.
    #[facet(default)]
    pub danger: Option<String>,
    /// News: a hint, a link, a note.
    #[facet(default)]
    pub info: Option<String>,
    /// Up to five series colours, in order (`chart-1` … `chart-5`).
    #[facet(default)]
    pub chart: Vec<String>,
}

impl Palette {
    /// Each colour with its field name, the set ones only.
    #[must_use]
    pub fn entries(&self) -> Vec<(String, &str)> {
        let named = [
            ("background", &self.background),
            ("foreground", &self.foreground),
            ("accent", &self.accent),
            ("surface", &self.surface),
            ("border", &self.border),
            ("safe", &self.safe),
            ("warn", &self.warn),
            ("danger", &self.danger),
            ("info", &self.info),
        ];
        named
            .into_iter()
            .filter_map(|(name, value)| value.as_deref().map(|v| (name.to_owned(), v)))
            .chain(
                self.chart
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (format!("chart[{i}]"), v.as_str())),
            )
            .collect()
    }

    /// `self` over `under`: each colour `self` sets wins.
    #[must_use]
    pub fn over(&self, under: &Self) -> Self {
        let pick = |a: &Option<String>, b: &Option<String>| a.clone().or_else(|| b.clone());
        Self {
            background: pick(&self.background, &under.background),
            foreground: pick(&self.foreground, &under.foreground),
            accent: pick(&self.accent, &under.accent),
            surface: pick(&self.surface, &under.surface),
            border: pick(&self.border, &under.border),
            safe: pick(&self.safe, &under.safe),
            warn: pick(&self.warn, &under.warn),
            danger: pick(&self.danger, &under.danger),
            info: pick(&self.info, &under.info),
            chart: if self.chart.is_empty() {
                under.chart.clone()
            } else {
                self.chart.clone()
            },
        }
    }
}

/// A theme file, `<themes dir>/<id>.json` (comments allowed): read
/// strictly, so a misspelt field is a problem rather than ignored.
///
/// ```json
/// {
///   "label": "Studio Night",
///   "appearance": "dark",
///   "extends": "fasttrackstudio-dark",
///   "palette": { "accent": "#f59e0b", "danger": "#ef4444" },
///   "overrides": { "radius": "0.375rem" }
/// }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
#[facet(deny_unknown_fields)]
pub struct ThemeFile {
    /// Its name in a picker; the id when left out.
    #[facet(default)]
    pub label: Option<String>,
    /// Light or dark. Left out, it is the parent's, or what the
    /// background reads as.
    #[facet(default)]
    pub appearance: Option<Scheme>,
    /// Another theme's id: its palette and overrides are this one's
    /// starting point.
    #[facet(default)]
    pub extends: Option<String>,
    #[facet(default)]
    pub palette: Palette,
    /// Any token by name (`"card"`, `"radius"`, `"ink-3"`, a component
    /// role an app adds): the last word, after everything is derived.
    #[facet(default)]
    pub overrides: BTreeMap<String, String>,
}

/// Where a theme came from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// Compiled into architect.
    Builtin,
    /// A file in a themes directory.
    File(String),
    /// A file an extension contributes (its id, the file).
    Extension { id: String, file: String },
}

impl Source {
    /// Its kind, as the actions name it.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::File(_) => "file",
            Self::Extension { .. } => "extension",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin => f.write_str("builtin"),
            Self::File(path) => f.write_str(path),
            Self::Extension { id, file } => write!(f, "{id}: {file}"),
        }
    }
}

/// Something wrong with a theme, said for a person: which file, which
/// field, what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The theme's id.
    pub theme: String,
    /// Its file (or `builtin`).
    pub source: String,
    /// The field, as a path (`palette.accent`, `overrides.ink-3`), when
    /// one is to blame.
    pub field: Option<String>,
    pub message: String,
}

impl Problem {
    #[must_use]
    pub fn new(theme: &str, source: &Source, message: impl Into<String>) -> Self {
        Self {
            theme: theme.to_owned(),
            source: source.to_string(),
            field: None,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn at(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)?;
        if let Some(field) = &self.field {
            write!(f, ": {field}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// A theme the app knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub id: String,
    pub label: String,
    pub file: ThemeFile,
    pub source: Source,
}

impl Theme {
    /// A theme from its file.
    #[must_use]
    pub fn new(id: impl Into<String>, file: ThemeFile, source: Source) -> Self {
        let id = id.into();
        let label = file.label.clone().unwrap_or_else(|| id.clone());
        Self {
            id,
            label,
            file,
            source,
        }
    }

    /// Reads a theme file's text strictly: unknown fields, values that
    /// are not colours where a colour goes, an id that is not a plain
    /// name are each a [`Problem`] naming the field.
    pub fn parse(id: &str, text: &str, source: Source) -> Result<Self, Vec<Problem>> {
        let mut problems = Vec::new();
        if !valid_id(id) {
            problems.push(
                Problem::new(
                    id,
                    &source,
                    "a theme id is lowercase letters, digits and `-`",
                )
                .at("id"),
            );
        }
        let file: ThemeFile = match facet_json::from_str_jsonc(text) {
            Ok(file) => file,
            Err(e) => {
                problems.push(parse_problem(id, &source, &e.to_string()));
                return Err(problems);
            }
        };
        for (field, value) in file.palette.entries() {
            if Rgba::parse(value).is_none() {
                problems.push(
                    Problem::new(id, &source, format!("{value:?} is not a colour"))
                        .at(format!("palette.{field}")),
                );
            }
        }
        if file.palette.chart.len() > 5 {
            problems
                .push(Problem::new(id, &source, "at most five chart colours").at("palette.chart"));
        }
        for (name, value) in &file.overrides {
            if !valid_token(name) {
                problems.push(
                    Problem::new(
                        id,
                        &source,
                        "a token name is lowercase letters, digits and `-`",
                    )
                    .at(format!("overrides.{name}")),
                );
            } else if value.trim().is_empty() {
                problems.push(Problem::new(id, &source, "empty").at(format!("overrides.{name}")));
            }
        }
        if file.extends.as_deref() == Some(id) {
            problems.push(Problem::new(id, &source, "a theme cannot extend itself").at("extends"));
        }
        if problems.is_empty() {
            Ok(Self::new(id, file, source))
        } else {
            Err(problems)
        }
    }
}

/// facet's message, with the field it names pulled out when it names one.
fn parse_problem(id: &str, source: &Source, message: &str) -> Problem {
    let field = message.split(['`', '\'', '"']).nth(1).filter(|f| {
        !f.is_empty()
            && f.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-[]".contains(c))
    });
    let problem = Problem::new(id, source, message.trim());
    match field {
        Some(f) => problem.at(f),
        None => problem,
    }
}

/// A theme id: lowercase letters, digits and `-`.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A token name: lowercase letters, digits and `-` (a CSS variable's
/// name without its `--`).
#[must_use]
pub fn valid_token(name: &str) -> bool {
    valid_id(name)
}

/// The themes the app knows, by id, and what was wrong reading them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    themes: BTreeMap<String, Theme>,
    problems: Vec<Problem>,
}

impl Registry {
    /// No themes. [`Self::builtin`] has architect's.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Adds `theme`, replacing one of the same id.
    pub fn insert(&mut self, theme: Theme) {
        self.themes.insert(theme.id.clone(), theme);
    }

    pub fn report(&mut self, problems: impl IntoIterator<Item = Problem>) {
        self.problems.extend(problems);
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Theme> {
        self.themes.get(id)
    }

    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.themes.contains_key(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Theme> {
        self.themes.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.themes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }

    /// Everything wrong: files that did not read, and parents that are
    /// missing or loop.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        let mut out = self.problems.clone();
        for theme in self.themes.values() {
            if let Err(p) = self.chain(&theme.id) {
                out.push(p);
            }
        }
        out
    }

    /// `id` and its ancestors, itself first.
    pub fn chain(&self, id: &str) -> Result<Vec<&Theme>, Problem> {
        let mut chain: Vec<&Theme> = Vec::new();
        let mut next = Some(id.to_owned());
        while let Some(at) = next.take() {
            let theme = self.themes.get(&at).ok_or_else(|| {
                let (child, source) = chain
                    .last()
                    .map_or((id, Source::Builtin), |t| (t.id.as_str(), t.source.clone()));
                let problem = Problem::new(child, &source, format!("no theme {at:?}"));
                if chain.is_empty() {
                    problem
                } else {
                    problem.at("extends")
                }
            })?;
            if chain.iter().any(|t| t.id == theme.id) {
                return Err(
                    Problem::new(id, &theme.source, "its `extends` loop back to it").at("extends"),
                );
            }
            next.clone_from(&theme.file.extends);
            // (`take` above left it empty.)
            chain.push(theme);
        }
        Ok(chain)
    }

    /// The scheme `id` is drawn in: its own, a parent's, or what its
    /// background reads as.
    #[must_use]
    pub fn scheme_of(&self, id: &str) -> Option<Scheme> {
        let chain = self.chain(id).ok()?;
        chain.iter().find_map(|t| t.file.appearance).or_else(|| {
            chain
                .iter()
                .find_map(|t| t.file.palette.background.as_deref())
                .and_then(Rgba::parse)
                .map(|bg| {
                    if bg.is_light() {
                        Scheme::Light
                    } else {
                        Scheme::Dark
                    }
                })
        })
    }
}
