//! Shared UI state — bridges the query engine to Dioxus reactivity.

use architect_launcher_core::{FilterState, Item, QueryEngine};
use std::sync::Arc;

pub struct LauncherState {
    engine: Arc<QueryEngine>,
}

impl LauncherState {
    #[must_use]
    pub fn new(engine: QueryEngine) -> Self {
        Self {
            engine: Arc::new(engine),
        }
    }

    #[must_use]
    pub fn engine(&self) -> &QueryEngine {
        &self.engine
    }

    #[must_use]
    pub fn query(&self, input: &str) -> Vec<Item> {
        self.engine.query(input)
    }

    #[must_use]
    pub fn query_filtered(&self, input: &str, filter: &FilterState) -> Vec<Item> {
        self.engine.query_filtered(input, filter)
    }

    pub fn activate(&self, item: &Item, action: &str, query: &str) -> bool {
        match self.engine.activate(item, action, query) {
            Ok(result) => matches!(result, architect_launcher_core::ActivationResult::Close),
            Err(e) => {
                tracing::error!(error = %e, "Activation failed");
                false
            }
        }
    }

    // NOT `#[must_use]`: toggling and ignoring the resulting state is
    // the ordinary call — the caller just re-reads it.
    #[allow(clippy::must_use_candidate)]
    pub fn toggle_favorite(&self, item_id: &str) -> bool {
        self.engine.toggle_favorite(item_id)
    }
    #[must_use]
    pub fn is_favorite(&self, item_id: &str) -> bool {
        self.engine.is_favorite(item_id)
    }

    pub fn set_rating(&self, item_id: &str, rating: u8) {
        self.engine.set_rating(item_id, rating);
    }
    #[must_use]
    pub fn rating(&self, item_id: &str) -> u8 {
        self.engine.rating(item_id)
    }

    pub fn set_note(&self, item_id: &str, note: &str) {
        self.engine.set_note(item_id, note);
    }
    #[must_use]
    pub fn note(&self, item_id: &str) -> String {
        self.engine.note(item_id)
    }

    #[must_use]
    pub fn toggle_hidden(&self, item_id: &str) -> bool {
        self.engine.toggle_hidden(item_id)
    }
    #[must_use]
    pub fn is_recently_added(&self, item_id: &str) -> bool {
        self.engine.is_recently_added(item_id)
    }

    pub fn add_user_tag(&self, item_id: &str, tag: &str) {
        self.engine.add_user_tag(item_id, tag);
    }
    pub fn remove_user_tag(&self, item_id: &str, tag: &str) {
        self.engine.remove_user_tag(item_id, tag);
    }
    #[must_use]
    pub fn user_tags(&self, item_id: &str) -> Vec<String> {
        self.engine.user_tags(item_id)
    }

    pub fn save_preset(&self, name: &str, state: &FilterState) {
        self.engine.save_preset(name, state.clone());
    }
    pub fn delete_preset(&self, name: &str) {
        self.engine.delete_preset(name);
    }
    #[must_use]
    pub fn presets(&self) -> Vec<architect_launcher_core::filter::FilterPreset> {
        self.engine.presets()
    }
    #[must_use]
    pub fn load_preset(&self, name: &str) -> Option<FilterState> {
        self.engine.load_preset(name)
    }
}
