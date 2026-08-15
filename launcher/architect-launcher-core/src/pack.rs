//! Workflow Pack system — community-registrable config files in Styx format.
//!
//! A "pack" is a `.styx` file (optionally with an icon directory) that registers:
//! - Custom items with multi-modifier actions
//! - Tag definitions with colors and icons
//! - Icon mappings
//! - Magic word shortcuts
//! - Filter presets
//!
//! Packs live in `~/.config/dioxus-launcher/packs/` and are auto-loaded.
//!
//! # Example Pack: `visibility-manager.styx`
//!
//! ```styx
//! pack {
//!     name "visibility-manager"
//!     version "1.0.0"
//!     author "FastTrackStudios"
//!     description "Track visibility management workflows"
//! }
//!
//! tags (
//!     {path "reaper/visibility", name "Visibility", color "#cba6f7"}
//! )
//!
//! items (
//!     {
//!         id "vis-guitars"
//!         label "Guitars"
//!         sub "Guitar track visibility"
//!         tags ("reaper/visibility" "reaper/tracks")
//!         actions {
//!             enter       {name "Toggle",    exec "reaper:_RS_TOGGLE_VIS_GUITARS"}
//!             shift_enter {name "Exclusive", exec "reaper:_RS_EXCLUSIVE_VIS_GUITARS"}
//!             ctrl_enter  {name "Show",      exec "reaper:_RS_SHOW_VIS_GUITARS", keep_open true}
//!         }
//!     }
//! )
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use facet::Facet;

use crate::provider::{ActionModifier, Item, ItemAction};
use crate::tags::TagSet;

// ── Pack Definition (Styx schema) ──────────────────────────────

/// A complete workflow pack as deserialized from Styx.
#[derive(Debug, Facet)]
pub struct PackDef {
    pub pack: PackMeta,
    #[facet(default)]
    pub tags: Vec<PackTag>,
    #[facet(default)]
    pub items: Vec<PackItem>,
    #[facet(default)]
    pub icons: HashMap<String, String>,
    #[facet(default)]
    pub magic_words: Vec<PackMagicWord>,
    #[facet(default)]
    pub presets: Vec<PackPreset>,
}

#[derive(Debug, Facet)]
pub struct PackMeta {
    pub name: String,
    #[facet(default)]
    pub version: String,
    #[facet(default)]
    pub author: String,
    #[facet(default)]
    pub description: String,
    #[facet(default)]
    pub url: String,
    #[facet(default)]
    pub prefix: Option<String>,
}

#[derive(Debug, Facet)]
pub struct PackTag {
    pub path: String,
    pub name: String,
    #[facet(default)]
    pub description: String,
    #[facet(default)]
    pub color: String,
    #[facet(default)]
    pub icon: String,
}

#[derive(Debug, Facet)]
pub struct PackItem {
    pub id: String,
    pub label: String,
    #[facet(default)]
    pub sub: String,
    #[facet(default)]
    pub icon: String,
    #[facet(default)]
    pub tags: Vec<String>,
    #[facet(default)]
    pub search_fields: Vec<String>,
    #[facet(default)]
    pub pinned: bool,
    #[facet(default)]
    pub actions: PackActions,
}

/// Actions keyed by modifier. Each field is optional.
#[derive(Debug, Default, Facet)]
pub struct PackActions {
    #[facet(default)]
    pub enter: Option<PackAction>,
    #[facet(default)]
    pub shift_enter: Option<PackAction>,
    #[facet(default)]
    pub ctrl_enter: Option<PackAction>,
    #[facet(default)]
    pub ctrl_shift_enter: Option<PackAction>,
    #[facet(default)]
    pub alt_enter: Option<PackAction>,
    #[facet(default)]
    pub alt_shift_enter: Option<PackAction>,
}

#[derive(Debug, Facet)]
pub struct PackAction {
    pub name: String,
    pub exec: String,
    #[facet(default)]
    pub description: String,
    #[facet(default)]
    pub keep_open: bool,
}

#[derive(Debug, Facet)]
pub struct PackMagicWord {
    pub keyword: String,
    pub preset: String,
}

#[derive(Debug, Facet)]
pub struct PackPreset {
    pub name: String,
    #[facet(default)]
    pub include: Vec<String>,
    #[facet(default)]
    pub exclude: Vec<String>,
}

