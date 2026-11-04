//! A theme resolved: every semantic token, derived from its palette, then
//! its overrides. One flat map ([`Resolved`]) every consumer reads.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::color::{BLACK, MAGENTA, Rgba, WHITE, contrast, with_contrast};
use crate::expr::Expr;
use crate::model::{Palette, Problem, Registry, Scheme, Theme};
use crate::store::LayerKind;

/// The token names architect derives, by who reads them. A name not here
/// still resolves when a theme's overrides set it (an app's own roles).
pub mod names {
    /// architect-ui's (`fts-theme.css`'s CSS variables, shadcn's names).
    pub const UI: &[&str] = &[
        "background",
        "foreground",
        "card",
        "card-foreground",
        "popover",
        "popover-foreground",
        "primary",
        "primary-foreground",
        "secondary",
        "secondary-foreground",
        "muted",
        "muted-foreground",
        "accent",
        "accent-foreground",
        "destructive",
        "destructive-foreground",
        "border",
        "input",
        "ring",
        "chart-1",
        "chart-2",
        "chart-3",
        "chart-4",
        "chart-5",
        "sidebar",
        "sidebar-foreground",
        "sidebar-primary",
        "sidebar-primary-foreground",
        "sidebar-accent",
        "sidebar-accent-foreground",
        "sidebar-border",
        "sidebar-ring",
    ];
    /// The signal colours (meters, automation, slots), architect-ui's too.
    pub const SIGNAL: &[&str] = &[
        "signal-safe",
        "signal-warn",
        "signal-danger",
        "signal-override",
        "signal-automation",
        "signal-mod",
        "signal-slot-a",
        "signal-slot-b",
        "signal-quick",
        "signal-full",
    ];
    /// The palette's status colours, by their own names.
    pub const STATUS: &[&str] = &["safe", "warn", "danger", "info"];
    /// The shell's (`ShellTheme`'s fields; its `accent` trio is `brand`
    /// here, since `accent` is architect-ui's hover wash).
    pub const SHELL: &[&str] = &[
        "desk",
        "sheet",
        "title-bar",
        "pill",
        "rule",
        "active",
        "ink",
        "ink-2",
        "ink-3",
        "brand",
        "brand-ink",
        "brand-soft",
        "danger",
        "scrim",
        "shadow-lift",
        "shadow-float",
        "font",
    ];
    /// Not colours: faces, radius, spacing, shadow parts.
    pub const SHAPE: &[&str] = &[
        "font-sans",
        "font-serif",
        "font-mono",
        "radius",
        "spacing",
        "letter-spacing",
        "shadow-color",
        "shadow-opacity",
        "shadow-blur",
        "shadow-spread",
        "shadow-offset-x",
        "shadow-offset-y",
    ];

    /// Every derived name, once.
    #[must_use]
    pub fn all() -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for name in [UI, SIGNAL, STATUS, SHELL, SHAPE].concat() {
            if !out.contains(&name) {
                out.push(name);
            }
        }
        out
    }
}

/// The shell's face: Inter where the app bundles it, the system's sans
/// elsewhere.
pub const SHELL_FONT: &str = "Inter, -apple-system, 'SF Pro Text', system-ui, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif";

/// What a palette leaves out, by scheme: `FastTrackStudio`'s.
#[must_use]
pub fn default_palette(scheme: Scheme) -> Palette {
    let s = |v: &str| Some(v.to_owned());
    let (background, foreground, accent, danger) = match scheme {
        Scheme::Dark => ("#09090b", "#e4e4e7", "#2dd4bf", "#f87171"),
        Scheme::Light => ("#fafafa", "#18181b", "#0f766e", "#dc2626"),
    };
    Palette {
        background: s(background),
        foreground: s(foreground),
        accent: s(accent),
        surface: None,
        border: None,
        safe: s("oklch(0.65 0.19 145)"),
        warn: s("oklch(0.75 0.16 85)"),
        danger: s(danger),
        info: s("oklch(0.65 0.15 250)"),
        chart: Vec::new(),
    }
}

