//! Extension system — Raycast-style extensible launcher commands.
//!
//! Extensions are Styx-manifest packages that register commands, preferences,
//! and UI views with the launcher. They can be:
//!
//! - **View commands**: Return structured UI (List, Detail, Form, Grid)
//! - **No-view commands**: Execute an action without UI (toast/HUD feedback)
//! - **Background commands**: Run on an interval (polling, status updates)
//!
//! Extensions are discovered from `~/.config/dioxus-launcher/extensions/`
//! and bundled extension directories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use facet::Facet;

// ── Extension Manifest (Styx schema) ───────────────────────────

/// Complete extension manifest, deserialized from extension.styx.
#[derive(Debug, Facet)]
pub struct ExtensionManifest {
    pub extension: ExtensionMeta,
    #[facet(default)]
    pub commands: Vec<CommandDef>,
    #[facet(default)]
    pub preferences: Vec<PreferenceDef>,
    #[facet(default)]
    pub tags: Vec<ExtTagDef>,
    #[facet(default)]
    pub icons: HashMap<String, String>,
}

#[derive(Debug, Facet)]
pub struct ExtensionMeta {
    pub name: String,
    pub title: String,
    #[facet(default)]
    pub version: String,
    #[facet(default)]
    pub author: String,
    #[facet(default)]
    pub description: String,
    #[facet(default)]
    pub icon: String,
    #[facet(default)]
    pub url: String,
    #[facet(default)]
    pub categories: Vec<String>,
}

/// A command registered by an extension.
#[derive(Debug, Facet)]
pub struct CommandDef {
    /// Unique command name within this extension.
    pub name: String,
    /// Display title.
    pub title: String,
    #[facet(default)]
    pub description: String,
    /// Command mode: "view", "no-view", "background".
    #[facet(default)]
    pub mode: String,
    #[facet(default)]
    pub icon: String,
    #[facet(default)]
    pub keywords: Vec<String>,
    /// Exec string for no-view/background commands.
    #[facet(default)]
    pub exec: String,
    /// Background interval (e.g. "30s", "5m", "1h").
    #[facet(default)]
    pub interval: String,
    /// Per-command preferences.
    #[facet(default)]
    pub preferences: Vec<PreferenceDef>,
    /// Arguments that appear inline in the search bar.
    #[facet(default)]
    pub arguments: Vec<ArgumentDef>,
    /// Actions available on this command's items.
    #[facet(default)]
    pub actions: Vec<ActionDef>,
    /// Static items this command provides (for simple list commands).
    #[facet(default)]
    pub items: Vec<CommandItemDef>,
}

/// A preference field.
#[derive(Debug, Facet)]
pub struct PreferenceDef {
    pub name: String,
    pub title: String,
    #[facet(default)]
    pub description: String,
    /// Type: "textfield", "password", "checkbox", "dropdown", "file", "directory".
    #[facet(default)]
    pub r#type: String,
    #[facet(default)]
    pub required: bool,
    #[facet(default)]
    pub default: String,
    /// For dropdown type: list of {title, value} options.
    #[facet(default)]
    pub options: Vec<DropdownOption>,
}

#[derive(Debug, Facet)]
pub struct DropdownOption {
    pub title: String,
    pub value: String,
}

/// An inline argument in the search bar.
#[derive(Debug, Facet)]
pub struct ArgumentDef {
    pub name: String,
    pub placeholder: String,
    #[facet(default)]
    pub required: bool,
    /// Type: "text", "dropdown".
    #[facet(default)]
    pub r#type: String,
}

/// An action definition within a command.
#[derive(Debug, Facet)]
pub struct ActionDef {
    #[facet(default)]
    pub name: String,
    pub title: String,
    #[facet(default)]
    pub exec: String,
    #[facet(default)]
    pub shortcut: String,
    #[facet(default)]
    pub icon: String,
    #[facet(default)]
    pub keep_open: bool,
}

/// A static item within a command.
#[derive(Debug, Facet)]
pub struct CommandItemDef {
    pub id: String,
    pub title: String,
    #[facet(default)]
    pub subtitle: String,
    #[facet(default)]
    pub icon: String,
    #[facet(default)]
    pub keywords: Vec<String>,
    #[facet(default)]
    pub tags: Vec<String>,
    #[facet(default)]
    pub actions: Vec<ActionDef>,
    /// Accessory text/badges shown on the right.
    #[facet(default)]
    pub accessories: Vec<AccessoryDef>,
}

#[derive(Debug, Facet)]
pub struct AccessoryDef {
    #[facet(default)]
    pub text: String,
    #[facet(default)]
    pub icon: String,
    #[facet(default)]
    pub tag: String,
    #[facet(default)]
    pub color: String,
}

/// Tag definition within an extension.
#[derive(Debug, Facet)]
pub struct ExtTagDef {
    pub path: String,
    pub name: String,
    #[facet(default)]
    pub color: String,
}

