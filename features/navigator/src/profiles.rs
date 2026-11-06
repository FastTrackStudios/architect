//! Vault profiles, each with what it hides, which files it shows, its
//! banner, its shortcuts and its recent notes.
//!
//! Notebook Navigator's `VaultProfile` (`src/settings/types.ts`,
//! `src/utils/vaultProfiles.ts`: `DEFAULT_VAULT_PROFILE_ID`,
//! `createValidatedVaultProfile`, `getActiveVaultProfile`), its shortcuts
//! (`src/types/shortcuts.ts` `ShortcutEntry`; `ShortcutsContext`'s
//! `addShortcut`, `removeShortcut`, `reorderShortcuts`, `renameShortcut`;
//! the `open-shortcut-1`…`9` commands) and its recent notes
//! (`src/services/RecentNotesService.ts` `recordFileOpen`, `renameEntry`;
//! `recentNotesCount` 5):
//!
//! - There is always one profile, "Default" (id `default`); the one in use
//!   is [`Profiles::active`], the default when its id is gone. Switching profiles changes what is hidden,
//!   which shortcuts and which recent notes show.
//! - A shortcut is a note, a folder, a tag, a property (a key or one
//!   value) or a saved search; one can be named. The same target is not
//!   added twice; a saved search is known by its name. The first nine
//!   open with `NAVIGATOR_OPEN_SHORTCUT_1`…`9`.
//! - Opening a note puts it first in the profile's recent notes, as many
//!   as `recent_notes_count` kept; a rename follows it, a delete drops it.

use facet::Facet;

use crate::hidden::{FileVisibility, HiddenRules};
use crate::selection::NavTarget;

/// The profile every vault has.
pub const DEFAULT_ID: &str = "default";

/// What a shortcut opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum ShortcutKind {
    #[default]
    Note,
    Folder,
    Tag,
    Property,
    Search,
}

impl ShortcutKind {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Folder => "folder",
            Self::Tag => "tag",
            Self::Property => "property",
            Self::Search => "search",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        [
            Self::Note,
            Self::Folder,
            Self::Tag,
            Self::Property,
            Self::Search,
        ]
        .into_iter()
        .find(|k| k.id() == id)
    }
}

/// A shortcut (`ShortcutEntry`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct Shortcut {
    pub kind: ShortcutKind,
    /// A note's or a folder's path, a tag, a property's key; a saved
    /// search's text.
    pub target: String,
    /// A property's value, for a shortcut to one value.
    pub value: Option<String>,
    /// The name it shows with (`alias`; a saved search's `name`); empty
    /// for the target's own.
    pub name: String,
}

impl Shortcut {
    #[must_use]
    pub fn note(path: impl Into<String>) -> Self {
        Self {
            kind: ShortcutKind::Note,
            target: path.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn search(name: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            kind: ShortcutKind::Search,
            target: query.into(),
            name: name.into(),
            value: None,
        }
    }

    /// A shortcut to a folder, a tag or a property.
    #[must_use]
    pub fn to(target: &NavTarget) -> Self {
        match target {
            NavTarget::Folder(f) => Self {
                kind: ShortcutKind::Folder,
                target: f.clone(),
                ..Self::default()
            },
            NavTarget::Tag(t) => Self {
                kind: ShortcutKind::Tag,
                target: t.clone(),
                ..Self::default()
            },
            NavTarget::Property { key, value } => Self {
                kind: ShortcutKind::Property,
                target: key.clone(),
                value: value.clone(),
                name: String::new(),
            },
        }
    }

    /// Whether it opens the same thing as `other` (a saved search: has
    /// the same name).
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        self.kind == other.kind
            && match self.kind {
                ShortcutKind::Search => self.name.eq_ignore_ascii_case(&other.name),
                ShortcutKind::Tag | ShortcutKind::Property => {
                    self.target.eq_ignore_ascii_case(&other.target)
                        && self.value.as_deref().map(str::to_lowercase)
                            == other.value.as_deref().map(str::to_lowercase)
                }
                ShortcutKind::Note | ShortcutKind::Folder => self.target == other.target,
            }
    }

