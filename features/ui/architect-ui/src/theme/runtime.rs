use dioxus::prelude::*;

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
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeStyle {
    pub tokens: Vec<ThemeToken>,
}

impl ThemeStyle {
    pub fn new(tokens: impl IntoIterator<Item = ThemeToken>) -> Self {
        Self {
            tokens: tokens.into_iter().collect(),
        }
    }

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

    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.set(key, value);
        self
    }

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

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeStyles {
    pub light: ThemeStyle,
    pub dark: ThemeStyle,
}

impl ThemeStyles {
    pub fn active(&self, mode: ThemeMode) -> &ThemeStyle {
        match mode {
            ThemeMode::Light => &self.light,
            ThemeMode::Dark => &self.dark,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemePreset {
    pub name: String,
    pub label: String,
    pub styles: ThemeStyles,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeState {
    pub mode: ThemeMode,
    pub preset: String,
    pub styles: ThemeStyles,
}

impl ThemeState {
    pub fn new(preset: ThemePreset, mode: ThemeMode) -> Self {
        Self {
            mode,
            preset: preset.name,
            styles: preset.styles,
        }
    }

    pub fn active_style(&self) -> &ThemeStyle {
        self.styles.active(self.mode)
    }

    pub fn set_mode(&mut self, mode: ThemeMode) {
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
        .map(|context| (context.state)().mode)
        .unwrap_or(ThemeMode::Light);
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

#[derive(Props, Clone, PartialEq)]
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

pub fn use_theme() -> ThemeContext {
    use_context::<ThemeContext>()
}

pub fn default_theme_state() -> ThemeState {
    ThemeState::new(default_theme_preset(), ThemeMode::Light)
}

pub fn default_theme_preset() -> ThemePreset {
    ThemePreset {
        name: "default".to_string(),
        label: "Default".to_string(),
        styles: ThemeStyles {
            light: default_light_style(),
            dark: default_dark_style(),
        },
    }
}

pub fn theme_presets() -> Vec<ThemePreset> {
    vec![
        default_theme_preset(),
        fasttrackstudio_theme_preset(),
        modern_minimal_theme_preset(),
        violet_bloom_theme_preset(),
        t3_chat_theme_preset(),
        amethyst_haze_theme_preset(),
        doom_64_theme_preset(),
        graphite_theme_preset(),
        supabase_theme_preset(),
        neo_brutalism_theme_preset(),
        catppuccin_theme_preset(),
        nord_theme_preset(),
        dracula_theme_preset(),
        gruvbox_theme_preset(),
        solarized_theme_preset(),
        tokyo_night_theme_preset(),
        rose_pine_theme_preset(),
        everforest_theme_preset(),
        zen_theme_preset(),
        obsidian_theme_preset(),
    ]
}

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

    css.push_str("--shadow-xs: ");
    css.push_str(&format!("{x} {y} {blur} {spread} {color_half};"));
    css.push_str("--shadow-sm: ");
    css.push_str(&format!(
        "{x} {y} {blur} {spread} {color_full}, {x} 1px 2px -1px {color_full};"
    ));
    css.push_str("--shadow-md: ");
    css.push_str(&format!(
        "{x} {y} {blur} {spread} {color_full}, {x} 2px 4px -1px {color_full};"
    ));
    css.push_str("--shadow-lg: ");
    css.push_str(&format!(
        "{x} {y} {blur} {spread} {color_full}, {x} 4px 6px -1px {color_full};"
    ));
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

fn default_light_style() -> ThemeStyle {
    ThemeStyle::new([
        ThemeToken::new("background", "oklch(1 0 0)"),
        ThemeToken::new("foreground", "oklch(0.145 0 0)"),
        ThemeToken::new("card", "oklch(1 0 0)"),
        ThemeToken::new("card-foreground", "oklch(0.145 0 0)"),
        ThemeToken::new("popover", "oklch(1 0 0)"),
        ThemeToken::new("popover-foreground", "oklch(0.145 0 0)"),
        ThemeToken::new("primary", "oklch(0.205 0 0)"),
        ThemeToken::new("primary-foreground", "oklch(0.985 0 0)"),
        ThemeToken::new("secondary", "oklch(0.97 0 0)"),
        ThemeToken::new("secondary-foreground", "oklch(0.205 0 0)"),
        ThemeToken::new("muted", "oklch(0.97 0 0)"),
        ThemeToken::new("muted-foreground", "oklch(0.556 0 0)"),
        ThemeToken::new("accent", "oklch(0.97 0 0)"),
        ThemeToken::new("accent-foreground", "oklch(0.205 0 0)"),
        ThemeToken::new("destructive", "oklch(0.5757 0.2352 27.92)"),
        ThemeToken::new("destructive-foreground", "oklch(0.985 0 0)"),
        ThemeToken::new("border", "oklch(0.922 0 0)"),
        ThemeToken::new("input", "oklch(0.922 0 0)"),
        ThemeToken::new("ring", "oklch(0.708 0 0)"),
        ThemeToken::new("chart-1", "oklch(0.646 0.222 41.116)"),
        ThemeToken::new("chart-2", "oklch(0.6 0.118 184.704)"),
        ThemeToken::new("chart-3", "oklch(0.398 0.07 227.392)"),
        ThemeToken::new("chart-4", "oklch(0.828 0.189 84.429)"),
        ThemeToken::new("chart-5", "oklch(0.769 0.188 70.08)"),
        ThemeToken::new("sidebar", "oklch(0.985 0 0)"),
        ThemeToken::new("sidebar-foreground", "oklch(0.145 0 0)"),
        ThemeToken::new("sidebar-primary", "oklch(0.205 0 0)"),
        ThemeToken::new("sidebar-primary-foreground", "oklch(0.985 0 0)"),
        ThemeToken::new("sidebar-accent", "oklch(0.97 0 0)"),
        ThemeToken::new("sidebar-accent-foreground", "oklch(0.205 0 0)"),
        ThemeToken::new("sidebar-border", "oklch(0.922 0 0)"),
        ThemeToken::new("sidebar-ring", "oklch(0.708 0 0)"),
        ThemeToken::new("font-sans", "ui-sans-serif, system-ui, sans-serif"),
        ThemeToken::new("font-serif", "ui-serif, Georgia, serif"),
        ThemeToken::new(
            "font-mono",
            "ui-monospace, SFMono-Regular, Menlo, monospace",
        ),
        ThemeToken::new("radius", "0.625rem"),
        ThemeToken::new("spacing", "0.25rem"),
        ThemeToken::new("letter-spacing", "0"),
        ThemeToken::new("shadow-color", "oklch(0 0 0)"),
        ThemeToken::new("shadow-opacity", "0.1"),
        ThemeToken::new("shadow-blur", "3px"),
        ThemeToken::new("shadow-spread", "0px"),
        ThemeToken::new("shadow-offset-x", "0px"),
        ThemeToken::new("shadow-offset-y", "1px"),
    ])
}

fn default_dark_style() -> ThemeStyle {
    default_light_style()
        .with("background", "oklch(0.145 0 0)")
        .with("foreground", "oklch(0.985 0 0)")
        .with("card", "oklch(0.145 0 0)")
        .with("card-foreground", "oklch(0.985 0 0)")
        .with("popover", "oklch(0.145 0 0)")
        .with("popover-foreground", "oklch(0.985 0 0)")
        .with("primary", "oklch(0.985 0 0)")
        .with("primary-foreground", "oklch(0.205 0 0)")
        .with("secondary", "oklch(0.269 0 0)")
        .with("secondary-foreground", "oklch(0.985 0 0)")
        .with("muted", "oklch(0.269 0 0)")
        .with("muted-foreground", "oklch(0.708 0 0)")
        .with("accent", "oklch(0.269 0 0)")
        .with("accent-foreground", "oklch(0.985 0 0)")
        .with("destructive", "oklch(0.5058 0.2066 27.85)")
        .with("border", "oklch(0.269 0 0)")
        .with("input", "oklch(0.269 0 0)")
        .with("ring", "oklch(0.439 0 0)")
        .with("sidebar", "oklch(0.205 0 0)")
        .with("sidebar-foreground", "oklch(0.985 0 0)")
        .with("sidebar-accent", "oklch(0.269 0 0)")
        .with("sidebar-accent-foreground", "oklch(0.985 0 0)")
        .with("sidebar-border", "oklch(0.269 0 0)")
        .with("sidebar-ring", "oklch(0.439 0 0)")
}

fn preset_from_pairs(
    name: &str,
    label: &str,
    light: &[(&str, &str)],
    dark: &[(&str, &str)],
) -> ThemePreset {
    ThemePreset {
        name: name.to_string(),
        label: label.to_string(),
        styles: ThemeStyles {
            light: style_from_pairs(light),
            dark: style_from_pairs(dark),
        },
    }
}

fn style_from_pairs(tokens: &[(&str, &str)]) -> ThemeStyle {
    ThemeStyle::new(
        tokens
            .iter()
            .map(|(key, value)| ThemeToken::new(*key, *value)),
    )
}

// Complete presets ported from jnsahaj/tweakcn's defaultPresets registry.

fn modern_minimal_theme_preset() -> ThemePreset {
    preset_from_pairs(
        "modern-minimal",
        "Modern Minimal",
        &[
            ("background", "#ffffff"),
            ("foreground", "#333333"),
            ("card", "#ffffff"),
            ("card-foreground", "#333333"),
            ("popover", "#ffffff"),
            ("popover-foreground", "#333333"),
            ("primary", "#3b82f6"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#f3f4f6"),
            ("secondary-foreground", "#4b5563"),
            ("muted", "#f9fafb"),
            ("muted-foreground", "#6b7280"),
            ("accent", "#e0f2fe"),
            ("accent-foreground", "#1e3a8a"),
            ("destructive", "#ef4444"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#e5e7eb"),
            ("input", "#e5e7eb"),
            ("ring", "#3b82f6"),
            ("chart-1", "#3b82f6"),
            ("chart-2", "#2563eb"),
            ("chart-3", "#1d4ed8"),
            ("chart-4", "#1e40af"),
            ("chart-5", "#1e3a8a"),
            ("radius", "0.375rem"),
            ("sidebar", "#f9fafb"),
            ("sidebar-foreground", "#333333"),
            ("sidebar-primary", "#3b82f6"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#e0f2fe"),
            ("sidebar-accent-foreground", "#1e3a8a"),
            ("sidebar-border", "#e5e7eb"),
            ("sidebar-ring", "#3b82f6"),
            ("font-sans", "Inter, sans-serif"),
            ("font-serif", "Source Serif 4, serif"),
            ("font-mono", "JetBrains Mono, monospace"),
        ],
        &[
            ("background", "#171717"),
            ("foreground", "#e5e5e5"),
            ("card", "#262626"),
            ("card-foreground", "#e5e5e5"),
            ("popover", "#262626"),
            ("popover-foreground", "#e5e5e5"),
            ("primary", "#3b82f6"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#262626"),
            ("secondary-foreground", "#e5e5e5"),
            ("muted", "#1f1f1f"),
            ("muted-foreground", "#a3a3a3"),
            ("accent", "#1e3a8a"),
            ("accent-foreground", "#bfdbfe"),
            ("destructive", "#ef4444"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#404040"),
            ("input", "#404040"),
            ("ring", "#3b82f6"),
            ("chart-1", "#60a5fa"),
            ("chart-2", "#3b82f6"),
            ("chart-3", "#2563eb"),
            ("chart-4", "#1d4ed8"),
            ("chart-5", "#1e40af"),
            ("radius", "0.375rem"),
            ("sidebar", "#171717"),
            ("sidebar-foreground", "#e5e5e5"),
            ("sidebar-primary", "#3b82f6"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#1e3a8a"),
            ("sidebar-accent-foreground", "#bfdbfe"),
            ("sidebar-border", "#404040"),
            ("sidebar-ring", "#3b82f6"),
        ],
    )
}

fn violet_bloom_theme_preset() -> ThemePreset {
    preset_from_pairs(
        "violet-bloom",
        "Violet Bloom",
        &[
            ("background", "#fdfdfd"),
            ("foreground", "#000000"),
            ("card", "#fdfdfd"),
            ("card-foreground", "#000000"),
            ("popover", "#fcfcfc"),
            ("popover-foreground", "#000000"),
            ("primary", "#7033ff"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#edf0f4"),
            ("secondary-foreground", "#080808"),
            ("muted", "#f5f5f5"),
            ("muted-foreground", "#525252"),
            ("accent", "#e2ebff"),
            ("accent-foreground", "#1e69dc"),
            ("destructive", "#e54b4f"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#e7e7ee"),
            ("input", "#ebebeb"),
            ("ring", "#000000"),
            ("chart-1", "#4ac885"),
            ("chart-2", "#7033ff"),
            ("chart-3", "#fd822b"),
            ("chart-4", "#3276e4"),
            ("chart-5", "#747474"),
            ("sidebar", "#f5f8fb"),
            ("sidebar-foreground", "#000000"),
            ("sidebar-primary", "#000000"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#ebebeb"),
            ("sidebar-accent-foreground", "#000000"),
            ("sidebar-border", "#ebebeb"),
            ("sidebar-ring", "#000000"),
            ("font-sans", "Plus Jakarta Sans, sans-serif"),
            ("font-serif", "Lora, serif"),
            ("font-mono", "IBM Plex Mono, monospace"),
            ("radius", "1.4rem"),
            ("shadow-color", "hsl(0 0% 0%)"),
            ("shadow-opacity", "0.16"),
            ("shadow-blur", "3px"),
            ("shadow-spread", "0px"),
            ("shadow-offset-x", "0px"),
            ("shadow-offset-y", "2px"),
            ("letter-spacing", "-0.025em"),
            ("spacing", "0.27rem"),
        ],
        &[
            ("background", "#1a1b1e"),
            ("foreground", "#f0f0f0"),
            ("card", "#222327"),
            ("card-foreground", "#f0f0f0"),
            ("popover", "#222327"),
            ("popover-foreground", "#f0f0f0"),
            ("primary", "#8c5cff"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#2a2c33"),
            ("secondary-foreground", "#f0f0f0"),
            ("muted", "#2a2c33"),
            ("muted-foreground", "#a0a0a0"),
            ("accent", "#1e293b"),
            ("accent-foreground", "#79c0ff"),
            ("destructive", "#f87171"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#33353a"),
            ("input", "#33353a"),
            ("ring", "#8c5cff"),
            ("chart-1", "#4ade80"),
            ("chart-2", "#8c5cff"),
            ("chart-3", "#fca5a5"),
            ("chart-4", "#5993f4"),
            ("chart-5", "#a0a0a0"),
            ("sidebar", "#161618"),
            ("sidebar-foreground", "#f0f0f0"),
            ("sidebar-primary", "#8c5cff"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#2a2c33"),
            ("sidebar-accent-foreground", "#8c5cff"),
            ("sidebar-border", "#33353a"),
            ("sidebar-ring", "#8c5cff"),
            ("font-sans", "Plus Jakarta Sans, sans-serif"),
            ("font-serif", "Lora, serif"),
            ("font-mono", "IBM Plex Mono, monospace"),
            ("radius", "1.4rem"),
            ("shadow-color", "hsl(0 0% 0%)"),
            ("shadow-opacity", "0.16"),
            ("shadow-blur", "3px"),
            ("shadow-spread", "0px"),
            ("shadow-offset-x", "0px"),
            ("shadow-offset-y", "2px"),
            ("letter-spacing", "-0.025em"),
            ("spacing", "0.27rem"),
        ],
    )
}

fn t3_chat_theme_preset() -> ThemePreset {
    preset_from_pairs(
        "t3-chat",
        "T3 Chat",
        &[
            ("background", "#faf5fa"),
            ("foreground", "#501854"),
            ("card", "#faf5fa"),
            ("card-foreground", "#501854"),
            ("popover", "#ffffff"),
            ("popover-foreground", "#501854"),
            ("primary", "#a84370"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#f1c4e6"),
            ("secondary-foreground", "#77347c"),
            ("muted", "#f6e5f3"),
            ("muted-foreground", "#834588"),
            ("accent", "#f1c4e6"),
            ("accent-foreground", "#77347c"),
            ("destructive", "#ab4347"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#efbdeb"),
            ("input", "#e7c1dc"),
            ("ring", "#db2777"),
            ("chart-1", "#d926a2"),
            ("chart-2", "#6c12b9"),
            ("chart-3", "#274754"),
            ("chart-4", "#e8c468"),
            ("chart-5", "#f4a462"),
            ("sidebar", "#f3e4f6"),
            ("sidebar-foreground", "#ac1668"),
            ("sidebar-primary", "#454554"),
            ("sidebar-primary-foreground", "#faf1f7"),
            ("sidebar-accent", "#f8f8f7"),
            ("sidebar-accent-foreground", "#454554"),
            ("sidebar-border", "#eceae9"),
            ("sidebar-ring", "#db2777"),
            ("radius", "0.5rem"),
        ],
        &[
            ("background", "#221d27"),
            ("foreground", "#d2c4de"),
            ("card", "#2c2632"),
            ("card-foreground", "#dbc5d2"),
            ("popover", "#100a0e"),
            ("popover-foreground", "#f8f1f5"),
            ("primary", "#a3004c"),
            ("primary-foreground", "#efc0d8"),
            ("secondary", "#362d3d"),
            ("secondary-foreground", "#d4c7e1"),
            ("muted", "#28222d"),
            ("muted-foreground", "#c2b6cf"),
            ("accent", "#463753"),
            ("accent-foreground", "#f8f1f5"),
            ("destructive", "#301015"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#3b3237"),
            ("input", "#3e343c"),
            ("ring", "#db2777"),
            ("chart-1", "#a84370"),
            ("chart-2", "#934dcb"),
            ("chart-3", "#e88c30"),
            ("chart-4", "#af57db"),
            ("chart-5", "#e23670"),
            ("sidebar", "#181117"),
            ("sidebar-foreground", "#e0cad6"),
            ("sidebar-primary", "#1d4ed8"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#261922"),
            ("sidebar-accent-foreground", "#f4f4f5"),
            ("sidebar-border", "#000000"),
            ("sidebar-ring", "#db2777"),
        ],
    )
}

fn amethyst_haze_theme_preset() -> ThemePreset {
    preset_from_pairs(
        "amethyst-haze",
        "Amethyst Haze",
        &[
            ("background", "#f8f7fa"),
            ("foreground", "#3d3c4f"),
            ("card", "#ffffff"),
            ("card-foreground", "#3d3c4f"),
            ("popover", "#ffffff"),
            ("popover-foreground", "#3d3c4f"),
            ("primary", "#8a79ab"),
            ("primary-foreground", "#f8f7fa"),
            ("secondary", "#dfd9ec"),
            ("secondary-foreground", "#3d3c4f"),
            ("muted", "#dcd9e3"),
            ("muted-foreground", "#6b6880"),
            ("accent", "#e6a5b8"),
            ("accent-foreground", "#4b2e36"),
            ("destructive", "#d95c5c"),
            ("destructive-foreground", "#f8f7fa"),
            ("border", "#cec9d9"),
            ("input", "#eae7f0"),
            ("ring", "#8a79ab"),
            ("chart-1", "#8a79ab"),
            ("chart-2", "#e6a5b8"),
            ("chart-3", "#77b8a1"),
            ("chart-4", "#f0c88d"),
            ("chart-5", "#a0bbe3"),
            ("sidebar", "#f1eff5"),
            ("sidebar-foreground", "#3d3c4f"),
            ("sidebar-primary", "#8a79ab"),
            ("sidebar-primary-foreground", "#f8f7fa"),
            ("sidebar-accent", "#e6a5b8"),
            ("sidebar-accent-foreground", "#4b2e36"),
            ("sidebar-border", "#d7d2e0"),
            ("sidebar-ring", "#8a79ab"),
            ("font-sans", "Geist, sans-serif"),
            ("font-serif", "\"Lora\", Georgia, serif"),
            ("font-mono", "\"Fira Code\", \"Courier New\", monospace"),
            ("radius", "0.5rem"),
            ("shadow-color", "hsl(0 0% 0%)"),
            ("shadow-opacity", "0.06"),
            ("shadow-blur", "5px"),
            ("shadow-spread", "1px"),
            ("shadow-offset-x", "1px"),
            ("shadow-offset-y", "2px"),
            ("letter-spacing", "0em"),
            ("spacing", "0.25rem"),
        ],
        &[
            ("background", "#1a1823"),
            ("foreground", "#e0ddef"),
            ("card", "#232030"),
            ("card-foreground", "#e0ddef"),
            ("popover", "#232030"),
            ("popover-foreground", "#e0ddef"),
            ("primary", "#a995c9"),
            ("primary-foreground", "#1a1823"),
            ("secondary", "#5a5370"),
            ("secondary-foreground", "#e0ddef"),
            ("muted", "#242031"),
            ("muted-foreground", "#a09aad"),
            ("accent", "#372e3f"),
            ("accent-foreground", "#f2b8c6"),
            ("destructive", "#e57373"),
            ("destructive-foreground", "#1a1823"),
            ("border", "#302c40"),
            ("input", "#2a273a"),
            ("ring", "#a995c9"),
            ("chart-1", "#a995c9"),
            ("chart-2", "#f2b8c6"),
            ("chart-3", "#77b8a1"),
            ("chart-4", "#f0c88d"),
            ("chart-5", "#a0bbe3"),
            ("sidebar", "#16141e"),
            ("sidebar-foreground", "#e0ddef"),
            ("sidebar-primary", "#a995c9"),
            ("sidebar-primary-foreground", "#1a1823"),
            ("sidebar-accent", "#372e3f"),
            ("sidebar-accent-foreground", "#f2b8c6"),
            ("sidebar-border", "#2a273a"),
            ("sidebar-ring", "#a995c9"),
        ],
    )
}

fn doom_64_theme_preset() -> ThemePreset {
    preset_from_pairs(
        "doom-64",
        "Doom 64",
        &[
            ("background", "#cccccc"),
            ("foreground", "#1f1f1f"),
            ("card", "#b0b0b0"),
            ("card-foreground", "#1f1f1f"),
            ("popover", "#b0b0b0"),
            ("popover-foreground", "#1f1f1f"),
            ("primary", "#b71c1c"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#556b2f"),
            ("secondary-foreground", "#ffffff"),
            ("muted", "#b8b8b8"),
            ("muted-foreground", "#4a4a4a"),
            ("accent", "#4682b4"),
            ("accent-foreground", "#ffffff"),
            ("destructive", "#ff6f00"),
            ("destructive-foreground", "#000000"),
            ("border", "#505050"),
            ("input", "#505050"),
            ("ring", "#b71c1c"),
            ("chart-1", "#b71c1c"),
            ("chart-2", "#556b2f"),
            ("chart-3", "#4682b4"),
            ("chart-4", "#ff6f00"),
            ("chart-5", "#8d6e63"),
            ("sidebar", "#b0b0b0"),
            ("sidebar-foreground", "#1f1f1f"),
            ("sidebar-primary", "#b71c1c"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#4682b4"),
            ("sidebar-accent-foreground", "#ffffff"),
            ("sidebar-border", "#505050"),
            ("sidebar-ring", "#b71c1c"),
            ("font-sans", "\"Oxanium\", sans-serif"),
            (
                "font-serif",
                "ui-serif, Georgia, Cambria, \"Times New Roman\", Times, serif",
            ),
            ("font-mono", "\"Source Code Pro\", monospace"),
            ("radius", "0px"),
            ("shadow-color", "hsl(0 0% 0%)"),
            ("shadow-opacity", "0.4"),
            ("shadow-blur", "4px"),
            ("shadow-spread", "0px"),
            ("shadow-offset-x", "0px"),
            ("shadow-offset-y", "2px"),
            ("letter-spacing", "0em"),
            ("spacing", "0.25rem"),
        ],
        &[
            ("background", "#1a1a1a"),
            ("foreground", "#e0e0e0"),
            ("card", "#2a2a2a"),
            ("card-foreground", "#e0e0e0"),
            ("popover", "#2a2a2a"),
            ("popover-foreground", "#e0e0e0"),
            ("primary", "#e53935"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#689f38"),
            ("secondary-foreground", "#000000"),
            ("muted", "#252525"),
            ("muted-foreground", "#a0a0a0"),
            ("accent", "#64b5f6"),
            ("accent-foreground", "#000000"),
            ("destructive", "#ffa000"),
            ("destructive-foreground", "#000000"),
            ("border", "#4a4a4a"),
            ("input", "#4a4a4a"),
            ("ring", "#e53935"),
            ("chart-1", "#e53935"),
            ("chart-2", "#689f38"),
            ("chart-3", "#64b5f6"),
            ("chart-4", "#ffa000"),
            ("chart-5", "#a1887f"),
            ("sidebar", "#141414"),
            ("sidebar-foreground", "#e0e0e0"),
            ("sidebar-primary", "#e53935"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#64b5f6"),
            ("sidebar-accent-foreground", "#000000"),
            ("sidebar-border", "#4a4a4a"),
            ("sidebar-ring", "#e53935"),
            ("font-sans", "\"Oxanium\", sans-serif"),
            (
                "font-serif",
                "ui-serif, Georgia, Cambria, \"Times New Roman\", Times, serif",
            ),
            ("font-mono", "\"Source Code Pro\", monospace"),
            ("radius", "0px"),
            ("shadow-color", "hsl(0 0% 0%)"),
            ("shadow-opacity", "0.6"),
            ("shadow-blur", "5px"),
            ("shadow-spread", "0px"),
            ("shadow-offset-x", "0px"),
            ("shadow-offset-y", "2px"),
            ("letter-spacing", "0em"),
            ("spacing", "0.25rem"),
        ],
    )
}

/// The FastTrackStudio brand preset — the fasttrackstudio.app look:
/// near-black zinc surfaces with the site's violet-indigo accent
/// (hero gradient oklch(0.72 0.18 264) → oklch(0.78 0.16 320)).
/// Dark is the canonical mode; light keeps the indigo accent on
/// plain zinc so both read as the same brand.
fn fasttrackstudio_theme_preset() -> ThemePreset {
    preset_from_pairs(
        "fasttrackstudio",
        "FastTrackStudio",
        &[
            ("background", "#fafafa"),
            ("foreground", "#18181b"),
            ("card", "#ffffff"),
            ("card-foreground", "#18181b"),
            ("popover", "#ffffff"),
            ("popover-foreground", "#18181b"),
            ("primary", "#6366f1"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#f4f4f5"),
            ("secondary-foreground", "#18181b"),
            ("muted", "#f4f4f5"),
            ("muted-foreground", "#71717a"),
            ("accent", "#eef2ff"),
            ("accent-foreground", "#4338ca"),
            ("destructive", "#dc2626"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#e4e4e7"),
            ("input", "#e4e4e7"),
            ("ring", "#6366f1"),
            ("chart-1", "#6366f1"),
            ("chart-2", "#a78bfa"),
            ("chart-3", "#38bdf8"),
            ("chart-4", "#10b981"),
            ("chart-5", "#fbbf24"),
            ("sidebar", "#f4f4f5"),
            ("sidebar-foreground", "#18181b"),
            ("sidebar-primary", "#6366f1"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#eef2ff"),
            ("sidebar-accent-foreground", "#4338ca"),
            ("sidebar-border", "#e4e4e7"),
            ("sidebar-ring", "#6366f1"),
            ("font-sans", "Inter, ui-sans-serif, system-ui, sans-serif"),
            ("font-serif", "Georgia, serif"),
            ("font-mono", "JetBrains Mono, Fira Code, monospace"),
            ("radius", "0.625rem"),
            ("shadow-color", "hsl(244 50% 20% / 0.1)"),
            ("shadow-opacity", "0.12"),
            ("shadow-blur", "4px"),
            ("shadow-spread", "0px"),
            ("shadow-offset-x", "0px"),
            ("shadow-offset-y", "2px"),
        ],
        &[
            ("background", "#09090b"),
            ("foreground", "#e4e4e7"),
            ("card", "#111114"),
            ("card-foreground", "#e4e4e7"),
            ("popover", "#111114"),
            ("popover-foreground", "#e4e4e7"),
            ("primary", "#818cf8"),
            ("primary-foreground", "#0d0a14"),
            ("secondary", "#1c1c21"),
            ("secondary-foreground", "#e4e4e7"),
            ("muted", "#18181b"),
            ("muted-foreground", "#8f8f98"),
            ("accent", "#26243a"),
            ("accent-foreground", "#c7d2fe"),
            ("destructive", "#ef4444"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#27272a"),
            ("input", "#1c1c21"),
            ("ring", "#818cf8"),
            ("chart-1", "#818cf8"),
            ("chart-2", "#a78bfa"),
            ("chart-3", "#38bdf8"),
            ("chart-4", "#34d399"),
            ("chart-5", "#fbbf24"),
            ("sidebar", "#0d0d10"),
            ("sidebar-foreground", "#e4e4e7"),
            ("sidebar-primary", "#818cf8"),
            ("sidebar-primary-foreground", "#0d0a14"),
            ("sidebar-accent", "#26243a"),
            ("sidebar-accent-foreground", "#c7d2fe"),
            ("sidebar-border", "#27272a"),
            ("sidebar-ring", "#818cf8"),
            ("font-sans", "Inter, ui-sans-serif, system-ui, sans-serif"),
            ("font-serif", "Georgia, serif"),
            ("font-mono", "JetBrains Mono, Fira Code, monospace"),
        ],
    )
}

fn graphite_theme_preset() -> ThemePreset {
    preset_from_pairs(
        "graphite",
        "Graphite",
        &[
            ("background", "#f0f0f0"),
            ("foreground", "#333333"),
            ("card", "#f5f5f5"),
            ("card-foreground", "#333333"),
            ("popover", "#f5f5f5"),
            ("popover-foreground", "#333333"),
            ("primary", "#606060"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#e0e0e0"),
            ("secondary-foreground", "#333333"),
            ("muted", "#d9d9d9"),
            ("muted-foreground", "#666666"),
            ("accent", "#c0c0c0"),
            ("accent-foreground", "#333333"),
            ("destructive", "#cc3333"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#d0d0d0"),
            ("input", "#e0e0e0"),
            ("ring", "#606060"),
            ("chart-1", "#606060"),
            ("chart-2", "#476666"),
            ("chart-3", "#909090"),
            ("chart-4", "#a8a8a8"),
            ("chart-5", "#c0c0c0"),
            ("sidebar", "#eaeaea"),
            ("sidebar-foreground", "#333333"),
            ("sidebar-primary", "#606060"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#c0c0c0"),
            ("sidebar-accent-foreground", "#333333"),
            ("sidebar-border", "#d0d0d0"),
            ("sidebar-ring", "#606060"),
            ("font-sans", "Montserrat, sans-serif"),
            ("font-serif", "Georgia, serif"),
            ("font-mono", "Fira Code, monospace"),
            ("radius", "0.35rem"),
            ("shadow-color", "hsl(0 0% 20% / 0.1)"),
            ("shadow-opacity", "0.15"),
            ("shadow-blur", "0px"),
            ("shadow-spread", "0px"),
            ("shadow-offset-x", "0px"),
            ("shadow-offset-y", "2px"),
        ],
        &[
            ("background", "#1a1a1a"),
            ("foreground", "#d9d9d9"),
            ("card", "#202020"),
            ("card-foreground", "#d9d9d9"),
            ("popover", "#202020"),
            ("popover-foreground", "#d9d9d9"),
            ("primary", "#a0a0a0"),
            ("primary-foreground", "#1a1a1a"),
            ("secondary", "#303030"),
            ("secondary-foreground", "#d9d9d9"),
            ("muted", "#2a2a2a"),
            ("muted-foreground", "#808080"),
            ("accent", "#404040"),
            ("accent-foreground", "#d9d9d9"),
            ("destructive", "#e06666"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#353535"),
            ("input", "#303030"),
            ("ring", "#a0a0a0"),
            ("chart-1", "#a0a0a0"),
            ("chart-2", "#7e9ca0"),
            ("chart-3", "#707070"),
            ("chart-4", "#585858"),
            ("chart-5", "#404040"),
            ("sidebar", "#1f1f1f"),
            ("sidebar-foreground", "#d9d9d9"),
            ("sidebar-primary", "#a0a0a0"),
            ("sidebar-primary-foreground", "#1a1a1a"),
            ("sidebar-accent", "#404040"),
            ("sidebar-accent-foreground", "#d9d9d9"),
            ("sidebar-border", "#353535"),
            ("sidebar-ring", "#a0a0a0"),
            ("font-sans", "Inter, sans-serif"),
            ("font-serif", "Georgia, serif"),
            ("font-mono", "Fira Code, monospace"),
        ],
    )
}

fn supabase_theme_preset() -> ThemePreset {
    preset_from_pairs(
        "supabase",
        "Supabase",
        &[
            ("background", "#fcfcfc"),
            ("foreground", "#171717"),
            ("card", "#fcfcfc"),
            ("card-foreground", "#171717"),
            ("popover", "#fcfcfc"),
            ("popover-foreground", "#525252"),
            ("primary", "#72e3ad"),
            ("primary-foreground", "#1e2723"),
            ("secondary", "#fdfdfd"),
            ("secondary-foreground", "#171717"),
            ("muted", "#ededed"),
            ("muted-foreground", "#202020"),
            ("accent", "#ededed"),
            ("accent-foreground", "#202020"),
            ("destructive", "#ca3214"),
            ("destructive-foreground", "#fffcfc"),
            ("border", "#dfdfdf"),
            ("input", "#f6f6f6"),
            ("ring", "#72e3ad"),
            ("chart-1", "#72e3ad"),
            ("chart-2", "#3b82f6"),
            ("chart-3", "#8b5cf6"),
            ("chart-4", "#f59e0b"),
            ("chart-5", "#10b981"),
            ("sidebar", "#fcfcfc"),
            ("sidebar-foreground", "#707070"),
            ("sidebar-primary", "#72e3ad"),
            ("sidebar-primary-foreground", "#1e2723"),
            ("sidebar-accent", "#ededed"),
            ("sidebar-accent-foreground", "#202020"),
            ("sidebar-border", "#dfdfdf"),
            ("sidebar-ring", "#72e3ad"),
            ("font-sans", "Outfit, sans-serif"),
            (
                "font-serif",
                "ui-serif, Georgia, Cambria, \"Times New Roman\", Times, serif",
            ),
            ("font-mono", "monospace"),
            ("radius", "0.5rem"),
            ("shadow-color", "#000000"),
            ("shadow-opacity", "0.17"),
            ("shadow-blur", "3px"),
            ("shadow-spread", "0px"),
            ("shadow-offset-x", "0px"),
            ("shadow-offset-y", "1px"),
            ("letter-spacing", "0.025em"),
        ],
        &[
            ("background", "#121212"),
            ("foreground", "#e2e8f0"),
            ("card", "#171717"),
            ("card-foreground", "#e2e8f0"),
            ("popover", "#242424"),
            ("popover-foreground", "#a9a9a9"),
            ("primary", "#006239"),
            ("primary-foreground", "#dde8e3"),
            ("secondary", "#242424"),
            ("secondary-foreground", "#fafafa"),
            ("muted", "#1f1f1f"),
            ("muted-foreground", "#a2a2a2"),
            ("accent", "#313131"),
            ("accent-foreground", "#fafafa"),
            ("destructive", "#541c15"),
            ("destructive-foreground", "#ede9e8"),
            ("border", "#292929"),
            ("input", "#242424"),
            ("ring", "#4ade80"),
            ("chart-1", "#4ade80"),
            ("chart-2", "#60a5fa"),
            ("chart-3", "#a78bfa"),
            ("chart-4", "#fbbf24"),
            ("chart-5", "#2dd4bf"),
            ("sidebar", "#121212"),
            ("sidebar-foreground", "#898989"),
            ("sidebar-primary", "#006239"),
            ("sidebar-primary-foreground", "#dde8e3"),
            ("sidebar-accent", "#313131"),
            ("sidebar-accent-foreground", "#fafafa"),
            ("sidebar-border", "#292929"),
            ("sidebar-ring", "#4ade80"),
        ],
    )
}

fn neo_brutalism_theme_preset() -> ThemePreset {
    preset_from_pairs(
        "neo-brutalism",
        "Neo Brutalism",
        &[
            ("background", "#ffffff"),
            ("foreground", "#000000"),
            ("card", "#ffffff"),
            ("card-foreground", "#000000"),
            ("popover", "#ffffff"),
            ("popover-foreground", "#000000"),
            ("primary", "#ff3333"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#ffff00"),
            ("secondary-foreground", "#000000"),
            ("muted", "#f0f0f0"),
            ("muted-foreground", "#333333"),
            ("accent", "#0066ff"),
            ("accent-foreground", "#ffffff"),
            ("destructive", "#000000"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#000000"),
            ("input", "#000000"),
            ("ring", "#ff3333"),
            ("chart-1", "#ff3333"),
            ("chart-2", "#ffff00"),
            ("chart-3", "#0066ff"),
            ("chart-4", "#00cc00"),
            ("chart-5", "#cc00cc"),
            ("radius", "0px"),
            ("sidebar", "#f0f0f0"),
            ("sidebar-foreground", "#000000"),
            ("sidebar-primary", "#ff3333"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#0066ff"),
            ("sidebar-accent-foreground", "#ffffff"),
            ("sidebar-border", "#000000"),
            ("sidebar-ring", "#ff3333"),
            ("font-sans", "DM Sans, sans-serif"),
            ("font-mono", "Space Mono, monospace"),
            ("shadow-color", "hsl(0 0% 0%)"),
            ("shadow-opacity", "1"),
            ("shadow-blur", "0px"),
            ("shadow-spread", "0px"),
            ("shadow-offset-x", "4px"),
            ("shadow-offset-y", "4px"),
        ],
        &[
            ("background", "#000000"),
            ("foreground", "#ffffff"),
            ("card", "#333333"),
            ("card-foreground", "#ffffff"),
            ("popover", "#333333"),
            ("popover-foreground", "#ffffff"),
            ("primary", "#ff6666"),
            ("primary-foreground", "#000000"),
            ("secondary", "#ffff33"),
            ("secondary-foreground", "#000000"),
            ("muted", "#1a1a1a"),
            ("muted-foreground", "#cccccc"),
            ("accent", "#3399ff"),
            ("accent-foreground", "#000000"),
            ("destructive", "#ffffff"),
            ("destructive-foreground", "#000000"),
            ("border", "#ffffff"),
            ("input", "#ffffff"),
            ("ring", "#ff6666"),
            ("chart-1", "#ff6666"),
            ("chart-2", "#ffff33"),
            ("chart-3", "#3399ff"),
            ("chart-4", "#33cc33"),
            ("chart-5", "#cc33cc"),
            ("radius", "0px"),
            ("sidebar", "#000000"),
            ("sidebar-foreground", "#ffffff"),
            ("sidebar-primary", "#ff6666"),
            ("sidebar-primary-foreground", "#000000"),
            ("sidebar-accent", "#3399ff"),
            ("sidebar-accent-foreground", "#000000"),
            ("sidebar-border", "#ffffff"),
            ("sidebar-ring", "#ff6666"),
        ],
    )
}

// Community color-scheme presets, ported from each scheme's published
// palette (catppuccin.com, nordtheme.com, draculatheme.com, gruvbox,
// ethanschoonover.com/solarized, tokyonight.nvim, rosepinetheme.com,
// everforest) plus two house presets (Zen, Obsidian). Dark-only schemes
// use the scheme's official light counterpart where one exists
// (gruvbox-light, solarized-light, rose-pine-dawn, tokyo-night-day,
// everforest-light); Dracula's light half is a tasteful derivation.
// Every block covers the full default_light_style key set so switching
// presets never leaves a stale token behind.

const STACK_SANS: &str = "ui-sans-serif, system-ui, sans-serif";
const STACK_SERIF: &str = "ui-serif, Georgia, serif";
const STACK_MONO: &str = "ui-monospace, SFMono-Regular, Menlo, monospace";

/// The non-color keys shared by the community presets — appended to
/// each color block so the full default_light_style key set is covered.
const PRESET_BASE_TOKENS: &[(&str, &str)] = &[
    ("font-sans", STACK_SANS),
    ("font-serif", STACK_SERIF),
    ("font-mono", STACK_MONO),
    ("radius", "0.5rem"),
    ("spacing", "0.25rem"),
    ("letter-spacing", "0"),
    ("shadow-color", "hsl(0 0% 0%)"),
    ("shadow-opacity", "0.1"),
    ("shadow-blur", "3px"),
    ("shadow-spread", "0px"),
    ("shadow-offset-x", "0px"),
    ("shadow-offset-y", "1px"),
];

/// `preset_from_pairs`, with [`PRESET_BASE_TOKENS`] appended to both
/// modes first so a preset's own pairs win on any key it restates.
fn preset_from_palette(
    name: &str,
    label: &str,
    light: &[(&str, &str)],
    dark: &[(&str, &str)],
) -> ThemePreset {
    let merge = |pairs: &[(&str, &str)]| {
        let mut style = style_from_pairs(PRESET_BASE_TOKENS);
        for (key, value) in pairs {
            style.set(*key, *value);
        }
        style
    };

    ThemePreset {
        name: name.to_string(),
        label: label.to_string(),
        styles: ThemeStyles {
            light: merge(light),
            dark: merge(dark),
        },
    }
}

/// Catppuccin — light = Latte, dark = Mocha. Mauve is the accent.
fn catppuccin_theme_preset() -> ThemePreset {
    preset_from_palette(
        "catppuccin",
        "Catppuccin",
        &[
            ("background", "#eff1f5"),
            ("foreground", "#4c4f69"),
            ("card", "#e6e9ef"),
            ("card-foreground", "#4c4f69"),
            ("popover", "#e6e9ef"),
            ("popover-foreground", "#4c4f69"),
            ("primary", "#8839ef"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#ccd0da"),
            ("secondary-foreground", "#4c4f69"),
            ("muted", "#ccd0da"),
            ("muted-foreground", "#6c6f85"),
            ("accent", "#ccd0da"),
            ("accent-foreground", "#4c4f69"),
            ("destructive", "#d20f39"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#bcc0cc"),
            ("input", "#ccd0da"),
            ("ring", "#8839ef"),
            ("chart-1", "#8839ef"),
            ("chart-2", "#1e66f5"),
            ("chart-3", "#40a02b"),
            ("chart-4", "#df8e1d"),
            ("chart-5", "#ea76cb"),
            ("sidebar", "#e6e9ef"),
            ("sidebar-foreground", "#4c4f69"),
            ("sidebar-primary", "#8839ef"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#ccd0da"),
            ("sidebar-accent-foreground", "#4c4f69"),
            ("sidebar-border", "#bcc0cc"),
            ("sidebar-ring", "#8839ef"),
        ],
        &[
            ("background", "#1e1e2e"),
            ("foreground", "#cdd6f4"),
            ("card", "#181825"),
            ("card-foreground", "#cdd6f4"),
            ("popover", "#181825"),
            ("popover-foreground", "#cdd6f4"),
            ("primary", "#cba6f7"),
            ("primary-foreground", "#1e1e2e"),
            ("secondary", "#313244"),
            ("secondary-foreground", "#cdd6f4"),
            ("muted", "#313244"),
            ("muted-foreground", "#a6adc8"),
            ("accent", "#313244"),
            ("accent-foreground", "#cdd6f4"),
            ("destructive", "#f38ba8"),
            ("destructive-foreground", "#1e1e2e"),
            ("border", "#45475a"),
            ("input", "#313244"),
            ("ring", "#cba6f7"),
            ("chart-1", "#cba6f7"),
            ("chart-2", "#89b4fa"),
            ("chart-3", "#a6e3a1"),
            ("chart-4", "#f9e2af"),
            ("chart-5", "#f5c2e7"),
            ("sidebar", "#181825"),
            ("sidebar-foreground", "#cdd6f4"),
            ("sidebar-primary", "#cba6f7"),
            ("sidebar-primary-foreground", "#1e1e2e"),
            ("sidebar-accent", "#313244"),
            ("sidebar-accent-foreground", "#cdd6f4"),
            ("sidebar-border", "#45475a"),
            ("sidebar-ring", "#cba6f7"),
        ],
    )
}

/// Nord — light = Snow Storm surfaces, dark = Polar Night. Frost blues
/// as accents (nord10 in light, nord8 in dark).
fn nord_theme_preset() -> ThemePreset {
    preset_from_palette(
        "nord",
        "Nord",
        &[
            ("background", "#eceff4"),
            ("foreground", "#2e3440"),
            ("card", "#e5e9f0"),
            ("card-foreground", "#2e3440"),
            ("popover", "#e5e9f0"),
            ("popover-foreground", "#2e3440"),
            ("primary", "#5e81ac"),
            ("primary-foreground", "#eceff4"),
            ("secondary", "#d8dee9"),
            ("secondary-foreground", "#2e3440"),
            ("muted", "#d8dee9"),
            ("muted-foreground", "#4c566a"),
            ("accent", "#d8dee9"),
            ("accent-foreground", "#2e3440"),
            ("destructive", "#bf616a"),
            ("destructive-foreground", "#eceff4"),
            ("border", "#d8dee9"),
            ("input", "#d8dee9"),
            ("ring", "#5e81ac"),
            ("chart-1", "#5e81ac"),
            ("chart-2", "#88c0d0"),
            ("chart-3", "#a3be8c"),
            ("chart-4", "#ebcb8b"),
            ("chart-5", "#b48ead"),
            ("sidebar", "#e5e9f0"),
            ("sidebar-foreground", "#2e3440"),
            ("sidebar-primary", "#5e81ac"),
            ("sidebar-primary-foreground", "#eceff4"),
            ("sidebar-accent", "#d8dee9"),
            ("sidebar-accent-foreground", "#2e3440"),
            ("sidebar-border", "#d8dee9"),
            ("sidebar-ring", "#5e81ac"),
        ],
        &[
            ("background", "#2e3440"),
            ("foreground", "#eceff4"),
            ("card", "#3b4252"),
            ("card-foreground", "#eceff4"),
            ("popover", "#3b4252"),
            ("popover-foreground", "#eceff4"),
            ("primary", "#88c0d0"),
            ("primary-foreground", "#2e3440"),
            ("secondary", "#434c5e"),
            ("secondary-foreground", "#eceff4"),
            ("muted", "#3b4252"),
            ("muted-foreground", "#94a3b8"),
            ("accent", "#434c5e"),
            ("accent-foreground", "#eceff4"),
            ("destructive", "#bf616a"),
            ("destructive-foreground", "#eceff4"),
            ("border", "#434c5e"),
            ("input", "#434c5e"),
            ("ring", "#88c0d0"),
            ("chart-1", "#88c0d0"),
            ("chart-2", "#81a1c1"),
            ("chart-3", "#a3be8c"),
            ("chart-4", "#ebcb8b"),
            ("chart-5", "#b48ead"),
            ("sidebar", "#3b4252"),
            ("sidebar-foreground", "#eceff4"),
            ("sidebar-primary", "#88c0d0"),
            ("sidebar-primary-foreground", "#2e3440"),
            ("sidebar-accent", "#434c5e"),
            ("sidebar-accent-foreground", "#eceff4"),
            ("sidebar-border", "#434c5e"),
            ("sidebar-ring", "#88c0d0"),
        ],
    )
}

/// Dracula — dark is the canonical palette; the light half is a derived
/// counterpart (Dracula ships none) keeping the purple/comment hues.
fn dracula_theme_preset() -> ThemePreset {
    preset_from_palette(
        "dracula",
        "Dracula",
        &[
            ("background", "#f8f8f2"),
            ("foreground", "#282a36"),
            ("card", "#ffffff"),
            ("card-foreground", "#282a36"),
            ("popover", "#ffffff"),
            ("popover-foreground", "#282a36"),
            ("primary", "#6f42c1"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#e6e6f0"),
            ("secondary-foreground", "#282a36"),
            ("muted", "#e6e6f0"),
            ("muted-foreground", "#6272a4"),
            ("accent", "#e6e6f0"),
            ("accent-foreground", "#282a36"),
            ("destructive", "#dc2626"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#d6d6e7"),
            ("input", "#d6d6e7"),
            ("ring", "#6f42c1"),
            ("chart-1", "#6f42c1"),
            ("chart-2", "#0e7490"),
            ("chart-3", "#15803d"),
            ("chart-4", "#c2410c"),
            ("chart-5", "#be185d"),
            ("sidebar", "#efeff9"),
            ("sidebar-foreground", "#282a36"),
            ("sidebar-primary", "#6f42c1"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#e6e6f0"),
            ("sidebar-accent-foreground", "#282a36"),
            ("sidebar-border", "#d6d6e7"),
            ("sidebar-ring", "#6f42c1"),
        ],
        &[
            ("background", "#282a36"),
            ("foreground", "#f8f8f2"),
            ("card", "#21222c"),
            ("card-foreground", "#f8f8f2"),
            ("popover", "#21222c"),
            ("popover-foreground", "#f8f8f2"),
            ("primary", "#bd93f9"),
            ("primary-foreground", "#282a36"),
            ("secondary", "#44475a"),
            ("secondary-foreground", "#f8f8f2"),
            ("muted", "#44475a"),
            ("muted-foreground", "#6272a4"),
            ("accent", "#44475a"),
            ("accent-foreground", "#f8f8f2"),
            ("destructive", "#ff5555"),
            ("destructive-foreground", "#282a36"),
            ("border", "#44475a"),
            ("input", "#44475a"),
            ("ring", "#bd93f9"),
            ("chart-1", "#bd93f9"),
            ("chart-2", "#8be9fd"),
            ("chart-3", "#50fa7b"),
            ("chart-4", "#ffb86c"),
            ("chart-5", "#ff79c6"),
            ("sidebar", "#21222c"),
            ("sidebar-foreground", "#f8f8f2"),
            ("sidebar-primary", "#bd93f9"),
            ("sidebar-primary-foreground", "#282a36"),
            ("sidebar-accent", "#44475a"),
            ("sidebar-accent-foreground", "#f8f8f2"),
            ("sidebar-border", "#44475a"),
            ("sidebar-ring", "#bd93f9"),
        ],
    )
}

/// Gruvbox — light/dark medium variants; faded accents in light, bright
/// in dark, per the scheme's own contrast rules.
fn gruvbox_theme_preset() -> ThemePreset {
    preset_from_palette(
        "gruvbox",
        "Gruvbox",
        &[
            ("background", "#fbf1c7"),
            ("foreground", "#3c3836"),
            ("card", "#f2e5bc"),
            ("card-foreground", "#3c3836"),
            ("popover", "#f2e5bc"),
            ("popover-foreground", "#3c3836"),
            ("primary", "#b57614"),
            ("primary-foreground", "#fbf1c7"),
            ("secondary", "#ebdbb2"),
            ("secondary-foreground", "#3c3836"),
            ("muted", "#ebdbb2"),
            ("muted-foreground", "#7c6f64"),
            ("accent", "#ebdbb2"),
            ("accent-foreground", "#3c3836"),
            ("destructive", "#9d0006"),
            ("destructive-foreground", "#fbf1c7"),
            ("border", "#d5c4a1"),
            ("input", "#ebdbb2"),
            ("ring", "#b57614"),
            ("chart-1", "#b57614"),
            ("chart-2", "#076678"),
            ("chart-3", "#79740e"),
            ("chart-4", "#af3a03"),
            ("chart-5", "#8f3f71"),
            ("sidebar", "#f2e5bc"),
            ("sidebar-foreground", "#3c3836"),
            ("sidebar-primary", "#b57614"),
            ("sidebar-primary-foreground", "#fbf1c7"),
            ("sidebar-accent", "#ebdbb2"),
            ("sidebar-accent-foreground", "#3c3836"),
            ("sidebar-border", "#d5c4a1"),
            ("sidebar-ring", "#b57614"),
            ("radius", "0.375rem"),
        ],
        &[
            ("background", "#282828"),
            ("foreground", "#ebdbb2"),
            ("card", "#32302f"),
            ("card-foreground", "#ebdbb2"),
            ("popover", "#32302f"),
            ("popover-foreground", "#ebdbb2"),
            ("primary", "#fabd2f"),
            ("primary-foreground", "#282828"),
            ("secondary", "#3c3836"),
            ("secondary-foreground", "#ebdbb2"),
            ("muted", "#3c3836"),
            ("muted-foreground", "#a89984"),
            ("accent", "#3c3836"),
            ("accent-foreground", "#ebdbb2"),
            ("destructive", "#fb4934"),
            ("destructive-foreground", "#282828"),
            ("border", "#504945"),
            ("input", "#3c3836"),
            ("ring", "#fabd2f"),
            ("chart-1", "#fabd2f"),
            ("chart-2", "#83a598"),
            ("chart-3", "#b8bb26"),
            ("chart-4", "#fe8019"),
            ("chart-5", "#d3869b"),
            ("sidebar", "#32302f"),
            ("sidebar-foreground", "#ebdbb2"),
            ("sidebar-primary", "#fabd2f"),
            ("sidebar-primary-foreground", "#282828"),
            ("sidebar-accent", "#3c3836"),
            ("sidebar-accent-foreground", "#ebdbb2"),
            ("sidebar-border", "#504945"),
            ("sidebar-ring", "#fabd2f"),
            ("radius", "0.375rem"),
        ],
    )
}

/// Solarized — the canonical 16-color scheme; light on base3/base2,
/// dark on base03/base02, blue as the accent in both.
fn solarized_theme_preset() -> ThemePreset {
    preset_from_palette(
        "solarized",
        "Solarized",
        &[
            ("background", "#fdf6e3"),
            ("foreground", "#586e75"),
            ("card", "#eee8d5"),
            ("card-foreground", "#586e75"),
            ("popover", "#eee8d5"),
            ("popover-foreground", "#586e75"),
            ("primary", "#268bd2"),
            ("primary-foreground", "#fdf6e3"),
            ("secondary", "#eee8d5"),
            ("secondary-foreground", "#586e75"),
            ("muted", "#eee8d5"),
            ("muted-foreground", "#93a1a1"),
            ("accent", "#eee8d5"),
            ("accent-foreground", "#586e75"),
            ("destructive", "#dc322f"),
            ("destructive-foreground", "#fdf6e3"),
            ("border", "#d9d2c2"),
            ("input", "#eee8d5"),
            ("ring", "#268bd2"),
            ("chart-1", "#268bd2"),
            ("chart-2", "#2aa198"),
            ("chart-3", "#859900"),
            ("chart-4", "#b58900"),
            ("chart-5", "#6c71c4"),
            ("sidebar", "#eee8d5"),
            ("sidebar-foreground", "#586e75"),
            ("sidebar-primary", "#268bd2"),
            ("sidebar-primary-foreground", "#fdf6e3"),
            ("sidebar-accent", "#e4ddc8"),
            ("sidebar-accent-foreground", "#586e75"),
            ("sidebar-border", "#d9d2c2"),
            ("sidebar-ring", "#268bd2"),
        ],
        &[
            ("background", "#002b36"),
            ("foreground", "#93a1a1"),
            ("card", "#073642"),
            ("card-foreground", "#93a1a1"),
            ("popover", "#073642"),
            ("popover-foreground", "#93a1a1"),
            ("primary", "#268bd2"),
            ("primary-foreground", "#fdf6e3"),
            ("secondary", "#073642"),
            ("secondary-foreground", "#93a1a1"),
            ("muted", "#073642"),
            ("muted-foreground", "#657b83"),
            ("accent", "#073642"),
            ("accent-foreground", "#93a1a1"),
            ("destructive", "#dc322f"),
            ("destructive-foreground", "#fdf6e3"),
            ("border", "#164752"),
            ("input", "#073642"),
            ("ring", "#268bd2"),
            ("chart-1", "#268bd2"),
            ("chart-2", "#2aa198"),
            ("chart-3", "#859900"),
            ("chart-4", "#b58900"),
            ("chart-5", "#6c71c4"),
            ("sidebar", "#073642"),
            ("sidebar-foreground", "#93a1a1"),
            ("sidebar-primary", "#268bd2"),
            ("sidebar-primary-foreground", "#fdf6e3"),
            ("sidebar-accent", "#0d4250"),
            ("sidebar-accent-foreground", "#93a1a1"),
            ("sidebar-border", "#164752"),
            ("sidebar-ring", "#268bd2"),
        ],
    )
}

/// Tokyo Night — light = the Day variant, dark = the Night variant.
fn tokyo_night_theme_preset() -> ThemePreset {
    preset_from_palette(
        "tokyo-night",
        "Tokyo Night",
        &[
            ("background", "#e1e2e7"),
            ("foreground", "#343b59"),
            ("card", "#e9e9ed"),
            ("card-foreground", "#343b59"),
            ("popover", "#e9e9ed"),
            ("popover-foreground", "#343b59"),
            ("primary", "#2e7de9"),
            ("primary-foreground", "#e1e2e7"),
            ("secondary", "#d5d6db"),
            ("secondary-foreground", "#343b59"),
            ("muted", "#d5d6db"),
            ("muted-foreground", "#6172b0"),
            ("accent", "#d5d6db"),
            ("accent-foreground", "#343b59"),
            ("destructive", "#f52a65"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#c4c8da"),
            ("input", "#d5d6db"),
            ("ring", "#2e7de9"),
            ("chart-1", "#2e7de9"),
            ("chart-2", "#9854f1"),
            ("chart-3", "#587539"),
            ("chart-4", "#8c6c3e"),
            ("chart-5", "#007197"),
            ("sidebar", "#d5d6db"),
            ("sidebar-foreground", "#343b59"),
            ("sidebar-primary", "#2e7de9"),
            ("sidebar-primary-foreground", "#e1e2e7"),
            ("sidebar-accent", "#c4c8da"),
            ("sidebar-accent-foreground", "#343b59"),
            ("sidebar-border", "#c4c8da"),
            ("sidebar-ring", "#2e7de9"),
        ],
        &[
            ("background", "#1a1b26"),
            ("foreground", "#c0caf5"),
            ("card", "#16161e"),
            ("card-foreground", "#c0caf5"),
            ("popover", "#16161e"),
            ("popover-foreground", "#c0caf5"),
            ("primary", "#7aa2f7"),
            ("primary-foreground", "#16161e"),
            ("secondary", "#292e42"),
            ("secondary-foreground", "#c0caf5"),
            ("muted", "#292e42"),
            ("muted-foreground", "#565f89"),
            ("accent", "#292e42"),
            ("accent-foreground", "#c0caf5"),
            ("destructive", "#f7768e"),
            ("destructive-foreground", "#16161e"),
            ("border", "#292e42"),
            ("input", "#292e42"),
            ("ring", "#7aa2f7"),
            ("chart-1", "#7aa2f7"),
            ("chart-2", "#bb9af7"),
            ("chart-3", "#9ece6a"),
            ("chart-4", "#e0af68"),
            ("chart-5", "#7dcfff"),
            ("sidebar", "#16161e"),
            ("sidebar-foreground", "#c0caf5"),
            ("sidebar-primary", "#7aa2f7"),
            ("sidebar-primary-foreground", "#16161e"),
            ("sidebar-accent", "#292e42"),
            ("sidebar-accent-foreground", "#c0caf5"),
            ("sidebar-border", "#292e42"),
            ("sidebar-ring", "#7aa2f7"),
        ],
    )
}

/// Rosé Pine — light = Dawn, dark = Main. Iris is the accent.
fn rose_pine_theme_preset() -> ThemePreset {
    preset_from_palette(
        "rose-pine",
        "Rosé Pine",
        &[
            ("background", "#faf4ed"),
            ("foreground", "#575279"),
            ("card", "#fffaf3"),
            ("card-foreground", "#575279"),
            ("popover", "#fffaf3"),
            ("popover-foreground", "#575279"),
            ("primary", "#907aa9"),
            ("primary-foreground", "#faf4ed"),
            ("secondary", "#f2e9e1"),
            ("secondary-foreground", "#575279"),
            ("muted", "#f2e9e1"),
            ("muted-foreground", "#797593"),
            ("accent", "#f2e9e1"),
            ("accent-foreground", "#575279"),
            ("destructive", "#b4637a"),
            ("destructive-foreground", "#faf4ed"),
            ("border", "#dfdad9"),
            ("input", "#f2e9e1"),
            ("ring", "#907aa9"),
            ("chart-1", "#907aa9"),
            ("chart-2", "#286983"),
            ("chart-3", "#56949f"),
            ("chart-4", "#ea9d34"),
            ("chart-5", "#d7827e"),
            ("sidebar", "#fffaf3"),
            ("sidebar-foreground", "#575279"),
            ("sidebar-primary", "#907aa9"),
            ("sidebar-primary-foreground", "#faf4ed"),
            ("sidebar-accent", "#f2e9e1"),
            ("sidebar-accent-foreground", "#575279"),
            ("sidebar-border", "#dfdad9"),
            ("sidebar-ring", "#907aa9"),
        ],
        &[
            ("background", "#191724"),
            ("foreground", "#e0def4"),
            ("card", "#1f1d2e"),
            ("card-foreground", "#e0def4"),
            ("popover", "#1f1d2e"),
            ("popover-foreground", "#e0def4"),
            ("primary", "#c4a7e7"),
            ("primary-foreground", "#191724"),
            ("secondary", "#26233a"),
            ("secondary-foreground", "#e0def4"),
            ("muted", "#26233a"),
            ("muted-foreground", "#908caa"),
            ("accent", "#26233a"),
            ("accent-foreground", "#e0def4"),
            ("destructive", "#eb6f92"),
            ("destructive-foreground", "#191724"),
            ("border", "#403d52"),
            ("input", "#26233a"),
            ("ring", "#c4a7e7"),
            ("chart-1", "#c4a7e7"),
            ("chart-2", "#31748f"),
            ("chart-3", "#9ccfd8"),
            ("chart-4", "#f6c177"),
            ("chart-5", "#ebbcba"),
            ("sidebar", "#1f1d2e"),
            ("sidebar-foreground", "#e0def4"),
            ("sidebar-primary", "#c4a7e7"),
            ("sidebar-primary-foreground", "#191724"),
            ("sidebar-accent", "#26233a"),
            ("sidebar-accent-foreground", "#e0def4"),
            ("sidebar-border", "#403d52"),
            ("sidebar-ring", "#c4a7e7"),
        ],
    )
}

/// Everforest — light/dark medium variants; green as the accent.
fn everforest_theme_preset() -> ThemePreset {
    preset_from_palette(
        "everforest",
        "Everforest",
        &[
            ("background", "#fdf6e3"),
            ("foreground", "#5c6a72"),
            ("card", "#f4f0d9"),
            ("card-foreground", "#5c6a72"),
            ("popover", "#f4f0d9"),
            ("popover-foreground", "#5c6a72"),
            ("primary", "#8da101"),
            ("primary-foreground", "#fdf6e3"),
            ("secondary", "#efebd4"),
            ("secondary-foreground", "#5c6a72"),
            ("muted", "#efebd4"),
            ("muted-foreground", "#939f91"),
            ("accent", "#efebd4"),
            ("accent-foreground", "#5c6a72"),
            ("destructive", "#f85552"),
            ("destructive-foreground", "#fdf6e3"),
            ("border", "#e6e2cc"),
            ("input", "#efebd4"),
            ("ring", "#8da101"),
            ("chart-1", "#8da101"),
            ("chart-2", "#3a94c5"),
            ("chart-3", "#35a77c"),
            ("chart-4", "#dfa000"),
            ("chart-5", "#df69ba"),
            ("sidebar", "#f4f0d9"),
            ("sidebar-foreground", "#5c6a72"),
            ("sidebar-primary", "#8da101"),
            ("sidebar-primary-foreground", "#fdf6e3"),
            ("sidebar-accent", "#efebd4"),
            ("sidebar-accent-foreground", "#5c6a72"),
            ("sidebar-border", "#e6e2cc"),
            ("sidebar-ring", "#8da101"),
        ],
        &[
            ("background", "#2d353b"),
            ("foreground", "#d3c6aa"),
            ("card", "#343f44"),
            ("card-foreground", "#d3c6aa"),
            ("popover", "#343f44"),
            ("popover-foreground", "#d3c6aa"),
            ("primary", "#a7c080"),
            ("primary-foreground", "#2d353b"),
            ("secondary", "#3d484d"),
            ("secondary-foreground", "#d3c6aa"),
            ("muted", "#3d484d"),
            ("muted-foreground", "#859289"),
            ("accent", "#3d484d"),
            ("accent-foreground", "#d3c6aa"),
            ("destructive", "#e67e80"),
            ("destructive-foreground", "#2d353b"),
            ("border", "#475258"),
            ("input", "#3d484d"),
            ("ring", "#a7c080"),
            ("chart-1", "#a7c080"),
            ("chart-2", "#7fbbb3"),
            ("chart-3", "#83c092"),
            ("chart-4", "#dbbc7f"),
            ("chart-5", "#d699b6"),
            ("sidebar", "#343f44"),
            ("sidebar-foreground", "#d3c6aa"),
            ("sidebar-primary", "#a7c080"),
            ("sidebar-primary-foreground", "#2d353b"),
            ("sidebar-accent", "#3d484d"),
            ("sidebar-accent-foreground", "#d3c6aa"),
            ("sidebar-border", "#475258"),
            ("sidebar-ring", "#a7c080"),
        ],
    )
}

/// Zen — a calm, low-contrast house preset: warm paper grays, a muted
/// sage accent, soft shadows and larger radii. Deliberately quiet.
fn zen_theme_preset() -> ThemePreset {
    preset_from_palette(
        "zen",
        "Zen",
        &[
            ("background", "#f8f7f4"),
            ("foreground", "#4a4a44"),
            ("card", "#f1f0ea"),
            ("card-foreground", "#4a4a44"),
            ("popover", "#f1f0ea"),
            ("popover-foreground", "#4a4a44"),
            ("primary", "#78826b"),
            ("primary-foreground", "#f8f7f4"),
            ("secondary", "#ecebe3"),
            ("secondary-foreground", "#55554e"),
            ("muted", "#ecebe3"),
            ("muted-foreground", "#8b8a80"),
            ("accent", "#e6e5db"),
            ("accent-foreground", "#55554e"),
            ("destructive", "#b3574e"),
            ("destructive-foreground", "#f8f7f4"),
            ("border", "#e2e1d7"),
            ("input", "#ecebe3"),
            ("ring", "#9aa38b"),
            ("chart-1", "#78826b"),
            ("chart-2", "#7a8ba0"),
            ("chart-3", "#a08e6f"),
            ("chart-4", "#8f7f95"),
            ("chart-5", "#6f9089"),
            ("sidebar", "#f1f0ea"),
            ("sidebar-foreground", "#4a4a44"),
            ("sidebar-primary", "#78826b"),
            ("sidebar-primary-foreground", "#f8f7f4"),
            ("sidebar-accent", "#e6e5db"),
            ("sidebar-accent-foreground", "#55554e"),
            ("sidebar-border", "#e2e1d7"),
            ("sidebar-ring", "#9aa38b"),
            ("radius", "0.75rem"),
            ("letter-spacing", "0.01em"),
            ("shadow-opacity", "0.06"),
        ],
        &[
            ("background", "#21221e"),
            ("foreground", "#c6c4b6"),
            ("card", "#282923"),
            ("card-foreground", "#c6c4b6"),
            ("popover", "#282923"),
            ("popover-foreground", "#c6c4b6"),
            ("primary", "#a3ad8f"),
            ("primary-foreground", "#21221e"),
            ("secondary", "#30312a"),
            ("secondary-foreground", "#c6c4b6"),
            ("muted", "#30312a"),
            ("muted-foreground", "#8b897c"),
            ("accent", "#363730"),
            ("accent-foreground", "#c6c4b6"),
            ("destructive", "#c98078"),
            ("destructive-foreground", "#21221e"),
            ("border", "#383931"),
            ("input", "#30312a"),
            ("ring", "#a3ad8f"),
            ("chart-1", "#a3ad8f"),
            ("chart-2", "#8fa3b8"),
            ("chart-3", "#c0ab86"),
            ("chart-4", "#a795ad"),
            ("chart-5", "#85a8a0"),
            ("sidebar", "#282923"),
            ("sidebar-foreground", "#c6c4b6"),
            ("sidebar-primary", "#a3ad8f"),
            ("sidebar-primary-foreground", "#21221e"),
            ("sidebar-accent", "#363730"),
            ("sidebar-accent-foreground", "#c6c4b6"),
            ("sidebar-border", "#383931"),
            ("sidebar-ring", "#a3ad8f"),
            ("radius", "0.75rem"),
            ("letter-spacing", "0.01em"),
            ("shadow-opacity", "0.06"),
        ],
    )
}

/// Obsidian — the app's default look: near-black `#1e1e1e` canvas with
/// the purple interactive accent (`#7f6df2` dark / `#705dcf` light).
fn obsidian_theme_preset() -> ThemePreset {
    preset_from_palette(
        "obsidian",
        "Obsidian",
        &[
            ("background", "#ffffff"),
            ("foreground", "#222222"),
            ("card", "#f6f6f6"),
            ("card-foreground", "#222222"),
            ("popover", "#ffffff"),
            ("popover-foreground", "#222222"),
            ("primary", "#705dcf"),
            ("primary-foreground", "#ffffff"),
            ("secondary", "#f2f3f5"),
            ("secondary-foreground", "#222222"),
            ("muted", "#f2f3f5"),
            ("muted-foreground", "#5c5c5c"),
            ("accent", "#eae8fb"),
            ("accent-foreground", "#46389e"),
            ("destructive", "#e93147"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#e0e0e0"),
            ("input", "#e0e0e0"),
            ("ring", "#705dcf"),
            ("chart-1", "#705dcf"),
            ("chart-2", "#086ddd"),
            ("chart-3", "#08b94e"),
            ("chart-4", "#e0ac00"),
            ("chart-5", "#d53984"),
            ("sidebar", "#f6f6f6"),
            ("sidebar-foreground", "#222222"),
            ("sidebar-primary", "#705dcf"),
            ("sidebar-primary-foreground", "#ffffff"),
            ("sidebar-accent", "#eae8fb"),
            ("sidebar-accent-foreground", "#46389e"),
            ("sidebar-border", "#e0e0e0"),
            ("sidebar-ring", "#705dcf"),
        ],
        &[
            ("background", "#1e1e1e"),
            ("foreground", "#dadada"),
            ("card", "#161616"),
            ("card-foreground", "#dadada"),
            ("popover", "#262626"),
            ("popover-foreground", "#dadada"),
            ("primary", "#7f6df2"),
            ("primary-foreground", "#fbfbfb"),
            ("secondary", "#262626"),
            ("secondary-foreground", "#dadada"),
            ("muted", "#262626"),
            ("muted-foreground", "#9c9c9c"),
            ("accent", "#303030"),
            ("accent-foreground", "#dadada"),
            ("destructive", "#fb464c"),
            ("destructive-foreground", "#ffffff"),
            ("border", "#363636"),
            ("input", "#363636"),
            ("ring", "#7f6df2"),
            ("chart-1", "#7f6df2"),
            ("chart-2", "#4d8dff"),
            ("chart-3", "#44cf6e"),
            ("chart-4", "#e0ac00"),
            ("chart-5", "#fa99cd"),
            ("sidebar", "#161616"),
            ("sidebar-foreground", "#dadada"),
            ("sidebar-primary", "#7f6df2"),
            ("sidebar-primary-foreground", "#fbfbfb"),
            ("sidebar-accent", "#303030"),
            ("sidebar-accent-foreground", "#dadada"),
            ("sidebar-border", "#363636"),
            ("sidebar-ring", "#7f6df2"),
        ],
    )
}
