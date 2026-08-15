use serde::{Deserialize, Serialize};

use crate::tags::TagSet;

/// A single result item returned by a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Unique identifier within this provider (used for history tracking).
    pub id: String,
    /// Display label shown in the results list.
    pub label: String,
    /// Secondary text (description, path, etc.).
    pub sub: String,
    /// Provider name that produced this item.
    pub provider: String,
    /// Icon identifier (path, name, or URI).
    pub icon: String,
    /// Available actions for this item.
    pub actions: Vec<ItemAction>,
    /// Searchable text fields (the matcher scores against these).
    /// First field is weighted highest, subsequent fields get a position penalty.
    pub search_fields: Vec<String>,
    /// Hierarchical tags for categorization and filtering.
    /// e.g. `["audio/effects/reverb", "reaper/fx-chain"]`
    pub tags: TagSet,
    /// Whether this item is pinned to the top of results.
    pub pinned: bool,
    /// Computed score after matching + history. Higher is better.
    pub score: f64,
    /// Character positions that matched (for highlight rendering in the UI).
    pub match_positions: Vec<u32>,
    /// Arbitrary metadata for provider-specific use.
    pub metadata: serde_json::Value,
}

impl Item {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        provider: impl Into<String>,
    ) -> Self {
        let label = label.into();
        let search_fields = vec![label.clone()];
        Self {
            id: id.into(),
            label,
            sub: String::new(),
            provider: provider.into(),
            icon: String::new(),
            actions: vec![ItemAction::default()],
            search_fields,
            tags: TagSet::new(),
            pinned: false,
            score: 0.0,
            match_positions: Vec::new(),
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_sub(mut self, sub: impl Into<String>) -> Self {
        self.sub = sub.into();
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn with_search_fields(mut self, fields: Vec<String>) -> Self {
        self.search_fields = fields;
        self
    }

    pub fn with_actions(mut self, actions: Vec<ItemAction>) -> Self {
        self.actions = actions;
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Add hierarchical tags to this item.
    /// Tags are slash-separated paths like `"audio/effects/reverb"`.
    pub fn with_tags(mut self, tags: &[&str]) -> Self {
        self.tags = TagSet::from_strs(tags);
        self
    }

    /// Add a single tag to this item.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.add(crate::tags::Tag::new(tag));
        self
    }
}

/// An action that can be performed on a result item.
///
/// Actions are keyed by modifier combination so a single item can
/// have different behaviors for Enter, Shift+Enter, Ctrl+Enter, etc.
/// This is the core of the workflow pack system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemAction {
    /// Display name for this action (shown in action bar).
    pub name: String,
    /// What modifier triggers this action.
    pub modifier: ActionModifier,
    /// Command to execute. Format depends on the provider:
    /// - Shell: `"sh:command arg1 arg2"`
    /// - Reaper: `"reaper:action-id"` or `"reaper:command-id:arg"`
    /// - Script: `"script:/path/to/script.lua"`
    /// - IPC: `"ipc:socket-path:message"`
    /// - Internal: `"internal:close"` / `"internal:copy-to-clipboard"`
    pub exec: String,
    /// Keep the launcher open after this action.
    pub keep_open: bool,
    /// Short description shown as tooltip/hint.
    pub description: String,
}

/// Which modifier key combination triggers an action.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ActionModifier {
    /// Plain Enter (no modifiers).
    #[default]
    None,
    /// Shift+Enter.
    Shift,
    /// Ctrl+Enter (Cmd on macOS).
    Ctrl,
    /// Ctrl+Shift+Enter.
    CtrlShift,
    /// Alt+Enter.
    Alt,
    /// Alt+Shift+Enter.
    AltShift,
}

impl ActionModifier {
    /// Short label for display in the action bar.
    pub fn label(&self) -> &str {
        match self {
            Self::None => "\u{23CE}",
            Self::Shift => "\u{21E7}\u{23CE}",
            Self::Ctrl => "^\u{23CE}",
            Self::CtrlShift => "^\u{21E7}\u{23CE}",
            Self::Alt => "\u{2325}\u{23CE}",
            Self::AltShift => "\u{2325}\u{21E7}\u{23CE}",
        }
    }