// ── Command Mode ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandMode {
    /// Returns structured UI (List, Detail, Form).
    View,
    /// Executes without UI, shows toast/HUD.
    NoView,
    /// Runs on a timer interval.
    Background,
}

impl CommandMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "view" => Self::View,
            "no-view" | "noview" | "no_view" => Self::NoView,
            "background" | "bg" => Self::Background,
            _ => Self::NoView,
        }
    }
}

// ── Loaded Extension ───────────────────────────────────────────

/// A loaded extension ready to register with the launcher.
#[derive(Debug)]
pub struct LoadedExtension {
    pub manifest: ExtensionManifest,
    pub base_dir: PathBuf,
}

impl LoadedExtension {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let content = std::fs::read_to_string(path)?;
        let manifest: ExtensionManifest = facet_styx::from_str(&content)?;
        let base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        Ok(Self { manifest, base_dir })
    }

    pub fn resolve_icon(&self, name: &str) -> Option<String> {
        self.manifest
            .icons
            .get(name)
            .map(|rel| self.base_dir.join(rel).to_string_lossy().to_string())
    }

    /// Convert commands into launcher Items.
    pub fn to_items(&self) -> Vec<crate::Item> {
        let ext_name = &self.manifest.extension.name;
        let mut items = Vec::new();

        for cmd in &self.manifest.commands {
            let _mode = CommandMode::from_str(&cmd.mode);

            // Commands themselves are searchable items
            let icon = if !cmd.icon.is_empty() {
                self.resolve_icon(&cmd.icon)
                    .unwrap_or_else(|| cmd.icon.clone())
            } else if !self.manifest.extension.icon.is_empty() {
                self.resolve_icon(&self.manifest.extension.icon)
                    .unwrap_or_else(|| self.manifest.extension.icon.clone())
            } else {
                ext_name
                    .chars()
                    .next()
                    .unwrap_or('E')
                    .to_uppercase()
                    .next()
                    .unwrap_or('E')
                    .to_string()
            };

            let mut search_fields = vec![cmd.title.clone(), cmd.name.clone()];
            search_fields.extend(cmd.keywords.clone());

            let mut actions: Vec<crate::ItemAction> = cmd
                .actions
                .iter()
                .map(|a| {
                    let modifier = parse_shortcut_modifier(&a.shortcut);
                    crate::ItemAction {
                        name: a.title.clone(),
                        modifier,
                        exec: if a.exec.is_empty() {
                            format!("ext:{ext_name}:{}", cmd.name)
                        } else {
                            a.exec.clone()
                        },
                        keep_open: a.keep_open,
                        description: String::new(),
                    }
                })
                .collect();

            // Default action if none specified
            if actions.is_empty() {
                actions.push(crate::ItemAction::new(
                    &cmd.title,
                    format!("ext:{ext_name}:{}", cmd.name),
                ));
            }

            let mut item =
                crate::Item::new(format!("{ext_name}/{}", cmd.name), &cmd.title, ext_name);
            item.sub = cmd.description.clone();
            item.icon = icon.clone();
            item.search_fields = search_fields;
            item.actions = actions;

            // Apply command tags
            let tag_strs: Vec<&str> = cmd
                .items
                .first()
                .map(|i| i.tags.iter().map(|s| s.as_str()).collect())
                .unwrap_or_default();
            if !tag_strs.is_empty() {
                item.tags = crate::TagSet::from_strs(&tag_strs);
            }

            items.push(item);

            // Also add static items from the command
            for ci in &cmd.items {
                let ci_icon = if !ci.icon.is_empty() {
                    self.resolve_icon(&ci.icon)
                        .unwrap_or_else(|| ci.icon.clone())
                } else {
                    icon.clone()
                };

                let mut ci_actions: Vec<crate::ItemAction> = ci
                    .actions
                    .iter()
                    .map(|a| {
                        let modifier = parse_shortcut_modifier(&a.shortcut);
                        crate::ItemAction {
                            name: a.title.clone(),
                            modifier,
                            exec: if a.exec.is_empty() {
                                format!("ext:{ext_name}:{}:{}", cmd.name, ci.id)
                            } else {
                                a.exec.clone()
                            },
                            keep_open: a.keep_open,
                            description: String::new(),
                        }
                    })
                    .collect();

                if ci_actions.is_empty() {
                    ci_actions.push(crate::ItemAction::new(
                        &cmd.title,
                        format!("ext:{ext_name}:{}:{}", cmd.name, ci.id),
                    ));
                }

                let ci_tags: Vec<&str> = ci.tags.iter().map(|s| s.as_str()).collect();
                let mut ci_search = vec![ci.title.clone()];
                ci_search.extend(ci.keywords.clone());

                let mut item = crate::Item::new(
                    format!("{ext_name}/{}/{}", cmd.name, ci.id),
                    &ci.title,
                    ext_name,
                );
                item.sub = ci.subtitle.clone();
                item.icon = ci_icon;
                item.search_fields = ci_search;
                item.actions = ci_actions;
                if !ci_tags.is_empty() {
                    item.tags = crate::TagSet::from_strs(&ci_tags);
                }

                items.push(item);
            }
        }

        items
    }
}

