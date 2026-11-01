//! Screensets: a workspace kept under a name, recalled from a menu or a
//! key (REAPER's numbered slots), one layout per form class, with the
//! working copy kept apart from the saved one (Premiere's "reset to saved
//! layout").

use facet::Facet;

use crate::model::Workspace;

/// The class of screen a layout is for. A set without one for this class
/// falls back to the nearest it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum FormClass {
    Desk,
    TabletLandscape,
    TabletPortrait,
    PhoneLandscape,
    PhonePortrait,
}

impl FormClass {
    /// The class of a screen `width` by `height` points: a phone or a
    /// tablet (`handheld`) by its shorter side, upright or on its side by
    /// its longer; anything else a desk (a touchscreen PC included).
    #[must_use]
    pub fn of(width: f32, height: f32, handheld: bool) -> Self {
        let upright = height > width;
        if !handheld {
            Self::Desk
        } else if width.min(height) < Self::PHONE_MAX {
            if upright {
                Self::PhonePortrait
            } else {
                Self::PhoneLandscape
            }
        } else if upright {
            Self::TabletPortrait
        } else {
            Self::TabletLandscape
        }
    }

    /// A handheld screen's shorter side below this is a phone's, points.
    pub const PHONE_MAX: f32 = 600.0;

    /// The classes to try for this one, nearest first.
    #[must_use]
    pub const fn fallbacks(self) -> [Self; 5] {
        use FormClass::{Desk, PhoneLandscape, PhonePortrait, TabletLandscape, TabletPortrait};
        match self {
            Desk => [
                Desk,
                TabletLandscape,
                TabletPortrait,
                PhoneLandscape,
                PhonePortrait,
            ],
            TabletLandscape => [
                TabletLandscape,
                Desk,
                TabletPortrait,
                PhoneLandscape,
                PhonePortrait,
            ],
            TabletPortrait => [
                TabletPortrait,
                TabletLandscape,
                Desk,
                PhonePortrait,
                PhoneLandscape,
            ],
            PhoneLandscape => [
                PhoneLandscape,
                PhonePortrait,
                TabletLandscape,
                TabletPortrait,
                Desk,
            ],
            PhonePortrait => [
                PhonePortrait,
                PhoneLandscape,
                TabletPortrait,
                TabletLandscape,
                Desk,
            ],
        }
    }
}

/// One form class's layout.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Layout {
    pub class: FormClass,
    pub workspace: Workspace,
}

/// A named layout.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Screenset {
    pub id: String,
    pub name: String,
    /// Recalled by ⌘1–9.
    pub slot: Option<u8>,
    /// Shipped by the app: it can be changed and reset, not deleted.
    pub builtin: bool,
    pub layouts: Vec<Layout>,
}

impl Screenset {
    /// A set the app ships (built in: it can be changed and reset, not
    /// deleted), its id from its name ("Mix" is `mix`).
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: slug(&name),
            name,
            slot: None,
            builtin: true,
            layouts: Vec::new(),
        }
    }

    /// Recalled by ⌘`n` (1–9).
    #[must_use]
    pub const fn slot(mut self, n: u8) -> Self {
        self.slot = Some(n);
        self
    }

    /// Its layout for `class`.
    #[must_use]
    pub fn layout(mut self, class: FormClass, workspace: Workspace) -> Self {
        self.set_layout(class, workspace);
        self
    }

    /// Its layout for `class`, or the nearest it has.
    #[must_use]
    pub fn layout_for(&self, class: FormClass) -> Option<&Workspace> {
        class
            .fallbacks()
            .iter()
            .find_map(|c| self.layouts.iter().find(|l| l.class == *c))
            .map(|l| &l.workspace)
    }

    fn set_layout(&mut self, class: FormClass, workspace: Workspace) {
        match self.layouts.iter_mut().find(|l| l.class == class) {
            Some(layout) => layout.workspace = workspace,
            None => self.layouts.push(Layout { class, workspace }),
        }
    }
}

/// What an app's recall action takes: the set to recall. A screenset's
/// row in a menu ([`Screensets::menu_rows`]) runs it with this.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Recall {
    pub id: String,
}

/// Every screenset, which is active, and the live (working) layout per
/// form class.
#[derive(Debug, Clone, PartialEq, Default, Facet)]
pub struct Screensets {
    pub sets: Vec<Screenset>,
    pub active: Option<String>,
    /// What the person has arranged since recalling or saving, per class.
    pub working: Vec<Layout>,
}

impl Screensets {
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Screenset> {
        self.sets.iter().find(|s| s.id == id)
    }

    /// The set in slot `n` (⌘n).
    #[must_use]
    pub fn in_slot(&self, n: u8) -> Option<&Screenset> {
        self.sets.iter().find(|s| s.slot == Some(n))
    }

    /// Every set the app ships joins these: one the person has (by id)
    /// keeps its changes and the app's name and slot; a new one is added.
    #[must_use]
    pub fn with_builtins(mut self, builtins: impl IntoIterator<Item = Screenset>) -> Self {
        for set in builtins {
            match self.sets.iter_mut().find(|s| s.id == set.id) {
                Some(kept) => {
                    kept.builtin = true;
                    kept.name = set.name;
                    kept.slot = set.slot;
                    for layout in set.layouts {
                        if !kept.layouts.iter().any(|l| l.class == layout.class) {
                            kept.layouts.push(layout);
                        }
                    }
                }
                None => self.sets.push(set),
            }
        }
        self
    }

