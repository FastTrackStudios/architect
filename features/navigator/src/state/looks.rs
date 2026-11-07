//! Icons and colours for folders, tags, properties and notes (Notebook
//! Navigator's `MetadataService`: `folderIcons`, `folderColors`,
//! `folderBackgroundColors` and theirs for tags, properties and files; a
//! note's from its frontmatter, `frontmatterIconField`, `…ColorField`,
//! `…BackgroundField`). Targets are keyed as the colours are:
//! `folder:<path>`, `tag:<tag>`, `property:<key>[=<value>]`,
//! `file:<path>`.

use super::{NavigatorState, Outcome, colour_key};
use crate::icons::{self, Icon};
use crate::source::{FileOp, PropertyValue, ancestors};

/// What is painted on a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paint {
    Icon,
    /// The name's (and icon's) colour.
    Colour,
    /// The row's background.
    Background,
}

impl Paint {
    /// `icon`, `colour` (or `color`), `background`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "icon" => Self::Icon,
            "colour" | "color" => Self::Colour,
            "background" => Self::Background,
            _ => return None,
        })
    }
}

impl NavigatorState {
    /// The note at `file:<path>`'s frontmatter `field`, as text, when
    /// names and dates come from frontmatter (`useFrontmatterMetadata`).
    fn frontmatter_of(&self, target: &str, field: &str) -> Option<&str> {
        let path = target.strip_prefix("file:")?;
        let field = field.trim();
        if !self.settings.use_frontmatter || field.is_empty() {
            return None;
        }
        match self.index.file(path)?.get(field)? {
            PropertyValue::Text(t) => Some(t.as_str()).filter(|t| !t.trim().is_empty()),
            PropertyValue::List(l) => l.first().map(String::as_str),
            _ => None,
        }
    }

    /// The icons, colours and backgrounds a person set, by target (the
    /// API's metadata; a note's in its frontmatter are not here).
    #[must_use]
    pub const fn painted(
        &self,
    ) -> (
        &std::collections::BTreeMap<String, String>,
        &std::collections::BTreeMap<String, String>,
        &std::collections::BTreeMap<String, String>,
    ) {
        (&self.icons, &self.colours, &self.backgrounds)
    }

    /// The pinned notes, and where each is pinned.
    #[must_use]
    pub const fn pins(&self) -> &std::collections::BTreeMap<String, crate::list::PinIn> {
        &self.pins
    }

    /// A note's colour from its frontmatter (`frontmatterColorField`).
    pub(super) fn frontmatter_colour(&self, key: &str) -> Option<&str> {
        self.frontmatter_of(key, &self.settings.frontmatter_color_field)
    }

    /// `target`'s own icon: set by a person, or (a note's) in its
    /// frontmatter.
    #[must_use]
    pub fn own_icon(&self, target: &str) -> Option<Icon> {
        let key = colour_key(target);
        self.frontmatter_of(&key, &self.settings.frontmatter_icon_field)
            .and_then(Icon::parse)
            .or_else(|| self.icons.get(&key).and_then(|i| Icon::parse(i)))
    }

    /// The icon a folder's row draws, when the settings show folder icons
    /// and it has one.
    #[must_use]
    pub fn folder_icon(&self, path: &str) -> Option<Icon> {
        self.settings
            .show_folder_icons
            .then(|| {
                self.own_icon(&format!("folder:{path}")).or_else(|| {
                    // A provider's container's own (a project's glyph).
                    self.index
                        .folder_meta_of(path)
                        .and_then(|m| m.icon.as_deref())
                        .and_then(Icon::parse)
                })
            })
            .flatten()
    }

    /// The icon a tag's or a property's row draws.
    #[must_use]
    pub fn item_icon(&self, target: &str) -> Option<Icon> {
        let show = if target.starts_with("tag:") {
            self.settings.show_tag_icons
        } else {
            self.settings.show_property_icons
        };
        show.then(|| self.own_icon(target)).flatten()
    }

    /// The icon a note's row draws (`resolveFileIconId`): its own, then
    /// one its name gives (`showFilenameMatchIcons`), then its type's
    /// (`showCategoryIcons`); none when file icons are off.
    #[must_use]
    pub fn file_icon(&self, path: &str) -> Option<Icon> {
        let s = &self.settings;
        if !s.show_file_icons {
            return None;
        }
        if let Some(own) = self.own_icon(&format!("file:{path}")) {
            return Some(own);
        }
        let file = self.index.file(path)?;
        // A provider's item's own icon (a song's, a capture's).
        if !self.index.in_vault(path)
            && let Some(PropertyValue::Text(i)) = file.get("icon")
            && let Some(icon) = Icon::parse(i)
        {
            return Some(icon);
        }
        if s.file_name_icons {
            let map = icons::icon_map(&s.file_name_icon_map);
            if let Some(i) = icons::name_icon(file.name(), &map) {
                return Some(i);
            }
        }
        s.file_type_icons
            .then(|| icons::type_icon(path, &icons::icon_map(&s.file_type_icon_map)))
            .flatten()
    }

    /// `target`'s row's background: its own, or (folders, tags) the
    /// nearest above's, as the colours inherit.
    #[must_use]
    pub fn background(&self, target: &str) -> Option<&str> {
        let key = colour_key(target);
        if let Some(c) = self.frontmatter_of(&key, &self.settings.frontmatter_background_field) {
            return Some(c);
        }
        self.inherited(&self.backgrounds, &key)
    }