fn parse_shortcut_modifier(shortcut: &str) -> crate::ActionModifier {
    if shortcut.is_empty() {
        return crate::ActionModifier::None;
    }
    let lower = shortcut.to_lowercase();
    let has_ctrl = lower.contains("ctrl") || lower.contains("cmd");
    let has_shift = lower.contains("shift");
    let has_alt = lower.contains("alt") || lower.contains("opt");
    crate::ActionModifier::from_modifiers(has_ctrl, has_shift, has_alt)
}

// ── Extension Registry ─────────────────────────────────────────

/// Central registry that holds all loaded extensions and their preferences.
pub struct ExtensionRegistry {
    extensions: Vec<LoadedExtension>,
    /// Per-extension preference values. Key = "ext_name.pref_name".
    preferences: HashMap<String, String>,
    prefs_path: PathBuf,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        let prefs_path = default_prefs_path();
        let preferences = load_prefs(&prefs_path);
        Self {
            extensions: Vec::new(),
            preferences,
            prefs_path,
        }
    }

    /// Scan a directory for extensions (each subdirectory with extension.styx).
    pub fn scan_dir(&mut self, dir: &Path) {
        tracing::info!(dir = %dir.display(), "Scanning for extensions");
        let Ok(entries) = std::fs::read_dir(dir) else {
            tracing::info!(dir = %dir.display(), "Extension directory not found, skipping");
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // extension.styx in a subdirectory
            if path.is_dir() {
                let manifest_path = path.join("extension.styx");
                if manifest_path.exists() {
                    self.load_extension(&manifest_path);
                }
            }
            // Standalone .styx files (simple extensions)
            if path.extension().is_some_and(|e| e == "styx") && path.is_file() {
                // Try to load as extension (has `extension { ... }` block)
                self.load_extension(&path);
            }
        }
    }

    fn load_extension(&mut self, path: &Path) {
        match LoadedExtension::load(path) {
            Ok(ext) => {
                tracing::info!(
                    extension = %ext.manifest.extension.name,
                    commands = ext.manifest.commands.len(),
                    path = %path.display(),
                    "Loaded extension"
                );
                self.extensions.push(ext);
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to load extension"
                );
            }
        }
    }

    /// Get all loaded extensions.
    pub fn extensions(&self) -> &[LoadedExtension] {
        &self.extensions
    }

    /// Get all items from all extensions.
    pub fn all_items(&self) -> Vec<crate::Item> {
        self.extensions.iter().flat_map(|e| e.to_items()).collect()
    }

    /// Register extension tags with a tag registry.
    pub fn register_tags(&self, registry: &mut crate::TagRegistry) {
        for ext in &self.extensions {
            for tag in &ext.manifest.tags {
                registry.register(&tag.path, &tag.name, "");
                if !tag.color.is_empty() {
                    registry.set_color(&tag.path, &tag.color);
                }
            }
        }
    }

    // ── Preferences ────────────────────────────────────────

    /// Get a preference value for an extension.
    pub fn get_pref(&self, ext_name: &str, pref_name: &str) -> Option<&str> {
        let key = format!("{ext_name}.{pref_name}");
        self.preferences.get(&key).map(|s| s.as_str())
    }

    /// Set a preference value.
    pub fn set_pref(&mut self, ext_name: &str, pref_name: &str, value: &str) {
        let key = format!("{ext_name}.{pref_name}");
        self.preferences.insert(key, value.to_string());
        let _ = save_prefs(&self.preferences, &self.prefs_path);
    }

    /// Get all preference definitions for an extension.
    pub fn pref_defs(&self, ext_name: &str) -> Vec<&PreferenceDef> {
        self.extensions
            .iter()
            .find(|e| e.manifest.extension.name == ext_name)
            .map(|e| e.manifest.preferences.iter().collect())
            .unwrap_or_default()
    }

    // ── Per-Extension Storage ──────────────────────────────

    /// Get the sandboxed storage directory for an extension.
    pub fn storage_dir(&self, ext_name: &str) -> PathBuf {
        let base = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                PathBuf::from(home).join(".local/share")
            });
        base.join("dioxus-launcher")
            .join("extensions")
            .join(ext_name)
    }
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Preference persistence ─────────────────────────────────────

fn default_prefs_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local/share")
        });
    base.join("dioxus-launcher").join("extension-prefs.json")
}

fn load_prefs(path: &PathBuf) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

fn save_prefs(
    prefs: &HashMap<String, String>,
    path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data = serde_json::to_string_pretty(prefs)?;
    std::fs::write(path, data)?;
    Ok(())
}

// ── Extension directory ────────────────────────────────────────

/// Default extensions directory.
pub fn default_extensions_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".config")
        });
    base.join("dioxus-launcher").join("extensions")
}