    /// Recall the set in slot `n` (⌘n) for `class`: its layout, to show.
    pub fn recall_slot(&mut self, n: u8, class: FormClass) -> Option<Workspace> {
        let id = self.in_slot(n)?.id.clone();
        self.recall(&id, class)
    }

    /// The live layout for `class`, if one has been arranged.
    #[must_use]
    pub fn working(&self, class: FormClass) -> Option<&Workspace> {
        self.working
            .iter()
            .find(|l| l.class == class)
            .map(|l| &l.workspace)
    }

    /// Record the live layout for `class` (after every change).
    pub fn set_working(&mut self, class: FormClass, workspace: Workspace) {
        match self.working.iter_mut().find(|l| l.class == class) {
            Some(layout) => layout.workspace = workspace,
            None => self.working.push(Layout { class, workspace }),
        }
    }

    /// Recall set `id` for `class`: it becomes active and its layout the
    /// live one. Its layout, to show.
    pub fn recall(&mut self, id: &str, class: FormClass) -> Option<Workspace> {
        let workspace = self.get(id)?.layout_for(class)?.clone();
        self.active = Some(id.to_owned());
        self.set_working(class, workspace.clone());
        Some(workspace)
    }

    /// Whether the live layout for `class` differs from the active set's.
    #[must_use]
    pub fn is_dirty(&self, class: FormClass) -> bool {
        let saved = self
            .active
            .as_deref()
            .and_then(|id| self.get(id))
            .and_then(|s| s.layout_for(class));
        match (saved, self.working(class)) {
            (Some(saved), Some(working)) => saved != working,
            (None, Some(_)) => true,
            _ => false,
        }
    }

    /// Save the live layout for `class` into the active set.
    pub fn save(&mut self, class: FormClass) -> bool {
        let Some(working) = self.working(class).cloned() else {
            return false;
        };
        let Some(id) = self.active.clone() else {
            return false;
        };
        self.sets
            .iter_mut()
            .find(|s| s.id == id)
            .map_or(false, |set| {
                set.set_layout(class, working);
                true
            })
    }

    /// Save every live layout as a new set, which becomes active.
    pub fn save_as(&mut self, name: impl Into<String>, slot: Option<u8>) -> String {
        let name = name.into();
        let base = slug(&name);
        let mut id = base.clone();
        let mut n = 2u32;
        while self.get(&id).is_some() {
            id = format!("{base}-{n}");
            n = n.saturating_add(1);
        }
        if let Some(slot) = slot {
            for set in &mut self.sets {
                if set.slot == Some(slot) {
                    set.slot = None;
                }
            }
        }
        self.sets.push(Screenset {
            id: id.clone(),
            name,
            slot,
            builtin: false,
            layouts: self.working.clone(),
        });
        self.active = Some(id.clone());
        id
    }

    /// Throw away the live changes for `class`: back to the active set's
    /// saved layout, which is returned to show.
    pub fn reset(&mut self, class: FormClass) -> Option<Workspace> {
        let id = self.active.clone()?;
        self.recall(&id, class)
    }

    /// Delete a set the person made (a built-in one stays).
    pub fn delete(&mut self, id: &str) -> bool {
        let before = self.sets.len();
        self.sets.retain(|s| s.id != id || s.builtin);
        let deleted = self.sets.len() < before;
        if deleted && self.active.as_deref() == Some(id) {
            self.active = None;
        }
        deleted
    }

    /// Serialise (facet JSON).
    pub fn to_json(&self) -> Result<String, String> {
        facet_json::to_string(self).map_err(|e| e.to_string())
    }

    /// Read back, every layout repaired.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut sets: Self = facet_json::from_str(json).map_err(|e| e.to_string())?;
        for layout in sets
            .sets
            .iter_mut()
            .flat_map(|s| s.layouts.iter_mut())
            .chain(sets.working.iter_mut())
        {
            layout.workspace.repair();
        }
        Ok(sets)
    }
}

/// A set's id from its name: lower case, a dash for anything else.
fn slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

/// The View menu's live list of screensets.
impl Screensets {
    /// The sets as a live menu section's rows
    /// (`architect_commands::Menu::section`), in order: each runs the
    /// app's `recall` action with a [`Recall`] of its id, and the active
    /// one is ticked. Called from the section, so it is the list as it
    /// is when the menu is drawn:
    ///
    /// ```ignore
    /// .menu("View", |m| m.section("Screensets", move || sets.read().menu_rows("VIEW_RECALL_SCREENSET")))
    /// ```
    #[must_use]
    pub fn menu_rows(&self, recall: &str) -> Vec<architect_commands::Live> {
        self.sets
            .iter()
            .filter_map(|set| {
                let args = facet_json::to_string(&Recall { id: set.id.clone() }).ok()?;
                Some(
                    architect_commands::Live::new(set.name.clone(), recall)
                        .args(args)
                        .checked(self.active.as_deref() == Some(set.id.as_str())),
                )
            })
            .collect()
    }
}
