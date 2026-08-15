//! TOML-based configuration system.
//!
//! Loads launcher settings from `~/.config/dioxus-launcher/config.toml`.
//! All fields are optional — missing values use defaults.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level launcher configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LauncherConfig {
    /// General settings.
    pub general: GeneralConfig,
    /// Window settings.
    pub window: WindowConfig,
    /// Theme override name or inline theme settings.
    pub theme: ThemeConfig,
    /// Keybind overrides.
    pub keybinds: KeybindConfig,
    /// Per-provider settings.
    pub providers: HashMap<String, ProviderOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Maximum results shown.
    pub max_results: usize,
    /// Default launcher mode: "full" or "palette".
    pub default_mode: String,
    /// Show sidebar in full mode.
    pub show_sidebar: bool,
    /// Show filter chips.
    pub show_chips: bool,
    /// Show action bar.
    pub show_action_bar: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            max_results: 50,
            default_mode: "full".into(),
            show_sidebar: true,
            show_chips: true,
            show_action_bar: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    /// Window width.
    pub width: f64,
    /// Window height.
    pub height: f64,
    /// Show window decorations.
    pub decorations: bool,
    /// Always on top.
    pub always_on_top: bool,
    /// Close when focus is lost.
    pub close_on_blur: bool,
    /// Remember window position.
    pub remember_position: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 800.0,
            height: 520.0,
            decorations: false,
            always_on_top: true,
            close_on_blur: false,
            remember_position: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    /// Theme name: "dark", "light", or a custom theme file path.
    pub name: String,
    /// Custom CSS to append to the theme.
    pub custom_css: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            name: "dark".into(),
            custom_css: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeybindConfig {
    /// Activate selected item.
    pub activate: String,
    /// Move selection down.
    pub next: String,
    /// Move selection up.
    pub prev: String,
    /// Clear query / close.
    pub escape: String,
    /// Toggle full/palette mode.
    pub toggle_mode: String,
}

impl Default for KeybindConfig {
    fn default() -> Self {
        Self {
            activate: "Enter".into(),
            next: "ArrowDown".into(),
            prev: "ArrowUp".into(),
            escape: "Escape".into(),
            toggle_mode: "Ctrl+Shift+P".into(),
        }
    }
}

/// Per-provider configuration override.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProviderOverride {
    /// Override the prefix character.
    pub prefix: Option<String>,
    /// Override max results for this provider.
    pub max_results: Option<usize>,
    /// Disable this provider.
    pub disabled: bool,
    /// Hide from provider list.
    pub hidden: bool,
}

impl LauncherConfig {
    /// Load configuration from the default path.
    pub fn load() -> Self {
        let path = Self::default_path();
        Self::load_from(&path)
    }

    /// Load configuration from a specific path.
    pub fn load_from(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(config) => {
                    tracing::info!(path = %path.display(), "Loaded config");
                    config
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Failed to parse config, using defaults");
                    Self::default()
                }
            },
            Err(_) => {
                tracing::debug!(path = %path.display(), "No config file found, using defaults");
                Self::default()
            }
        }
    }

    /// Save configuration to the default path.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let path = Self::default_path();
        self.save_to(&path)
    }

    /// Save configuration to a specific path.
    pub fn save_to(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Default configuration file path.
    pub fn default_path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                PathBuf::from(home).join(".config")
            });
        base.join("dioxus-launcher").join("config.toml")
    }

    /// Generate a default config file with comments.
    pub fn generate_default_toml() -> String {
        r#"# Dioxus Launcher Configuration

[general]
max_results = 50
default_mode = "full"    # "full" or "palette"
show_sidebar = true
show_chips = true
show_action_bar = true

[window]
width = 800.0
height = 520.0
decorations = false
always_on_top = true
close_on_blur = false
remember_position = false

[theme]
name = "dark"            # "dark", "light", or path to custom theme
custom_css = ""

[keybinds]
activate = "Enter"
next = "ArrowDown"
prev = "ArrowUp"
escape = "Escape"
toggle_mode = "Ctrl+Shift+P"

# Per-provider overrides
# [providers.applications]
# prefix = "a"
# max_results = 30
# disabled = false
# hidden = false
"#
        .to_string()
    }
}
