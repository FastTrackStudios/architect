//! Every section there is now: the app's, registered once, and each
//! extension's, registered while it is loaded.

use architect_commands::fuzzy;

use crate::model::{Owner, Section, Setting};

/// Why a section was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// A section of this id is there already (from another owner).
    SectionTaken(String),
    /// A setting of this key is there already, in section `in_section`.
    KeyTaken { key: String, in_section: String },
}

/// The sections, in the order they were registered.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Registry {
    sections: Vec<Section>,
}

/// A setting search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub section: String,
    pub key: String,
    pub score: i64,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `section`. One of the same id and owner is replaced (an
    /// extension reloaded); a key another section has is refused.
    ///
    /// # Errors
    ///
    /// [`RegistryError`]: the id or a key belongs to someone else.
    pub fn register(&mut self, section: Section) -> Result<(), RegistryError> {
        if let Some(other) = self.sections.iter().find(|s| s.id == section.id) {
            if other.owner != section.owner {
                return Err(RegistryError::SectionTaken(section.id));
            }
        }
        for setting in section.settings() {
            let clash = self
                .sections
                .iter()
                .filter(|s| s.id != section.id)
                .find(|s| s.settings().any(|o| o.key == setting.key));
            if let Some(s) = clash {
                return Err(RegistryError::KeyTaken {
                    key: setting.key.clone(),
                    in_section: s.id.clone(),
                });
            }
        }
        match self.sections.iter_mut().find(|s| s.id == section.id) {
            Some(slot) => *slot = section,
            None => self.sections.push(section),
        }
        Ok(())
    }

    /// Removes the section `id`.
    pub fn remove(&mut self, id: &str) {
        self.sections.retain(|s| s.id != id);
    }

    /// Removes every section `owner` registered (an extension unloaded).
    pub fn remove_owner(&mut self, owner: &Owner) {
        self.sections.retain(|s| &s.owner != owner);
    }

    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    #[must_use]
    pub fn section(&self, id: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.id == id)
    }

    /// The setting `key` and its section.
    #[must_use]
    pub fn setting(&self, key: &str) -> Option<(&Section, &Setting)> {
        self.sections
            .iter()
            .find_map(|s| s.settings().find(|t| t.key == key).map(|t| (s, t)))
    }

    /// The groups in the list, in the order their first section came,
    /// each with its sections (the untitled group first).
    #[must_use]
    pub fn grouped(&self) -> Vec<(Option<String>, Vec<&Section>)> {
        let mut out: Vec<(Option<String>, Vec<&Section>)> = Vec::new();
        for s in &self.sections {
            match out.iter_mut().find(|(g, _)| *g == s.group) {
                Some((_, list)) => list.push(s),
                None => out.push((s.group.clone(), vec![s])),
            }
        }
        out.sort_by_key(|(g, _)| g.is_some());
        out
    }

    /// The settings `query` finds, best first: by title, then its
    /// aliases, its description, its section's title.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<Hit> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<Hit> = self
            .sections
            .iter()
            .flat_map(|s| s.settings().map(move |t| (s, t)))
            .filter_map(|(s, t)| {
                let title = fuzzy(query, &t.title).map(|m| m.score.saturating_mul(2));
                let alias = t
                    .aliases
                    .iter()
                    .filter_map(|a| fuzzy(query, a))
                    .map(|m| m.score)
                    .max();
                let words = contains(&t.description, query).then_some(8);
                let section = fuzzy(query, &s.title).map(|m| m.score / 2);
                let score = [title, alias, words, section].into_iter().flatten().max()?;
                Some(Hit {
                    section: s.id.clone(),
                    key: t.key.clone(),
                    score,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.key.cmp(&b.key)));
        hits
    }
}

fn contains(text: &str, query: &str) -> bool {
    text.to_lowercase().contains(&query.trim().to_lowercase())
}
