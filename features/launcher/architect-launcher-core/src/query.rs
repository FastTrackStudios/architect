//! Query engine — the central orchestrator.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::export::ExportBundle;
use crate::favorites::Favorites;
use crate::filter::{FilterPresets, FilterState, MagicWords};
use crate::history::History;
use crate::matching;
use crate::provider::{ActivationResult, Item, Provider};
use crate::tags::{Tag, TagRegistry, TagSet};
use crate::userdata::UserDataStore;

pub struct QueryEngine {
    providers: Vec<Box<dyn Provider>>,
    history: Arc<RwLock<History>>,
    history_path: PathBuf,
    favorites: Arc<RwLock<Favorites>>,
    favorites_path: PathBuf,
    presets: Arc<RwLock<FilterPresets>>,
    presets_path: PathBuf,
    userdata: Arc<RwLock<UserDataStore>>,
    userdata_path: PathBuf,
    magic_words: Arc<RwLock<MagicWords>>,
    tag_registry: TagRegistry,
    max_results: usize,
}

impl QueryEngine {
    pub fn new() -> Self {
        let history_path = crate::history::default_history_path();
        let history = History::load(&history_path);
        let favorites_path = crate::favorites::default_favorites_path();
        let favorites = Favorites::load(&favorites_path);
        let presets_path = crate::filter::default_presets_path();
        let presets = FilterPresets::load(&presets_path);
        let userdata_path = crate::userdata::default_userdata_path();
        let userdata = UserDataStore::load(&userdata_path);
        Self {
            providers: Vec::new(),
            history: Arc::new(RwLock::new(history)),
            history_path,
            favorites: Arc::new(RwLock::new(favorites)),
            favorites_path,
            presets: Arc::new(RwLock::new(presets)),
            presets_path,
            userdata: Arc::new(RwLock::new(userdata)),
            userdata_path,
            magic_words: Arc::new(RwLock::new(MagicWords::default())),
            tag_registry: TagRegistry::new(),
            max_results: 50,
        }
    }