/// `theme`, with what it extends from `registry`, as one flat map of
/// tokens.
///
/// The palette (its parents' under it, the scheme's default
/// under those) derived into every name in [`names`], then the
/// overrides, the parents' first, then every expression
/// (`crate::expr`) evaluated against the final value of what it names.
/// Deterministic: the same theme and registry give the same map. A
/// missing or looping parent is left out (it is one of
/// [`Registry::problems`]).
#[must_use]
pub fn resolve(theme: &Theme, registry: &Registry) -> Resolved {
    resolve_in(theme, registry, None, &BTreeMap::new(), &BTreeMap::new())
}

/// Who wrote a token's value, before it is evaluated.
#[derive(Clone, Copy)]
enum By<'a> {
    Derived,
    Theme(&'a Theme),
    Layer(LayerKind),
}

/// [`resolve`] under the cascade: `chosen_by` is the layer whose choice
/// shows `theme`, `overrides` every layer's (the higher over the lower)
/// and `set_by` the layer each came from. The layers' values go in after
/// the theme's, so a later literal wins, and only then is anything
/// evaluated: a reference sees the final value of what it names.
pub(crate) fn resolve_in(
    theme: &Theme,
    registry: &Registry,
    chosen_by: Option<LayerKind>,
    overrides: &BTreeMap<String, String>,
    set_by: &BTreeMap<String, LayerKind>,
) -> Resolved {
    let mut chain = vec![theme];
    let mut seen = BTreeSet::from([theme.id.as_str()]);
    let mut next = theme.file.extends.as_deref();
    while let Some(id) = next {
        let Some(parent) = registry.get(id).filter(|_| seen.insert(id)) else {
            break;
        };
        chain.push(parent);
        next = parent.file.extends.as_deref();
    }
    let scheme = chain
        .iter()
        .find_map(|t| t.file.appearance)
        .or_else(|| {
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
        .unwrap_or_default();
    // The root's palette first, each child's over it.
    let palette = chain
        .iter()
        .rev()
        .fold(default_palette(scheme), |under, t| {
            t.file.palette.over(&under)
        });
    let mut raw = derive(&palette, scheme);
    let mut by: BTreeMap<String, By<'_>> = raw.keys().map(|k| (k.clone(), By::Derived)).collect();
    let layer_of = |name: &str| set_by.get(name).copied().unwrap_or(LayerKind::Builtin);
    // The extension layer is extensions' defaults for their own tokens:
    // under the theme's overrides, so a theme file restyles them too.
    let (defaults, layered): (Vec<_>, Vec<_>) = overrides
        .iter()
        .partition(|(name, _)| layer_of(name) == LayerKind::Extension);
    for (name, value) in defaults {
        raw.insert(name.clone(), value.clone());
        by.insert(name.clone(), By::Layer(LayerKind::Extension));
    }
    for t in chain.iter().rev() {
        for (name, value) in &t.file.overrides {
            raw.insert(name.clone(), value.clone());
            by.insert(name.clone(), By::Theme(t));
        }
    }
    for (name, value) in layered {
        raw.insert(name.clone(), value.clone());
        by.insert(name.clone(), By::Layer(layer_of(name)));
    }

    let mut eval = Eval::new(&raw);
    let mut tokens = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut problems = Vec::new();
    for (name, value) in &raw {
        let who = by.get(name).copied().unwrap_or(By::Derived);
        let reference = crate::expr::is_expression(value);
        let final_value = if reference {
            eval.value_of(name).unwrap_or_else(|reason| {
                problems.push(problem(theme, name, who, reason));
                MAGENTA_CSS.to_owned()
            })
        } else {
            value.clone()
        };
        tokens.insert(name.clone(), final_value);
        sources.insert(
            name.clone(),
            TokenSource {
                origin: match who {
                    By::Derived => Origin::Derived,
                    By::Theme(_) => Origin::Theme,
                    By::Layer(_) => Origin::Layer,
                },
                layer: match who {
                    By::Layer(kind) => Some(kind),
                    By::Derived | By::Theme(_) => chosen_by,
                },
                id: match who {
                    By::Derived => theme.id.clone(),
                    By::Theme(t) => t.id.clone(),
                    By::Layer(kind) => kind.as_str().to_owned(),
                },
                expression: reference.then(|| value.trim().to_owned()),
            },
        );
    }
    Resolved::build(&theme.id, scheme, tokens, sources, problems)
}

/// A token whose expression does not resolve, said for a person.
fn problem(theme: &Theme, name: &str, who: By<'_>, reason: String) -> Problem {
    let field = format!("overrides.{name}");
    match who {
        By::Theme(t) => Problem::new(&t.id, &t.source, reason).at(field),
        By::Layer(kind) => Problem {
            theme: kind.as_str().to_owned(),
            source: format!("{} layer", kind.as_str()),
            field: Some(field),
            message: reason,
        },
        By::Derived => Problem::new(&theme.id, &theme.source, reason).at(field),
    }
}

/// What an expression that does not resolve shows: magenta.
const MAGENTA_CSS: &str = "#ff00ff";

/// Evaluates expressions against the raw map, each name once, finding
/// the cycles.
struct Eval<'a> {
    raw: &'a BTreeMap<String, String>,
    done: BTreeMap<&'a str, Result<String, String>>,
    /// The names being evaluated, outermost first.
    stack: Vec<&'a str>,
    /// Each name on a cycle, and the cycle.
    cycles: BTreeMap<&'a str, String>,
}

impl<'a> Eval<'a> {
    const fn new(raw: &'a BTreeMap<String, String>) -> Self {
        Self {
            raw,
            done: BTreeMap::new(),
            stack: Vec::new(),
            cycles: BTreeMap::new(),
        }
    }

    /// The final value of `name` (a name in the map).
    fn value_of(&mut self, name: &str) -> Result<String, String> {
        let Some((name, raw)) = self.raw.get_key_value(name) else {
            return Err(format!("`${name}` names no token"));
        };
        let name = name.as_str();
        if let Some(done) = self.done.get(name) {
            return done.clone();
        }
        if let Some(at) = self.stack.iter().position(|n| *n == name) {
            let looped = self.stack.get(at..).unwrap_or_default().to_vec();
            let path: Vec<String> = looped
                .iter()
                .chain([&name])
                .map(|n| format!("${n}"))
                .collect();
            let cycle = format!("a cycle: {}", path.join(" → "));
            for n in looped {
                self.cycles.entry(n).or_insert_with(|| cycle.clone());
            }
            return Err(cycle);
        }
        self.stack.push(name);
        let result = match crate::expr::parse(raw) {
            Ok(None) => Ok(raw.clone()),
            Ok(Some(expr)) => self.eval(&expr),
            Err(e) => Err(e),
        };
        self.stack.pop();
        let result = self.cycles.get(name).map_or(result, |c| Err(c.clone()));
        self.done.insert(name, result.clone());
        result
    }

    fn eval(&mut self, expr: &Expr) -> Result<String, String> {
        match expr {
            Expr::Lit(text) => Ok(text.clone()),
            Expr::Ref(name) => {
                if !self.raw.contains_key(name) {
                    return Err(format!("`${name}` names no token"));
                }
                self.value_of(name)
                    .map_err(|why| format!("`${name}` does not resolve ({why})"))
            }
            Expr::Mix(a, b, t) => {
                let (a, b, t) = (self.colour(a)?, self.colour(b)?, self.fraction(t)?);
                Ok(a.mix(b, t).to_css())
            }
            Expr::Alpha(a, t) => {
                let (a, t) = (self.colour(a)?, self.fraction(t)?);
                Ok(a.alpha(t).to_css())
            }
        }
    }

    fn colour(&mut self, expr: &Expr) -> Result<Rgba, String> {
        let value = self.eval(expr)?;
        Rgba::parse(&value).ok_or_else(|| format!("{value:?} is not a colour"))
    }

    fn fraction(&mut self, expr: &Expr) -> Result<f32, String> {
        let value = self.eval(expr)?;
        let n = crate::expr::number(&value).ok_or_else(|| format!("{value:?} is not a number"))?;
        if (0.0..=1.0).contains(&n) {
            Ok(n)
        } else {
            Err(format!("{value} is not between 0 and 1"))
        }
    }
}

/// Every derived token from a full palette (each colour set; a missing
/// one is the scheme's default).
#[must_use]
pub fn derive(palette: &Palette, scheme: Scheme) -> BTreeMap<String, String> {
    // The ratio aimed at: 4.5:1 (WCAG AA), with room for the rounding to hex.
    const AA: f32 = 4.6;
    let fallback = default_palette(scheme);
    let color = |own: &Option<String>, def: &Option<String>| {
        own.as_deref()
            .or(def.as_deref())
            .and_then(Rgba::parse)
            .unwrap_or(MAGENTA)
    };
    let dark = scheme.is_dark();
    let bg = color(&palette.background, &fallback.background);
    let fg = color(&palette.foreground, &fallback.foreground);
    let acc = color(&palette.accent, &fallback.accent);
    let safe = color(&palette.safe, &fallback.safe);
    let warn = color(&palette.warn, &fallback.warn);
    let danger = color(&palette.danger, &fallback.danger);
    let info = color(&palette.info, &fallback.info);
    // The far end ink moves toward when it needs more contrast.
    let far = if dark { WHITE } else { BLACK };
    let step = |t: f32| bg.mix(fg, t);

    let surface = palette
        .surface
        .as_deref()
        .and_then(Rgba::parse)
        .unwrap_or_else(|| {
            if dark {
                step(0.035)
            } else {
                bg.mix(WHITE, 0.7)
            }
        });
    let border = palette
        .border
        .as_deref()
        .and_then(Rgba::parse)
        .unwrap_or_else(|| step(if dark { 0.15 } else { 0.13 }));

    let desk = bg;
    let sheet = step(if dark { 0.02 } else { 0.025 });
    let title_bar = step(if dark { 0.065 } else { 0.05 });
    let pill = if dark {
        step(0.075)
    } else {
        bg.mix(WHITE, 0.8)
    };
    let rule = step(if dark { 0.14 } else { 0.12 });
    let brand_soft = acc.alpha(0.14);
    let shell_grounds = [desk, sheet, title_bar, pill, brand_soft.over(pill)];
    let ink_2 = with_contrast(fg.mix(bg, 0.3), &shell_grounds, AA, far);
    let ink_3 = with_contrast(fg.mix(bg, 0.45), &shell_grounds, AA, far);
    let brand_ink = ink_on(acc);

    let secondary = step(if dark { 0.09 } else { 0.05 });
    let muted = step(if dark { 0.07 } else { 0.04 });
    let muted_fg = with_contrast(fg.mix(bg, 0.42), &[bg, muted, surface], AA, far);
    let accent = bg.mix(acc, if dark { 0.16 } else { 0.10 });
    let accent_fg = with_contrast(fg, &[accent], AA, far);
    let chart_default = [acc, info, safe, warn, danger];

    let mut t: BTreeMap<String, String> = BTreeMap::new();
    let mut put = |name: &str, value: String| {
        t.insert(name.to_owned(), value);
    };
    let css = Rgba::to_css;

    put("background", css(bg));
    put("foreground", css(fg));
    put("card", css(surface));
    put("card-foreground", css(fg));
    put("popover", css(surface));
    put("popover-foreground", css(fg));
    put("primary", css(acc));
    put("primary-foreground", css(brand_ink));
    put("secondary", css(secondary));
    put("secondary-foreground", css(fg));
    put("muted", css(muted));
    put("muted-foreground", css(muted_fg));
    put("accent", css(accent));
    put("accent-foreground", css(accent_fg));
    put("destructive", css(danger));
    put("destructive-foreground", css(ink_on(danger)));
    put("border", css(border));
    put("input", css(border));
    put("ring", css(acc));
    for (i, default) in chart_default.iter().enumerate() {
        let own = palette.chart.get(i).and_then(|c| Rgba::parse(c));
        put(
            &format!("chart-{}", i.saturating_add(1)),
            css(own.unwrap_or(*default)),
        );
    }
    put("sidebar", css(sheet));
    put("sidebar-foreground", css(fg));
    put("sidebar-primary", css(acc));
    put("sidebar-primary-foreground", css(brand_ink));
    put("sidebar-accent", css(accent));
    put("sidebar-accent-foreground", css(accent_fg));
    put("sidebar-border", css(border));
    put("sidebar-ring", css(acc));

    put("safe", css(safe));
    put("warn", css(warn));
    put("danger", css(danger));
    put("info", css(info));
    put("signal-safe", css(safe));
    put("signal-warn", css(warn));
    put("signal-danger", css(danger));
    // A DAW's own signals: fts-theme.css's, until a theme says otherwise.
    for (name, value) in [
        ("signal-override", "oklch(0.68 0.16 55)"),
        ("signal-automation", "oklch(0.65 0.15 145)"),
        ("signal-mod", "oklch(0.58 0.20 290)"),
        ("signal-slot-a", "oklch(0.65 0.15 250)"),
        ("signal-slot-b", "oklch(0.68 0.16 55)"),
        ("signal-quick", "oklch(0.60 0.15 250)"),
        ("signal-full", "oklch(0.55 0.18 300)"),
    ] {
        put(name, value.to_owned());
    }

    put("desk", css(desk));
    put("sheet", css(sheet));
    put("title-bar", css(title_bar));
    put("pill", css(pill));
    put("rule", css(rule));
    put("active", css(fg.alpha(if dark { 0.10 } else { 0.08 })));
    put("ink", css(fg));
    put("ink-2", css(ink_2));
    put("ink-3", css(ink_3));
    put("brand", css(acc));
    put("brand-ink", css(brand_ink));
    put("brand-soft", css(brand_soft));
    put("scrim", css(BLACK.alpha(if dark { 0.55 } else { 0.32 })));
    let (lift, float) = if dark { (0.35, 0.55) } else { (0.12, 0.18) };
    put("shadow-lift", format!("0 6px 20px rgba(0,0,0,{lift:.2})"));
    put(
        "shadow-float",
        format!("0 16px 44px rgba(0,0,0,{float:.2})"),
    );
    put("font", SHELL_FONT.to_owned());

    for (name, value) in [
        ("font-sans", "ui-sans-serif, system-ui, sans-serif"),
        ("font-serif", "ui-serif, Georgia, serif"),
        (
            "font-mono",
            "ui-monospace, SFMono-Regular, Menlo, monospace",
        ),
        ("radius", "0.625rem"),
        ("spacing", "0.25rem"),
        ("letter-spacing", "0"),
        ("shadow-color", "oklch(0 0 0)"),
        ("shadow-opacity", "0.1"),
        ("shadow-blur", "3px"),
        ("shadow-spread", "0px"),
        ("shadow-offset-x", "0px"),
        ("shadow-offset-y", "1px"),
    ] {
        put(name, value.to_owned());
    }
    t
}

/// Text on `fill`: a deep shade of it or white, whichever reads better,
/// pushed to 4.5:1 where it can be.
fn ink_on(fill: Rgba) -> Rgba {
    let deep = fill.mix(BLACK, 0.88);
    let (best, far) = if contrast(deep, fill) >= contrast(WHITE, fill) {
        (deep, BLACK)
    } else {
        (WHITE, WHITE)
    };
    with_contrast(best, &[fill], 4.6, far)
}

/// Where a token's final value came from ([`Resolved::source`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// Derived from the theme's palette.
    Derived,
    /// A theme file's (or an in-memory theme's) `overrides`.
    Theme,
    /// A layer's override (`Layer::token`, `THEME_TOKEN_SET`).
    Layer,
}