    /// What it shows as: its name, else its target's.
    #[must_use]
    pub fn label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        match self.kind {
            ShortcutKind::Note => {
                let file = crate::source::file_name(&self.target);
                file.rsplit_once('.')
                    .filter(|(stem, _)| !stem.is_empty())
                    .map_or(file, |(stem, _)| stem)
                    .to_owned()
            }
            ShortcutKind::Folder if self.target.is_empty() => "Vault".to_owned(),
            ShortcutKind::Folder => crate::source::file_name(&self.target).to_owned(),
            ShortcutKind::Tag => format!("#{}", self.target),
            ShortcutKind::Property => self
                .value
                .as_ref()
                .map_or_else(|| self.target.clone(), |v| format!("{} = {v}", self.target)),
            ShortcutKind::Search => self.target.clone(),
        }
    }

    /// The folder, tag or property it selects.
    #[must_use]
    pub fn nav_target(&self) -> Option<NavTarget> {
        match self.kind {
            ShortcutKind::Folder => Some(NavTarget::Folder(self.target.clone())),
            ShortcutKind::Tag => Some(NavTarget::Tag(self.target.clone())),
            ShortcutKind::Property => Some(NavTarget::Property {
                key: self.target.clone(),
                value: self.value.clone(),
            }),
            ShortcutKind::Note | ShortcutKind::Search => None,
        }
    }
}

/// A vault profile (`VaultProfile`).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct VaultProfile {
    pub id: String,
    pub name: String,
    pub file_visibility: FileVisibility,
    pub hidden_folders: Vec<String>,
    /// Folders whose notes stay out of the lists above them
    /// (`descendantExcludedFolders`).
    pub descendant_excluded: Vec<String>,
    pub hidden_tags: Vec<String>,
    /// Notes hidden for a tag they have (`hiddenFileTags`).
    pub hidden_file_tags: Vec<String>,
    /// Notes and files hidden by name or path (`hiddenFileNames`).
    pub hidden_file_names: Vec<String>,
    /// Notes hidden by frontmatter: `key` or `key=value`
    /// (`hiddenFileProperties`).
    pub hidden_file_properties: Vec<String>,
    /// An image over the folders: a path in the vault or a URL
    /// (`navigationBanner`).
    pub banner: Option<String>,
    pub shortcuts: Vec<Shortcut>,
    /// The notes opened last, newest first.
    pub recent: Vec<String>,
}

