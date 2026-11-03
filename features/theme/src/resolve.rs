//! A theme resolved: every semantic token, derived from its palette, then
//! its overrides. One flat map ([`Resolved`]) every consumer reads.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::color::{BLACK, MAGENTA, Rgba, WHITE, contrast, with_contrast};
use crate::model::{Palette, Registry, Scheme, Theme};

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
/// overrides, the parents' first. Deterministic: the same theme and
/// registry give the same map. A missing or looping parent is left out
/// (it is one of [`Registry::problems`]).
#[must_use]
pub fn resolve(theme: &Theme, registry: &Registry) -> Resolved {
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
    let mut tokens = derive(&palette, scheme);
    for t in chain.iter().rev() {
        for (name, value) in &t.file.overrides {
            tokens.insert(name.clone(), value.clone());
        }
    }
    Resolved::new(&theme.id, scheme, tokens)
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

/// A theme resolved: one flat map of tokens (CSS values by name, no
/// leading `--`), its scheme, the theme it is, and a revision the store
/// bumps on every change. Cheap to clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    inner: Arc<Inner>,
}

#[derive(Debug, PartialEq, Eq)]
struct Inner {
    id: String,
    scheme: Scheme,
    tokens: BTreeMap<String, String>,
    revision: u64,
}

impl Default for Resolved {
    /// The built-in default theme, revision 0.
    fn default() -> Self {
        crate::builtin::default_resolved()
    }
}

impl Resolved {
    #[must_use]
    pub fn new(id: &str, scheme: Scheme, tokens: BTreeMap<String, String>) -> Self {
        Self {
            inner: Arc::new(Inner {
                id: id.to_owned(),
                scheme,
                tokens,
                revision: 0,
            }),
        }
    }

    /// The same, at `revision`.
    #[must_use]
    pub fn at_revision(&self, revision: u64) -> Self {
        Self {
            inner: Arc::new(Inner {
                id: self.inner.id.clone(),
                scheme: self.inner.scheme,
                tokens: self.inner.tokens.clone(),
                revision,
            }),
        }
    }

    /// The same, with `overrides` over its tokens.
    #[must_use]
    pub fn with_overrides(&self, overrides: &BTreeMap<String, String>) -> Self {
        if overrides.is_empty() {
            return self.clone();
        }
        let mut tokens = self.inner.tokens.clone();
        tokens.extend(overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
        Self {
            inner: Arc::new(Inner {
                id: self.inner.id.clone(),
                scheme: self.inner.scheme,
                tokens,
                revision: self.inner.revision,
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

    /// Every token.
    #[must_use]
    pub fn tokens(&self) -> &BTreeMap<String, String> {
        &self.inner.tokens
    }

    /// The token `name`, if the theme has it.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.inner.tokens.get(name).map(String::as_str)
    }

    /// The token `name`'s value; magenta (`#ff00ff`) for a name the
    /// theme does not have, so the mistake shows — and is logged, once
    /// per name.
    #[must_use]
    pub fn value(&self, name: &str) -> &str {
        self.get(name).unwrap_or_else(|| {
            unknown(name);
            "#ff00ff"
        })
    }

    /// The token `name` as a colour; magenta for a name the theme does
    /// not have (logged once) or a value that is not a colour.
    #[must_use]
    pub fn color(&self, name: &str) -> Rgba {
        Rgba::parse(self.value(name)).unwrap_or(MAGENTA)
    }

    /// Whether it looks the same as `other` (the revision aside).
    #[must_use]
    pub fn same_look(&self, other: &Self) -> bool {
        self.inner.id == other.inner.id
            && self.inner.scheme == other.inner.scheme
            && self.inner.tokens == other.inner.tokens
    }
}

/// Logs an unknown token, once per name for the process.
fn unknown(name: &str) {
    static SEEN: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    let first = SEEN
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(name.to_owned());
    if first {
        tracing::warn!(
            target: "architect_theme",
            token = name,
            "unknown theme token, drawn magenta"
        );
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