    pub fn builder() -> QueryEngineBuilder {
        QueryEngineBuilder::new()
    }

    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }

    pub fn tag_registry(&self) -> &TagRegistry {
        &self.tag_registry
    }

    pub fn tag_registry_mut(&mut self) -> &mut TagRegistry {
        &mut self.tag_registry
    }

    pub fn add_provider(&mut self, mut provider: Box<dyn Provider>) {
        if provider.available() {
            if let Err(e) = provider.setup() {
                tracing::warn!(provider = provider.name(), error = %e, "Failed to setup provider, skipping");
                return;
            }
            tracing::info!(provider = provider.name(), "Registered provider");
            self.providers.push(provider);
        } else {
            tracing::debug!(
                provider = provider.name(),
                "Provider not available, skipping"
            );
        }
    }

    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.name()).collect()
    }

    pub fn provider_configs(&self) -> Vec<(&str, &crate::provider::ProviderConfig)> {
        self.providers
            .iter()
            .filter(|p| !p.config().hidden)
            .map(|p| (p.name(), p.config()))
            .collect()
    }

    // ── Favorites ──────────────────────────────────────────

    pub fn is_favorite(&self, item_id: &str) -> bool {
        self.favorites
            .read()
            .map(|f| f.is_favorite(item_id))
            .unwrap_or(false)
    }

    pub fn toggle_favorite(&self, item_id: &str) -> bool {
        if let Ok(mut favs) = self.favorites.write() {
            let r = favs.toggle(item_id);
            let _ = favs.save(&self.favorites_path);
            r
        } else {
            false
        }
    }

    // ── User Tags (Quick Tag) ──────────────────────────────

    pub fn add_user_tag(&self, item_id: &str, tag: &str) {
        if let Ok(mut ud) = self.userdata.write() {
            ud.add_tag(item_id, tag);
            let _ = ud.save(&self.userdata_path);
        }
    }

    pub fn remove_user_tag(&self, item_id: &str, tag: &str) {
        if let Ok(mut ud) = self.userdata.write() {
            ud.remove_tag(item_id, tag);
            let _ = ud.save(&self.userdata_path);
        }
    }

    pub fn user_tags(&self, item_id: &str) -> Vec<String> {
        self.userdata
            .read()
            .map(|ud| ud.user_tags(item_id).to_vec())
            .unwrap_or_default()
    }

    // ── Ratings ────────────────────────────────────────────

    pub fn set_rating(&self, item_id: &str, rating: u8) {
        if let Ok(mut ud) = self.userdata.write() {
            ud.set_rating(item_id, rating);
            let _ = ud.save(&self.userdata_path);
        }
    }

    pub fn rating(&self, item_id: &str) -> u8 {
        self.userdata
            .read()
            .map(|ud| ud.rating(item_id))
            .unwrap_or(0)
    }

    // ── Notes ──────────────────────────────────────────────

    pub fn set_note(&self, item_id: &str, note: &str) {
        if let Ok(mut ud) = self.userdata.write() {
            ud.set_note(item_id, note);
            let _ = ud.save(&self.userdata_path);
        }
    }

    pub fn note(&self, item_id: &str) -> String {
        self.userdata
            .read()
            .map(|ud| ud.note(item_id).to_string())
            .unwrap_or_default()
    }

    // ── Hidden ─────────────────────────────────────────────

    pub fn toggle_hidden(&self, item_id: &str) -> bool {
        if let Ok(mut ud) = self.userdata.write() {
            let r = ud.toggle_hidden(item_id);
            let _ = ud.save(&self.userdata_path);
            r
        } else {
            false
        }
    }

    pub fn is_hidden(&self, item_id: &str) -> bool {
        self.userdata
            .read()
            .map(|ud| ud.is_hidden(item_id))
            .unwrap_or(false)
    }

    // ── Recently Added ─────────────────────────────────────

    pub fn is_recently_added(&self, item_id: &str) -> bool {
        self.userdata
            .read()
            .map(|ud| ud.is_recently_added(item_id))
            .unwrap_or(false)
    }

    /// Call after all providers are set up to mark scan timestamp.
    pub fn mark_scan_complete(&self) {
        if let Ok(mut ud) = self.userdata.write() {
            ud.mark_scan_complete();
            let _ = ud.save(&self.userdata_path);
        }
    }

    // ── Presets ────────────────────────────────────────────

    pub fn presets(&self) -> Vec<crate::filter::FilterPreset> {
        self.presets
            .read()
            .map(|p| p.list().to_vec())
            .unwrap_or_default()
    }

    pub fn save_preset(&self, name: &str, state: FilterState) {
        if let Ok(mut presets) = self.presets.write() {
            presets.add(name, state);
            let _ = presets.save(&self.presets_path);
        }
    }

    pub fn delete_preset(&self, name: &str) {
        if let Ok(mut presets) = self.presets.write() {
            presets.remove(name);
            let _ = presets.save(&self.presets_path);
        }
    }

    pub fn load_preset(&self, name: &str) -> Option<FilterState> {
        self.presets
            .read()
            .ok()
            .and_then(|p| p.get(name).map(|p| p.state.clone()))
    }

    // ── Magic Words ────────────────────────────────────────

    pub fn add_magic_word(&self, keyword: &str, preset_name: &str) {
        if let Ok(mut mw) = self.magic_words.write() {
            mw.add(keyword, preset_name);
        }
    }

    /// Check if query matches a magic word. Returns (FilterState, remaining_query).
    pub fn check_magic_word(&self, query: &str) -> Option<(FilterState, String)> {
        let mw = self.magic_words.read().ok()?;
        let (preset_name, remainder) = mw.check(query)?;
        let preset = self.load_preset(preset_name)?;
        Some((preset, remainder.to_string()))
    }

    // ── Export / Import ────────────────────────────────────

    pub fn export_all(&self) -> ExportBundle {
        ExportBundle {
            version: ExportBundle::CURRENT_VERSION,
            favorites: self.favorites.read().map(|f| f.clone()).unwrap_or_default(),
            presets: self.presets.read().map(|p| p.clone()).unwrap_or_default(),
            userdata: self.userdata.read().map(|u| u.clone()).unwrap_or_default(),
            history: self.history.read().map(|h| h.clone()).unwrap_or_default(),
        }
    }

    pub fn import_all(&self, bundle: &ExportBundle) {
        if let Ok(mut favs) = self.favorites.write() {
            for id in bundle.favorites.all() {
                favs.add(id);
            }
            let _ = favs.save(&self.favorites_path);
        }
        if let Ok(mut presets) = self.presets.write() {
            for preset in bundle.presets.list() {
                presets.add(&preset.name, preset.state.clone());
            }
            let _ = presets.save(&self.presets_path);
        }
        if let Ok(mut ud) = self.userdata.write() {
            ud.merge(&bundle.userdata);
            let _ = ud.save(&self.userdata_path);
        }
    }

    // ── Query ──────────────────────────────────────────────

    pub fn query_filtered(&self, raw_query: &str, filter: &FilterState) -> Vec<Item> {
        let raw_query = raw_query.trim();

        // Check magic words first
        let (magic_filter, magic_remainder) = match self.check_magic_word(raw_query) {
            Some((mf, rem)) => (Some(mf), Some(rem)),
            None => (None, None),
        };

        let effective_query = magic_remainder.as_deref().unwrap_or(raw_query);
        let effective_filter = magic_filter.as_ref().unwrap_or(filter);

        // Parse inline #tag from query
        let (tag_filter, query_after_tag) = self.parse_tag_filter(effective_query);
        let working_query = query_after_tag.unwrap_or(effective_query);

        let (target_providers, query) = self.parse_prefix(working_query);

        let exact = query.starts_with('\'');
        let query = if exact {
            query.trim_start_matches('\'')
        } else {
            query
        };

        let provider_filter = effective_filter.provider.as_deref();

        let mut all_items: Vec<Item> = Vec::new();

        for provider in &self.providers {
            if let Some(ref targets) = target_providers
                && !targets.contains(&provider.name())
            {
                continue;
            }
            if let Some(pf) = provider_filter
                && provider.name() != pf
            {
                continue;
            }

            match provider.query(query, exact) {
                Ok(items) => {
                    let max = provider.config().max_results;
                    let default_tags = &provider.config().default_tags;
                    let mut items: Vec<Item> = items.into_iter().take(max).collect();
                    for item in &mut items {
                        if item.provider.is_empty() {
                            item.provider = provider.name().to_string();
                        }
                        if item.tags.is_empty() && !default_tags.is_empty() {
                            item.tags = TagSet::from_strs(
                                &default_tags.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                            );
                        }
                    }
                    all_items.extend(items);
                }
                Err(e) => {
                    tracing::warn!(provider = provider.name(), error = %e, "Provider query failed");
                }
            }
        }

        // Filter hidden items + apply user-assigned tags
        if let Ok(ud) = self.userdata.read() {
            all_items.retain(|item| !ud.is_hidden(&item.id));
            // Merge user-assigned tags into items
            for item in &mut all_items {
                for user_tag in ud.user_tags(&item.id) {
                    item.tags.add(crate::tags::Tag::new(user_tag));
                }
            }
        }

        // Apply inline #tag filter
        if let Some(ref ft) = tag_filter {
            all_items.retain(|item| item.tags.matches(ft));
        }

        // Apply FilterState multi-tag filter
        if !effective_filter.include.is_empty() || !effective_filter.exclude.is_empty() {
            all_items.retain(|item| effective_filter.matches(&item.tags));
        }

        // Mark first-seen for new items
        if let Ok(mut ud) = self.userdata.write() {
            for item in &all_items {
                ud.mark_seen(&item.id);
            }
        }

        // Score items
        let mut scored =
            if query.is_empty() && tag_filter.is_none() && effective_filter.include.is_empty() {
                all_items
            } else if exact {
                matching::score_items_exact(all_items, query)
            } else if query.is_empty() {
                all_items
            } else {
                matching::score_items(all_items, query)
            };

        // Apply boosts: history + favorites + ratings
        if let (Ok(history), Ok(favs), Ok(ud)) = (
            self.history.read(),
            self.favorites.read(),
            self.userdata.read(),
        ) {
            for item in &mut scored {
                item.score += history.usage_score(raw_query, &item.id);
                item.score += favs.score_boost(&item.id);
                item.score += ud.rating_boost(&item.id);
                item.pinned = item.pinned || favs.is_favorite(&item.id);
            }
        }

        scored.sort_by(|a, b| {
            b.pinned.cmp(&a.pinned).then(
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });

        scored.truncate(self.max_results);
        scored
    }

    pub fn query(&self, raw_query: &str) -> Vec<Item> {
        self.query_filtered(raw_query, &FilterState::new())
    }

    pub fn query_tagged(&self, tag: &str, search: &str) -> Vec<Item> {
        self.query(&format!("#{tag} {search}"))
    }

    fn parse_tag_filter<'a>(&self, query: &'a str) -> (Option<Tag>, Option<&'a str>) {
        if !query.starts_with('#') {
            return (None, None);
        }
        let without_hash = &query[1..];
        let (tag_str, remainder) = match without_hash.find(' ') {
            Some(pos) => (
                &without_hash[..pos],
                Some(without_hash[pos + 1..].trim_start()),
            ),
            None => (without_hash, None),
        };
        if tag_str.is_empty() {
            return (None, None);
        }
        let tag = self.tag_registry.resolve(tag_str);
        let remainder = remainder.filter(|r| !r.is_empty());
        (Some(tag), remainder)
    }

    fn parse_prefix<'a>(&self, query: &'a str) -> (Option<Vec<&str>>, &'a str) {
        let mut chars = query.chars();
        if let Some(first) = chars.next()
            && chars.next() == Some(' ')
        {
            let matching: Vec<&str> = self
                .providers
                .iter()
                .filter(|p| p.config().prefix == Some(first))
                .map(|p| p.name())
                .collect();
            if !matching.is_empty() {
                return (Some(matching), query[2..].trim_start());
            }
        }
        (None, query)
    }

    pub fn activate(
        &self,
        item: &Item,
        action: &str,
        query: &str,
    ) -> Result<ActivationResult, Box<dyn std::error::Error + Send + Sync>> {
        if let Ok(mut history) = self.history.write() {
            history.record(query, &item.id);
            let _ = history.save(&self.history_path);
        }
        for provider in &self.providers {
            if provider.name() == item.provider {
                return provider.activate(item, action);
            }
        }
        Err(format!(
            "No provider '{}' found for item '{}'",
            item.provider, item.id
        )
        .into())
    }
}