impl Origin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Derived => "derived",
            Self::Theme => "theme",
            Self::Layer => "layer",
        }
    }
}

/// Who set a token's final value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSource {
    pub origin: Origin,
    /// The layer that set it ([`Origin::Layer`]), or the one whose choice
    /// shows the theme that did; `None` for a theme resolved on its own
    /// ([`resolve`]).
    pub layer: Option<LayerKind>,
    /// The theme that derived or set it (a parent's id when the parent's
    /// overrides did), or the layer's name.
    pub id: String,
    /// The expression as written, when the value named other tokens
    /// (`mix($border, $foreground, 0.2)`); the value itself is evaluated.
    pub expression: Option<String>,
}

impl TokenSource {
    /// Whether the value was an expression over other tokens.
    #[must_use]
    pub const fn is_reference(&self) -> bool {
        self.expression.is_some()
    }
}

/// A theme resolved: one flat map of tokens (CSS values by name).
///
/// No leading `--`. With it: its scheme, the theme it is, where each value
/// came from, what did not resolve, and a revision the store bumps on
/// every change. Cheap to clone (an `Arc`): a reader on any thread holds one
/// whole, consistent snapshot.
#[derive(Debug, Clone)]
pub struct Resolved {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    id: String,
    scheme: Scheme,
    tokens: BTreeMap<String, String>,
    sources: BTreeMap<String, TokenSource>,
    problems: Vec<Problem>,
    revision: u64,
    /// The names asked for and not there, since it was resolved.
    misses: Mutex<BTreeSet<String>>,
}