    /// `key`'s value in `map`, or the nearest folder's or tag's above it
    /// (`inheritFolderColors`, `inheritTagColors`), or a value's key's
    /// (`inheritPropertyColors`).
    pub(super) fn inherited<'a>(
        &self,
        map: &'a std::collections::BTreeMap<String, String>,
        key: &str,
    ) -> Option<&'a str> {
        if let Some(c) = map.get(key) {
            return Some(c);
        }
        let s = &self.settings;
        if let Some(path) = key
            .strip_prefix("folder:")
            .filter(|_| s.inherit_folder_colors)
        {
            return ancestors(path)
                .find_map(|a| map.get(&format!("folder:{a}")).map(String::as_str));
        }
        if let Some(tag) = key.strip_prefix("tag:").filter(|_| s.inherit_tag_colors) {
            return ancestors(tag)
                .filter(|a| !a.is_empty())
                .find_map(|a| map.get(&format!("tag:{a}")))
                .map(String::as_str);
        }
        if let Some((k, _)) = key
            .strip_prefix("property:")
            .and_then(|p| p.split_once('='))
            .filter(|_| s.inherit_property_colors)
        {
            return map.get(&format!("property:{k}")).map(String::as_str);
        }
        None
    }

    /// Paints `target` with `value` (an icon's name, a CSS colour; none
    /// takes it off; `setFolderIcon`, `setTagColor`, `setFileIcon`, …). A
    /// note's goes to its frontmatter when names and dates come from
    /// frontmatter: the write is returned for the source to do.
    ///
    /// # Errors
    ///
    /// An icon that does not read, or a target that is no folder, tag,
    /// property or note.
    pub fn paint(
        &mut self,
        what: Paint,
        target: &str,
        value: Option<&str>,
    ) -> Result<Option<Outcome>, String> {
        let key = colour_key(target);
        let known = ["folder:", "tag:", "property:", "file:"]
            .iter()
            .any(|p| key.starts_with(p));
        if !known {
            return Err(format!(
                "{target:?}: folder:<path>, tag:<tag>, property:<key>[=<value>] or file:<path>"
            ));
        }
        let value = value.map(str::trim).filter(|v| !v.is_empty());
        let value = match (what, value) {
            (Paint::Icon, Some(v)) => {
                Some(Icon::parse(v).ok_or_else(|| format!("{v:?} is no icon"))?)
            }
            _ => None,
        }
        .map_or_else(|| value.map(str::to_owned), |i| Some(i.id()));
        // A note's, in its frontmatter.
        if let Some(path) = key.strip_prefix("file:")
            && self.settings.use_frontmatter
            && self
                .index
                .file(path)
                .is_some_and(crate::source::NoteFile::is_markdown)
        {
            let field = match what {
                Paint::Icon => &self.settings.frontmatter_icon_field,
                Paint::Colour => &self.settings.frontmatter_color_field,
                Paint::Background => &self.settings.frontmatter_background_field,
            }
            .trim()
            .to_owned();
            if !field.is_empty() {
                let written = match (what, &value) {
                    (Paint::Icon, Some(v)) => Icon::parse(v).map(|i| i.frontmatter()),
                    (_, v) => v.clone(),
                };
                let op = written.map_or_else(
                    || FileOp::RemoveProperty {
                        path: path.to_owned(),
                        key: field.clone(),
                    },
                    |v| FileOp::SetProperty {
                        path: path.to_owned(),
                        key: field.clone(),
                        value: PropertyValue::Text(v),
                    },
                );
                // Kept in one place: what the view state had goes.
                self.map_of(what).remove(&key);
                return Ok(Some(Outcome::Write {
                    ops: vec![op],
                    open: None,
                }));
            }
        }
        let map = self.map_of(what);
        match value {
            Some(v) => map.insert(key, v),
            None => map.remove(&key),
        };
        self.rebuild_nav();
        self.rebuild_list();
        Ok(None)
    }

    const fn map_of(&mut self, what: Paint) -> &mut std::collections::BTreeMap<String, String> {
        match what {
            Paint::Icon => &mut self.icons,
            Paint::Colour => &mut self.colours,
            Paint::Background => &mut self.backgrounds,
        }
    }

    /// Icons, colours and backgrounds follow folders and files moved
    /// (`from` to `to`), or go with them.
    pub(super) fn follow_looks(&mut self, from: &str, to: Option<&str>) {
        for map in [&mut self.icons, &mut self.colours, &mut self.backgrounds] {
            let moved: Vec<String> = map
                .keys()
                .filter(|k| {
                    ["folder:", "file:"].iter().any(|p| {
                        k.strip_prefix(p)
                            .is_some_and(|rest| crate::source::is_within(rest, from))
                    })
                })
                .cloned()
                .collect();
            for k in moved {
                if let Some(v) = map.remove(&k) {
                    if let Some(to) = to {
                        let (prefix, rest) =
                            k.split_at(k.find(':').map_or(0, |i| i.saturating_add(1)));
                        let tail = rest.get(from.len()..).unwrap_or("");
                        map.insert(format!("{prefix}{to}{tail}"), v);
                    }
                }
            }
        }
    }
}
