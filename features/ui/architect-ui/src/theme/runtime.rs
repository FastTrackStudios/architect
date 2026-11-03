use dioxus::prelude::*;

use std::fmt::Write as _;

use crate::components::{Button, ButtonVariant, Label, Select, SelectContent, SelectItem};

const SHADOW_KEYS: [&str; 6] = [
    "shadow-color",
    "shadow-opacity",
    "shadow-blur",
    "shadow-spread",
    "shadow-offset-x",
    "shadow-offset-y",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeToken {
    pub key: String,
    pub value: String,
}

impl ThemeToken {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeStyle {
    pub tokens: Vec<ThemeToken>,
}

impl ThemeStyle {
    pub fn new(tokens: impl IntoIterator<Item = ThemeToken>) -> Self {
        Self {
            tokens: tokens.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.tokens
            .iter()
            .find(|token| token.key == key)
            .map(|token| token.value.as_str())
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();

        if let Some(token) = self.tokens.iter_mut().find(|token| token.key == key) {
            token.value = value;
        } else {
            self.tokens.push(ThemeToken::new(key, value));
        }
    }

    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.set(key, value);
        self
    }

    #[must_use]
    pub fn css_variables(&self) -> String {
        let mut css = String::new();

        for token in &self.tokens {
            if !token.value.trim().is_empty() {
                css.push_str("--");
                css.push_str(&token.key);
                css.push_str(": ");
                css.push_str(&token.value);
                css.push(';');
            }
        }

        append_tailwind_alias_variables(&mut css, self);
        append_shadow_variables(&mut css, self);
        css
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeStyles {
    pub light: ThemeStyle,
    pub dark: ThemeStyle,
}

impl ThemeStyles {
    #[must_use]
    pub const fn active(&self, mode: ThemeMode) -> &ThemeStyle {
        match mode {
            ThemeMode::Light => &self.light,
            ThemeMode::Dark => &self.dark,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemePreset {
    pub name: String,
    pub label: String,
    pub styles: ThemeStyles,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeState {
    pub mode: ThemeMode,
    pub preset: String,
    pub styles: ThemeStyles,
}

impl ThemeState {
    #[must_use]
    pub fn new(preset: ThemePreset, mode: ThemeMode) -> Self {
        Self {
            mode,
            preset: preset.name,
            styles: preset.styles,
        }
    }

    #[must_use]
    pub const fn active_style(&self) -> &ThemeStyle {
        self.styles.active(self.mode)
    }

    pub const fn set_mode(&mut self, mode: ThemeMode) {
        self.mode = mode;
    }

    pub fn set_preset(&mut self, preset: ThemePreset) {
        self.preset = preset.name;
        self.styles = preset.styles;
    }

    pub fn set_token(&mut self, mode: ThemeMode, key: impl Into<String>, value: impl Into<String>) {
        match mode {
            ThemeMode::Light => self.styles.light.set(key, value),
            ThemeMode::Dark => self.styles.dark.set(key, value),
        }
    }
}

#[derive(Clone, Copy)]
pub struct ThemeContext {
    pub state: Signal<ThemeState>,
}

#[derive(Props, Clone, PartialEq)]
pub struct ThemeProviderProps {
    pub state: Signal<ThemeState>,
    #[props(default = false)]
    pub allow_layout_scale: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ThemeProvider(props: ThemeProviderProps) -> Element {
    use_context_provider(|| ThemeContext { state: props.state });

    let state = (props.state)();
    let style = theme_style_attribute(&state.styles, state.mode, props.allow_layout_scale);
    let mode = state.mode.as_str();

    rsx! {
        ThemeRuntimeStyle {}
        div {
            class: crate::cn::merge_slice(&["fts-theme-root min-h-full", props.class.as_str()]),
            "data-theme-mode": mode,
            style,
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ThemeScopeProps {
    pub styles: ThemeStyles,
    #[props(default)]
    pub mode: Option<ThemeMode>,
    #[props(default = false)]
    pub allow_layout_scale: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ThemeScope(props: ThemeScopeProps) -> Element {
    let inherited = try_consume_context::<ThemeContext>();
    let inherited_mode = inherited
        .as_ref()
        .map_or(ThemeMode::Light, |context| (context.state)().mode);
    let mode = props.mode.unwrap_or(inherited_mode);
    let style = theme_style_attribute(&props.styles, mode, props.allow_layout_scale);

    rsx! {
        div {
            class: crate::cn::merge_slice(&["fts-theme-scope", props.class.as_str()]),
            "data-theme-mode": mode.as_str(),
            style,
            {props.children}
        }
    }
}

#[component]
pub fn ThemeRuntimeStyle() -> Element {
    rsx! {
        document::Style {
            r#"
                .fts-theme-root,
                .fts-theme-scope {{
                    font-family: var(--default-font-family, var(--font-sans));
                    letter-spacing: var(--letter-spacing, 0);
                }}

                .fts-theme-root *,
                .fts-theme-scope * {{
                    letter-spacing: inherit;
                }}

                .fts-theme-root .font-serif,
                .fts-theme-scope .font-serif {{
                    font-family: var(--font-serif);
                }}

                .fts-theme-root .font-mono,
                .fts-theme-scope .font-mono {{
                    font-family: var(--font-mono);
                }}

                .fts-theme-root .shadow-xs,
                .fts-theme-scope .shadow-xs {{
                    box-shadow: var(--shadow-xs) !important;
                }}

                .fts-theme-root .shadow-sm,
                .fts-theme-scope .shadow-sm {{
                    box-shadow: var(--shadow-sm) !important;
                }}

                .fts-theme-root .shadow-md,
                .fts-theme-scope .shadow-md {{
                    box-shadow: var(--shadow-md) !important;
                }}

                .fts-theme-root .shadow-lg,
                .fts-theme-scope .shadow-lg {{
                    box-shadow: var(--shadow-lg) !important;
                }}
            "#
        }
    }
}

#[derive(Props, Clone, PartialEq, Eq)]
pub struct ThemeSwitcherProps {
    pub state: Signal<ThemeState>,
    #[props(default)]
    pub class: String,
}

#[component]
pub fn ThemeSwitcher(mut props: ThemeSwitcherProps) -> Element {
    let current = (props.state)();
    let mode = current.mode;
    let preset = current.preset.clone();
    let radius = current
        .active_style()
        .get("radius")
        .unwrap_or("0.625rem")
        .to_string();
    let spacing = current
        .active_style()
        .get("spacing")
        .unwrap_or("0.25rem")
        .to_string();
    let font_sans = current
        .active_style()
        .get("font-sans")
        .unwrap_or("ui-sans-serif, system-ui, sans-serif")
        .to_string();
    let mut preset_value = use_signal(|| preset.clone());
    let input_class = "h-9 w-full rounded-lg border border-input bg-input/30 px-3 py-1 text-sm shadow-xs transition-colors focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50";

    use_effect(move || {
        preset_value.set(preset.clone());
    });

    rsx! {
        div {
            class: crate::cn::merge_slice(
                &["grid gap-3 rounded-lg border border-border bg-card p-4 text-card-foreground shadow-sm", props.class.as_str()],
            ),
            div { class: "grid gap-1",
                Label { "Preset" }
                Select {
                    value: preset_value,
                    placeholder: "Choose a theme...".to_string(),
                    on_change: move |value: String| {
                        if let Some(preset) = theme_preset(&value) {
                            props.state.write().set_preset(preset);
                        }
                    },
                    SelectContent {
                        for (index, preset_option) in theme_presets().into_iter().enumerate() {
                            SelectItem {
                                value: preset_option.name,
                                index,
                                "{preset_option.label}"
                            }
                        }
                    }
                }
            }
            div { class: "grid grid-cols-2 gap-2",
                Button {
                    variant: if mode == ThemeMode::Light { ButtonVariant::Primary } else { ButtonVariant::Outline },
                    on_click: move |_| props.state.write().set_mode(ThemeMode::Light),
                    "Light"
                }
                Button {
                    variant: if mode == ThemeMode::Dark { ButtonVariant::Primary } else { ButtonVariant::Outline },
                    on_click: move |_| props.state.write().set_mode(ThemeMode::Dark),
                    "Dark"
                }
            }
            div { class: "grid gap-1",
                Label { "Radius" }
                input {
                    class: input_class,
                    value: "{radius}",
                    oninput: move |event: FormEvent| props.state.write().set_token(mode, "radius", event.value()),
                }
            }
            div { class: "grid gap-1",
                Label { "Spacing" }
                input {
                    class: input_class,
                    value: "{spacing}",
                    oninput: move |event: FormEvent| props.state.write().set_token(mode, "spacing", event.value()),
                }
            }
            div { class: "grid gap-1",
                Label { "Typography" }
                input {
                    class: input_class,
                    value: "{font_sans}",
                    oninput: move |event: FormEvent| props.state.write().set_token(mode, "font-sans", event.value()),
                }
            }
        }
    }
}

#[must_use]
pub fn use_theme() -> ThemeContext {
    use_context::<ThemeContext>()
}

#[must_use]
pub fn default_theme_state() -> ThemeState {
    ThemeState::new(default_theme_preset(), ThemeMode::Light)
}

#[must_use]
pub fn theme_preset(name: &str) -> Option<ThemePreset> {
    theme_presets()
        .into_iter()
        .find(|preset| preset.name == name)
}

fn theme_style_attribute(
    styles: &ThemeStyles,
    mode: ThemeMode,
    allow_layout_scale: bool,
) -> String {
    let mut style = styles.active(mode).css_variables();
    let color_scheme = if mode.is_dark() { "dark" } else { "light" };

    if !allow_layout_scale {
        push_css_var(&mut style, "spacing", "0.25rem");
    }

    style.push_str("color-scheme: ");
    style.push_str(color_scheme);
    style.push(';');
    style.push_str("color: var(--foreground); background-color: var(--background);");
    style
}

fn append_tailwind_alias_variables(css: &mut String, style: &ThemeStyle) {
    if let Some(font_sans) = style.get("font-sans") {
        push_css_var(css, "default-font-family", font_sans);
    }

    if let Some(font_mono) = style.get("font-mono") {
        push_css_var(css, "default-mono-font-family", font_mono);
    }

    if let Some(radius) = style.get("radius") {
        push_css_var(css, "radius-sm", &format!("calc({radius} - 4px)"));
        push_css_var(css, "radius-md", &format!("calc({radius} - 2px)"));
        push_css_var(css, "radius-lg", radius);
        push_css_var(css, "radius-xl", &format!("calc({radius} + 4px)"));
        push_css_var(css, "radius-2xl", &format!("calc({radius} + 8px)"));
        push_css_var(css, "radius-3xl", &format!("calc({radius} + 12px)"));
    }

    if let Some(letter_spacing) = style.get("letter-spacing") {
        push_css_var(css, "tracking-tight", letter_spacing);
        push_css_var(css, "tracking-normal", letter_spacing);
        push_css_var(css, "tracking-wide", letter_spacing);
    }
}

fn append_shadow_variables(css: &mut String, style: &ThemeStyle) {
    if !SHADOW_KEYS.iter().all(|key| style.get(key).is_some()) {
        return;
    }

    let color = style.get("shadow-color").unwrap_or("oklch(0 0 0)");
    let opacity = style.get("shadow-opacity").unwrap_or("0.1");
    let blur = style.get("shadow-blur").unwrap_or("3px");
    let spread = style.get("shadow-spread").unwrap_or("0px");
    let x = style.get("shadow-offset-x").unwrap_or("0px");
    let y = style.get("shadow-offset-y").unwrap_or("1px");

    let color_half = format!("color-mix(in oklab, {color} calc({opacity} * 50%), transparent)");
    let color_full = format!("color-mix(in oklab, {color} calc({opacity} * 100%), transparent)");

    // `write!` into `css` rather than `push_str(&format!(..))`: same
    // output, one fewer temporary `String` per line. Writing to a
    // `String` is infallible, so the result is discarded.
    let _ = write!(css, "--shadow-xs: {x} {y} {blur} {spread} {color_half};");
    let _ = write!(
        css,
        "--shadow-sm: {x} {y} {blur} {spread} {color_full}, {x} 1px 2px -1px {color_full};"
    );
    let _ = write!(
        css,
        "--shadow-md: {x} {y} {blur} {spread} {color_full}, {x} 2px 4px -1px {color_full};"
    );
    let _ = write!(
        css,
        "--shadow-lg: {x} {y} {blur} {spread} {color_full}, {x} 4px 6px -1px {color_full};"
    );
}

fn push_css_var(css: &mut String, key: &str, value: &str) {
    if value.trim().is_empty() {
        return;
    }

    css.push_str("--");
    css.push_str(key);
    css.push_str(": ");
    css.push_str(value);
    css.push(';');
}

/// Every resolved token, as the CSS variables [`ThemeProvider`] sets.
impl architect_theme::FromResolved for ThemeStyle {
    fn from_resolved(resolved: &architect_theme::Resolved) -> Self {
        Self::new(
            resolved
                .tokens()
                .iter()
                .map(|(key, value)| ThemeToken::new(key.clone(), value.clone())),
        )
    }
}

impl ThemeState {
    /// The theme a store resolved, as the state a [`ThemeProvider`]
    /// draws: both modes its style, the mode its scheme, the preset its
    /// family (the id without `-light` / `-dark`).
    #[must_use]
    pub fn from_resolved(resolved: &architect_theme::Resolved) -> Self {
        use architect_theme::FromResolved as _;
        let style = ThemeStyle::from_resolved(resolved);
        Self {
            mode: mode_of(resolved.scheme()),
            preset: family(resolved.id()).to_owned(),
            styles: ThemeStyles {
                light: style.clone(),
                dark: style,
            },
        }
    }
}

const fn mode_of(scheme: architect_theme::Scheme) -> ThemeMode {
    match scheme {
        architect_theme::Scheme::Light => ThemeMode::Light,
        architect_theme::Scheme::Dark => ThemeMode::Dark,
    }
}

/// A theme id's family: `catppuccin-dark` → `catppuccin`.
fn family(id: &str) -> &str {
    id.strip_suffix("-dark")
        .or_else(|| id.strip_suffix("-light"))
        .unwrap_or(id)
}

/// A [`ThemeProvider`] that follows the app's theme store
/// (`architect_theme::use_theme_store` above it): every change the store
/// makes — a pick, a preview, a token, a theme file saved — reaches the
/// CSS variables. With no store above, the built-in default.
#[component]
pub fn ThemeStoreProvider(
    #[props(default = false)] allow_layout_scale: bool,
    #[props(default)] class: String,
    children: Element,
) -> Element {
    let resolved = architect_theme::use_resolved();
    let next = ThemeState::from_resolved(&resolved);
    let mut state = use_signal(|| next.clone());
    if *state.peek() != next {
        state.set(next);
    }
    rsx! {
        ThemeProvider { state, allow_layout_scale, class, {children} }
    }
}

/// A built-in theme pair (`<name>-light`, `<name>-dark`) as a preset.
fn builtin_preset(name: &str) -> Option<ThemePreset> {
    use architect_theme::FromResolved as _;
    let registry = architect_theme::Registry::builtin();
    let half = |scheme: &str| {
        registry
            .get(&format!("{name}-{scheme}"))
            .map(|t| (t.label.clone(), architect_theme::resolve(t, &registry)))
    };
    let (label, light) = half("light")?;
    let (_, dark) = half("dark")?;
    Some(ThemePreset {
        name: name.to_owned(),
        label: label.strip_suffix(" Light").unwrap_or(&label).to_owned(),
        styles: ThemeStyles {
            light: ThemeStyle::from_resolved(&light),
            dark: ThemeStyle::from_resolved(&dark),
        },
    })
}

#[must_use]
pub fn default_theme_preset() -> ThemePreset {
    builtin_preset("default").unwrap_or_else(|| ThemePreset {
        name: "default".to_owned(),
        label: "Default".to_owned(),
        styles: ThemeStyles {
            light: ThemeStyle::new([]),
            dark: ThemeStyle::new([]),
        },
    })
}

/// Every preset: architect-theme's built-in light and dark pairs
/// (`features/theme/themes/`), `default` first.
#[must_use]
pub fn theme_presets() -> Vec<ThemePreset> {
    let mut names: Vec<&str> = architect_theme::builtin::FILES
        .iter()
        .filter_map(|(id, _)| id.strip_suffix("-light"))
        .collect();
    names.sort_by_key(|n| *n != "default");
    names.into_iter().filter_map(builtin_preset).collect()
}