impl PartialEq for Resolved {
    fn eq(&self, other: &Self) -> bool {
        self.inner.revision == other.inner.revision && self.same_look(other)
    }
}

impl Eq for Resolved {}

impl Default for Resolved {
    /// The built-in default theme, revision 0.
    fn default() -> Self {
        crate::builtin::default_resolved()
    }
}

impl Resolved {
    /// Literal `tokens`, as they are (nothing evaluated, no sources).
    #[must_use]
    pub fn new(id: &str, scheme: Scheme, tokens: BTreeMap<String, String>) -> Self {
        Self::build(id, scheme, tokens, BTreeMap::new(), Vec::new())
    }

    fn build(
        id: &str,
        scheme: Scheme,
        tokens: BTreeMap<String, String>,
        sources: BTreeMap<String, TokenSource>,
        problems: Vec<Problem>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                id: id.to_owned(),
                scheme,
                tokens,
                sources,
                problems,
                revision: 0,
                misses: Mutex::default(),
            }),
        }
    }

    /// The same, at `revision`.
    #[must_use]
    pub fn at_revision(&self, revision: u64) -> Self {
        let i = &self.inner;
        Self {
            inner: Arc::new(Inner {
                id: i.id.clone(),
                scheme: i.scheme,
                tokens: i.tokens.clone(),
                sources: i.sources.clone(),
                problems: i.problems.clone(),
                revision,
                misses: Mutex::default(),
            }),
        }
    }

    /// The theme's id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    #[must_use]
    pub fn scheme(&self) -> Scheme {
        self.inner.scheme
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.inner.revision
    }

    /// Every token, architect's and the app's (dotted) alike.
    #[must_use]
    pub fn tokens(&self) -> &BTreeMap<String, String> {
        &self.inner.tokens
    }

    /// architect's own tokens, the undotted ones: what a UI kit writes as
    /// CSS variables (`--ink-2`). An app's namespaced tokens are left out
    /// ([`Self::css_namespaced`] asks for them).
    pub fn css_tokens(&self) -> impl Iterator<Item = (&str, &str)> {
        self.inner
            .tokens
            .iter()
            .filter(|(k, _)| !k.contains('.'))
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// An app's dotted tokens in `namespaces` (`["arrange", "mixer"]`: each
    /// name starting `arrange.` or `mixer.`), as CSS variable names
    /// ([`css_name`]: `arrange-item-edge`) and values, by name. The
    /// opt-in for an app whose own CSS reads `var(--arrange-item-edge)`.
    /// A name that would collide with one of architect's undotted tokens
    /// (`ink.2` and `ink-2`) is left out: architect's wins.
    #[must_use]
    pub fn css_namespaced(&self, namespaces: &[&str]) -> Vec<(String, &str)> {
        self.inner
            .tokens
            .iter()
            .filter(|(k, _)| {
                k.split_once('.')
                    .is_some_and(|(head, _)| namespaces.contains(&head))
            })
            .map(|(k, v)| (css_name(k), v.as_str()))
            .filter(|(css, _)| !self.inner.tokens.contains_key(css))
            .collect()
    }

    /// The token `name`, if the theme has it.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.inner.tokens.get(name).map(String::as_str)
    }

    /// The token `name`'s value; magenta (`#ff00ff`) for a name the
    /// theme does not have, so the mistake shows — logged, once per name,
    /// and kept in [`Self::misses`].
    #[must_use]
    pub fn value(&self, name: &str) -> &str {
        self.get(name).unwrap_or_else(|| {
            self.miss(name);
            MAGENTA_CSS
        })
    }

    /// The token `name` as a colour; magenta for a name the theme does
    /// not have (logged once, a miss) or a value that is not a colour.
    #[must_use]
    pub fn color(&self, name: &str) -> Rgba {
        Rgba::parse(self.value(name)).unwrap_or(MAGENTA)
    }

    /// The token `name` as a number (`0.4`, `12`): an alpha, a weight, a
    /// size in the app's own units. `NaN` for a name the theme does not
    /// have (logged once, a miss) or a value that is not a plain number
    /// (logged once).
    #[must_use]
    pub fn number(&self, name: &str) -> f32 {
        let Some(value) = self.get(name) else {
            self.miss(name);
            return f32::NAN;
        };
        match value.trim().parse::<f32>() {
            Ok(n) if n.is_finite() => n,
            _ => {
                log_once(name, "theme token is not a number, read as NaN");
                f32::NAN
            }
        }
    }

    /// Who set token `name`'s final value: derived from a theme's palette,
    /// a theme's overrides, or a layer's, and whether it was an expression.
    /// `None` for a name it does not have (or a [`Self::new`] map).
    #[must_use]
    pub fn source(&self, name: &str) -> Option<&TokenSource> {
        self.inner.sources.get(name)
    }

    /// The expressions that did not resolve (an unknown `$name`, a cycle,
    /// a value that is not a colour): each token named, drawn magenta.
    #[must_use]
    pub fn problems(&self) -> &[Problem] {
        &self.inner.problems
    }

    /// The names asked for ([`Self::value`], [`Self::color`],
    /// [`Self::number`]) and not found since it was resolved, sorted.
    #[must_use]
    pub fn misses(&self) -> Vec<String> {
        self.inner
            .misses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    fn miss(&self, name: &str) {
        let first = self
            .inner
            .misses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned());
        if first {
            log_once(name, "unknown theme token, drawn magenta");
        }
    }

    /// Whether it looks the same as `other` (the revision aside): the
    /// same theme, scheme and tokens, set by the same layers, with the
    /// same problems.
    #[must_use]
    pub fn same_look(&self, other: &Self) -> bool {
        let (a, b) = (&self.inner, &other.inner);
        a.id == b.id
            && a.scheme == b.scheme
            && a.tokens == b.tokens
            && a.sources == b.sources
            && a.problems == b.problems
    }
}