impl VaultProfile {
    /// A profile with nothing hidden, showing every file.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            file_visibility: FileVisibility::All,
            hidden_folders: Vec::new(),
            descendant_excluded: Vec::new(),
            hidden_tags: Vec::new(),
            hidden_file_tags: Vec::new(),
            hidden_file_names: Vec::new(),
            hidden_file_properties: Vec::new(),
            banner: None,
            shortcuts: Vec::new(),
            recent: Vec::new(),
        }
    }

    /// Its hiding rules.
    #[must_use]
    pub fn rules(&self) -> HiddenRules<'_> {
        HiddenRules {
            folders: &self.hidden_folders,
            descendant_excluded: &self.descendant_excluded,
            tags: &self.hidden_tags,
            file_tags: &self.hidden_file_tags,
            file_names: &self.hidden_file_names,
            file_properties: &self.hidden_file_properties,
            visibility: self.file_visibility,
        }
    }

    /// Adds `shortcut` (at `at`, else last), unless it is there; returns
    /// whether it was added.
    pub fn add_shortcut(&mut self, shortcut: Shortcut, at: Option<usize>) -> bool {
        if self.shortcuts.iter().any(|s| s.same(&shortcut)) {
            return false;
        }
        let at = at.unwrap_or(self.shortcuts.len()).min(self.shortcuts.len());
        self.shortcuts.insert(at, shortcut);
        true
    }

    /// Takes out the shortcut that opens what `shortcut` opens.
    pub fn remove_shortcut(&mut self, shortcut: &Shortcut) -> bool {
        let before = self.shortcuts.len();
        self.shortcuts.retain(|s| !s.same(shortcut));
        self.shortcuts.len() != before
    }

    /// Moves the shortcut at `from` to `to` (`reorderShortcuts`).
    pub fn move_shortcut(&mut self, from: usize, to: usize) -> bool {
        if from >= self.shortcuts.len() {
            return false;
        }
        let s = self.shortcuts.remove(from);
        let to = to.min(self.shortcuts.len());
        self.shortcuts.insert(to, s);
        from != to
    }

    /// Records `path` opened (`recordFileOpen`).
    pub fn record(&mut self, path: &str, limit: usize) -> bool {
        let before = self.recent.clone();
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(limit.max(1));
        self.recent != before
    }

    /// Follows a note moved, or drops it (`to` `None`): its recent entry
    /// and its shortcut; a folder moved takes the shortcuts below it.
    pub fn follow(&mut self, from: &str, to: Option<&str>) {
        let rebase = |p: &str| -> Option<Option<String>> {
            if p == from {
                Some(to.map(str::to_owned))
            } else {
                p.strip_prefix(from)
                    .filter(|rest| rest.starts_with('/') && !from.is_empty())
                    .map(|rest| to.map(|t| format!("{t}{rest}")))
            }
        };
        let mut seen = std::collections::BTreeSet::new();
        self.recent = std::mem::take(&mut self.recent)
            .into_iter()
            .filter_map(|p| rebase(&p).unwrap_or(Some(p)))
            .filter(|p| seen.insert(p.clone()))
            .collect();
        self.shortcuts.retain_mut(|s| {
            if !matches!(s.kind, ShortcutKind::Note | ShortcutKind::Folder) {
                return true;
            }
            match rebase(&s.target) {
                Some(Some(new)) => {
                    s.target = new;
                    true
                }
                Some(None) => false,
                None => true,
            }
        });
    }
}

/// Every profile, and the one in use: the default one first, always
/// there.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Profiles {
    pub default: VaultProfile,
    pub others: Vec<VaultProfile>,
    /// The id of the one in use.
    pub active: String,
}

impl Default for Profiles {
    fn default() -> Self {
        Self {
            default: VaultProfile::new(DEFAULT_ID, "Default"),
            others: Vec::new(),
            active: DEFAULT_ID.to_owned(),
        }
    }
}

impl Profiles {
    /// Every profile, the default first.
    pub fn all(&self) -> impl Iterator<Item = &VaultProfile> {
        std::iter::once(&self.default).chain(&self.others)
    }

    /// The profile in use (the default when the id is gone).
    #[must_use]
    pub fn active(&self) -> &VaultProfile {
        self.others
            .iter()
            .find(|p| p.id == self.active)
            .unwrap_or(&self.default)
    }

    /// The profile in use, to change.
    pub fn active_mut(&mut self) -> &mut VaultProfile {
        if let Some(p) = self.others.iter_mut().find(|p| p.id == self.active) {
            return p;
        }
        &mut self.default
    }