impl Default for QueryEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ── Builder ────────────────────────────────────────────────────

pub struct QueryEngineBuilder {
    providers: Vec<Box<dyn Provider>>,
    tag_registry: TagRegistry,
    magic_words: MagicWords,
    max_results: usize,
    history_path: Option<PathBuf>,
}

impl QueryEngineBuilder {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            tag_registry: TagRegistry::new(),
            magic_words: MagicWords::default(),
            max_results: 50,
            history_path: None,
        }
    }

    pub fn max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }

    pub fn history_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.history_path = Some(path.into());
        self
    }

    pub fn register_tags(mut self, f: impl FnOnce(&mut TagRegistry)) -> Self {
        f(&mut self.tag_registry);
        self
    }

    /// Register magic words: keyword + Space loads a filter preset.
    pub fn magic_word(
        mut self,
        keyword: impl Into<String>,
        preset_name: impl Into<String>,
    ) -> Self {
        self.magic_words.add(keyword, preset_name);
        self
    }

    pub fn provider(mut self, provider: Box<dyn Provider>) -> Self {
        self.providers.push(provider);
        self
    }

    pub fn providers(mut self, providers: Vec<Box<dyn Provider>>) -> Self {
        self.providers.extend(providers);
        self
    }

    /// Load and register workflow packs from a directory.
    ///
    /// This reads all `.toml` pack files and `pack.toml` subdirectories,
    /// registers their tags (with colors), presets, and magic words,
    /// and stores the loaded packs for later provider creation.
    ///
    /// After calling this, you still need to add a `WorkflowProvider`
    /// (from the `providers` crate) that serves the pack items.
    /// Or use `register_packs_with_provider()` to do both at once.
    pub fn register_packs(mut self, packs: &[crate::pack::LoadedPack]) -> Self {
        for pack in packs {
            // Register tags
            for tag in &pack.def.tags {
                self.tag_registry
                    .register(&tag.path, &tag.name, &tag.description);
                if !tag.color.is_empty() {
                    self.tag_registry.set_color(&tag.path, &tag.color);
                }
                if !tag.icon.is_empty()
                    && let Some(info) = self.tag_registry.info_mut(&Tag::new(&tag.path))
                {
                    info.icon = tag.icon.clone();
                }
            }
            // Register magic words
            for mw in &pack.def.magic_words {
                self.magic_words.add(&mw.keyword, &mw.preset);
            }
        }
        self
    }

    pub fn build(self) -> QueryEngine {
        let history_path = self
            .history_path
            .unwrap_or_else(crate::history::default_history_path);
        let history = History::load(&history_path);
        let favorites_path = crate::favorites::default_favorites_path();
        let favorites = Favorites::load(&favorites_path);
        let presets_path = crate::filter::default_presets_path();
        let presets = FilterPresets::load(&presets_path);
        let userdata_path = crate::userdata::default_userdata_path();
        let userdata = UserDataStore::load(&userdata_path);

        let mut engine = QueryEngine {
            providers: Vec::new(),
            history: Arc::new(RwLock::new(history)),
            history_path,
            favorites: Arc::new(RwLock::new(favorites)),
            favorites_path,
            presets: Arc::new(RwLock::new(presets)),
            presets_path,
            userdata: Arc::new(RwLock::new(userdata)),
            userdata_path,
            magic_words: Arc::new(RwLock::new(self.magic_words)),
            tag_registry: self.tag_registry,
            max_results: self.max_results,
        };

        for provider in self.providers {
            engine.add_provider(provider);
        }

        // Mark items seen and update scan timestamp
        engine.mark_scan_complete();

        engine
    }
}

impl Default for QueryEngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}