/// A token name as a CSS variable's (without its `--`): the dots become
/// dashes, `arrange.item.edge` → `arrange-item-edge`.
#[must_use]
pub fn css_name(name: &str) -> String {
    name.replace('.', "-")
}

/// Logs `message` for token `name`, once per name and message for the
/// process.
fn log_once(name: &str, message: &'static str) {
    static SEEN: OnceLock<Mutex<BTreeSet<(String, &'static str)>>> = OnceLock::new();
    let first = SEEN
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert((name.to_owned(), message));
    if first {
        tracing::warn!(target: "architect_theme", token = name, "{message}");
    }
}

/// Builds a typed slice of a resolved theme: what a consumer caches and
/// compares, so it redraws only when its own part changes.
pub trait FromResolved: Sized {
    fn from_resolved(resolved: &Resolved) -> Self;
}

/// A consumer's cached projection (herdr's `Vt::set_theme`): `update`
/// says whether its own slice changed, so only then does it redraw or
/// drop its caches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projected<T> {
    value: T,
    revision: u64,
}

impl<T: FromResolved + PartialEq> Projected<T> {
    #[must_use]
    pub fn new(resolved: &Resolved) -> Self {
        Self {
            value: T::from_resolved(resolved),
            revision: resolved.revision(),
        }
    }

    #[must_use]
    pub const fn get(&self) -> &T {
        &self.value
    }

    /// The revision of the theme it was last cut from: a recording stamps
    /// it to say which theme it was drawn with.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Takes `resolved`; true when the slice it gives differs from the
    /// one kept.
    pub fn update(&mut self, resolved: &Resolved) -> bool {
        if resolved.revision() == self.revision {
            return false;
        }
        self.revision = resolved.revision();
        let next = T::from_resolved(resolved);
        if next == self.value {
            false
        } else {
            self.value = next;
            true
        }
    }
}