    /// The profile named or numbered `which`: its id, its name (any case),
    /// or `1`… its place.
    #[must_use]
    pub fn find(&self, which: &str) -> Option<&VaultProfile> {
        let which = which.trim();
        self.all()
            .find(|p| p.id == which)
            .or_else(|| self.all().find(|p| p.name.eq_ignore_ascii_case(which)))
            .or_else(|| {
                which
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|i| self.all().nth(i))
            })
    }

    /// The profile `which` names (as [`find`](Self::find)), to change.
    pub fn find_mut(&mut self, which: &str) -> Option<&mut VaultProfile> {
        let id = self.find(which)?.id.clone();
        if self.default.id == id {
            return Some(&mut self.default);
        }
        self.others.iter_mut().find(|p| p.id == id)
    }

    /// Uses the profile `which` names; returns whether it changed.
    pub fn select(&mut self, which: &str) -> Result<bool, String> {
        let id = self
            .find(which)
            .map(|p| p.id.clone())
            .ok_or_else(|| format!("No profile {which}"))?;
        let changed = id != self.active().id;
        self.active = id;
        Ok(changed)
    }

    /// Adds a profile named `name`, as the one in use is (`copy`) or
    /// empty; returns its id.
    pub fn add(&mut self, name: &str, copy: bool) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A profile needs a name".to_owned());
        }
        if self.all().any(|p| p.name.eq_ignore_ascii_case(name)) {
            return Err(format!("There is a profile {name} already"));
        }
        let mut id: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        while self.all().any(|p| p.id == id) {
            id.push('-');
        }
        let mut profile = if copy {
            self.active().clone()
        } else {
            VaultProfile::new("", "")
        };
        profile.id.clone_from(&id);
        name.clone_into(&mut profile.name);
        profile.recent.clear();
        self.others.push(profile);
        Ok(id)
    }

    /// Takes out the profile `which` names; the default stays.
    pub fn remove(&mut self, which: &str) -> Result<(), String> {
        let id = self
            .find(which)
            .map(|p| p.id.clone())
            .ok_or_else(|| format!("No profile {which}"))?;
        if id == self.default.id {
            return Err("The default profile stays".to_owned());
        }
        self.others.retain(|p| p.id != id);
        Ok(())
    }

    /// Renames the profile `which` names.
    pub fn rename(&mut self, which: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A profile needs a name".to_owned());
        }
        let id = self
            .find(which)
            .map(|p| p.id.clone())
            .ok_or_else(|| format!("No profile {which}"))?;
        if let Some(p) = std::iter::once(&mut self.default)
            .chain(&mut self.others)
            .find(|p| p.id == id)
        {
            name.clone_into(&mut p.name);
        }
        Ok(())
    }

    /// A note moved or gone, in every profile.
    pub fn follow(&mut self, from: &str, to: Option<&str>) {
        for p in std::iter::once(&mut self.default).chain(&mut self.others) {
            p.follow(from, to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_added_once_moved_and_followed() {
        let mut p = VaultProfile::new("a", "A");
        assert!(p.add_shortcut(Shortcut::note("x/Plan.md"), None));
        assert!(!p.add_shortcut(Shortcut::note("x/Plan.md"), None));
        assert!(p.add_shortcut(Shortcut::to(&NavTarget::Tag("Work".into())), None));
        assert!(!p.add_shortcut(Shortcut::to(&NavTarget::Tag("work".into())), None));
        assert!(p.add_shortcut(Shortcut::search("Open tasks", "has:task"), Some(0)));
        assert_eq!(p.shortcuts[0].label(), "Open tasks");
        assert!(p.move_shortcut(0, 2));
        assert_eq!(p.shortcuts[2].kind, ShortcutKind::Search);
        assert_eq!(p.shortcuts[0].label(), "Plan");
        p.follow("x", Some("y"));
        assert_eq!(p.shortcuts[0].target, "y/Plan.md");
        p.follow("y/Plan.md", None);
        assert_eq!(p.shortcuts.len(), 2);
    }

    #[test]
    fn recents_are_newest_first_and_capped() {
        let mut p = VaultProfile::new("a", "A");
        for n in ["a.md", "b.md", "c.md", "a.md"] {
            p.record(n, 2);
        }
        assert_eq!(p.recent, ["a.md", "c.md"]);
        assert!(!p.record("a.md", 2));
        p.follow("c.md", Some("d.md"));
        assert_eq!(p.recent, ["a.md", "d.md"]);
    }

    #[test]
    fn profiles_switch_by_name_number_or_id_and_the_default_stays() {
        let mut ps = Profiles::default();
        ps.active_mut().hidden_folders.push("Archive".into());
        let id = ps.add("Work", true).unwrap();
        assert_eq!(ps.find("2").unwrap().id, id);
        assert!(ps.select("work").unwrap());
        assert_eq!(ps.active().hidden_folders, ["Archive"]);
        assert!(ps.add("WORK", false).is_err());
        assert!(ps.remove("default").is_err());
        ps.remove("Work").unwrap();
        assert_eq!(ps.active().id, DEFAULT_ID);
    }
}