    /// Match keyboard modifiers to an ActionModifier.
    pub fn from_modifiers(ctrl: bool, shift: bool, alt: bool) -> Self {
        match (ctrl, shift, alt) {
            (true, true, false) => Self::CtrlShift,
            (true, false, false) => Self::Ctrl,
            (false, true, false) => Self::Shift,
            (false, false, true) => Self::Alt,
            (true, true, true) => Self::CtrlShift, // fallback
            (false, true, true) => Self::AltShift,
            _ => Self::None,
        }
    }
}

impl Default for ItemAction {
    fn default() -> Self {
        Self {
            name: "activate".into(),
            modifier: ActionModifier::None,
            exec: String::new(),
            keep_open: false,
            description: String::new(),
        }
    }
}

impl ItemAction {
    pub fn new(name: impl Into<String>, exec: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            exec: exec.into(),
            ..Default::default()
        }
    }

    pub fn with_modifier(mut self, modifier: ActionModifier) -> Self {
        self.modifier = modifier;
        self
    }

    pub fn with_keep_open(mut self) -> Self {
        self.keep_open = true;
        self
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }
}

/// Configuration common to all providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Display name for this provider.
    pub name: String,
    /// Icon for this provider in the provider list.
    pub icon: String,
    /// Single-character prefix that routes queries to this provider exclusively.
    pub prefix: Option<char>,
    /// Default tags applied to all items from this provider.
    /// Items inherit these unless they override with their own tags.
    pub default_tags: Vec<String>,
    /// Minimum score threshold — items below this are filtered out.
    pub min_score: f64,
    /// Whether to hide this provider from the provider list UI.
    pub hidden: bool,
    /// Maximum number of results this provider returns per query.
    pub max_results: usize,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            icon: String::new(),
            prefix: None,
            default_tags: Vec::new(),
            min_score: 0.0,
            hidden: false,
            max_results: 50,
        }
    }
}

/// The core provider trait. Each data source implements this.
///
/// This is the Rust equivalent of Elephant's Go plugin interface.
/// Providers are registered with the QueryEngine at startup.
///
/// # For library consumers (e.g. Reaper extensions)
///
/// Implement this trait for your domain-specific data sources:
///
/// ```ignore
/// struct ReaperActionsProvider { /* ... */ }
///
/// impl Provider for ReaperActionsProvider {
///     fn name(&self) -> &str { "reaper-actions" }
///     fn query(&self, query: &str, exact: bool) -> Result<Vec<Item>, _> {
///         // Return Reaper actions as Items, tagged with "reaper/actions/..."
///     }
/// }
/// ```
pub trait Provider: Send + Sync {
    /// Unique name identifier for this provider.
    fn name(&self) -> &str;

    /// Provider configuration.
    fn config(&self) -> &ProviderConfig;

    /// Mutable access to config (for runtime updates).
    fn config_mut(&mut self) -> &mut ProviderConfig;

    /// Whether this provider is available on the current system.
    fn available(&self) -> bool {
        true
    }

    /// Called once at startup to initialize the provider (scan data, etc.).
    fn setup(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Query this provider. Returns unscored items — the engine handles scoring.
    ///
    /// `query` is the search string (with any prefix stripped).
    /// `exact` requests exact substring matching instead of fuzzy.
    fn query(
        &self,
        query: &str,
        exact: bool,
    ) -> Result<Vec<Item>, Box<dyn std::error::Error + Send + Sync>>;

    /// Activate an item (perform its default or named action).
    fn activate(
        &self,
        item: &Item,
        action: &str,
    ) -> Result<ActivationResult, Box<dyn std::error::Error + Send + Sync>> {
        let _ = (item, action);
        Ok(ActivationResult::Close)
    }
}

/// What should happen after an item is activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationResult {
    /// Close the launcher window.
    Close,
    /// Keep the launcher open (for actions that don't dismiss).
    KeepOpen,
    /// Replace the query with this string (drill-down / menu navigation).
    ReplaceQuery(String),
}