// ── Pack Loading ───────────────────────────────────────────────

/// A loaded pack, ready to be registered with the engine.
#[derive(Debug)]
pub struct LoadedPack {
    pub def: PackDef,
    /// Directory the pack was loaded from (for resolving relative icon paths).
    pub base_dir: PathBuf,
}

impl LoadedPack {
    /// Load a pack from a Styx file.
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let content = std::fs::read_to_string(path)?;
        let def: PackDef = facet_styx::from_str(&content)?;
        let base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        Ok(Self { def, base_dir })
    }

    /// Resolve an icon name to a file path using the pack's icon mappings.
    pub fn resolve_icon(&self, name: &str) -> Option<String> {
        self.def.icons.get(name).map(|rel_path| {
            let full = self.base_dir.join(rel_path);
            full.to_string_lossy().to_string()
        })
    }

    /// Convert pack items into engine Items.
    pub fn to_items(&self) -> Vec<Item> {
        let pack_name = &self.def.pack.name;
        self.def
            .items
            .iter()
            .map(|pi| {
                let icon = self
                    .resolve_icon(&pi.icon)
                    .unwrap_or_else(|| pi.icon.clone());

                let mut actions = Vec::new();
                if let Some(a) = &pi.actions.enter {
                    actions.push(pack_action_to_item_action(a, ActionModifier::None));
                }
                if let Some(a) = &pi.actions.shift_enter {
                    actions.push(pack_action_to_item_action(a, ActionModifier::Shift));
                }
                if let Some(a) = &pi.actions.ctrl_enter {
                    actions.push(pack_action_to_item_action(a, ActionModifier::Ctrl));
                }
                if let Some(a) = &pi.actions.ctrl_shift_enter {
                    actions.push(pack_action_to_item_action(a, ActionModifier::CtrlShift));
                }
                if let Some(a) = &pi.actions.alt_enter {
                    actions.push(pack_action_to_item_action(a, ActionModifier::Alt));
                }
                if let Some(a) = &pi.actions.alt_shift_enter {
                    actions.push(pack_action_to_item_action(a, ActionModifier::AltShift));
                }
                if actions.is_empty() {
                    actions.push(ItemAction::default());
                }

                let search_fields = if pi.search_fields.is_empty() {
                    vec![pi.label.clone()]
                } else {
                    pi.search_fields.clone()
                };

                let tags =
                    TagSet::from_strs(&pi.tags.iter().map(|s| s.as_str()).collect::<Vec<_>>());

                let mut item = Item::new(&pi.id, &pi.label, pack_name);
                item.sub = pi.sub.clone();
                item.icon = icon;
                item.actions = actions;
                item.search_fields = search_fields;
                item.tags = tags;
                item.pinned = pi.pinned;
                item
            })
            .collect()
    }
}

fn pack_action_to_item_action(pa: &PackAction, modifier: ActionModifier) -> ItemAction {
    ItemAction {
        name: pa.name.clone(),
        modifier,
        exec: pa.exec.clone(),
        keep_open: pa.keep_open,
        description: pa.description.clone(),
    }
}

// ── Pack Directory Scanner ─────────────────────────────────────

/// Scan a directory for pack files (.styx and legacy .toml).
pub fn scan_packs(dir: &Path) -> Vec<LoadedPack> {
    let mut packs = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return packs;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // .styx files
        if path.extension().is_some_and(|e| e == "styx") {
            load_and_push(&path, &mut packs);
        }
        // Subdirectories with pack.styx
        if path.is_dir() {
            let pack_file = path.join("pack.styx");
            if pack_file.exists() {
                load_and_push(&pack_file, &mut packs);
            }
        }
    }
    packs
}

fn load_and_push(path: &Path, packs: &mut Vec<LoadedPack>) {
    match LoadedPack::load(path) {
        Ok(pack) => {
            tracing::info!(
                pack = %pack.def.pack.name,
                items = pack.def.items.len(),
                tags = pack.def.tags.len(),
                path = %path.display(),
                "Loaded workflow pack"
            );
            packs.push(pack);
        }
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to load pack"
            );
        }
    }
}

/// Default pack directory.
pub fn default_pack_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".config")
        });
    base.join("dioxus-launcher").join("packs")
}
