//! The tags' actions (feature `actions`).
//!
//! Over an open vault ([`vault::actions::OpenVault`]): a note's tags are its frontmatter
//! `tags` and its inline `#tags` ([`vault::actions::page_tags`]), and a
//! tag's look (icon, colour) is the registry's (`Records/tags.json`,
//! [`crate::VaultTags`]).
//!
//! | action | does |
//! |---|---|
//! | `TAGS_LIST` | every tag the notes use, with how many notes use it and its look from the registry |
//! | `TAGS_NOTES {tag}` | the notes tagged `tag` or a tag nested under it (`lab` holds `lab/sample`) |
//! | `TAGS_ADD {paths, tag}` | tags the notes at `paths` (a note with the tag or one above it is left; the tags below it go) |
//! | `TAGS_REMOVE {paths, tag}` | takes the tag itself (not the ones below it) off the notes at `paths` |
//! | `TAGS_CLEAR {paths}` | takes every tag off the notes at `paths` |
//! | `TAGS_RENAME {from, to}` | renames a tag and the ones below it in every note |
//! | `TAGS_DELETE {tag}` | takes a tag and the ones below it off every note |
//!
//! The writes change the markdown: the frontmatter's `tags` and the inline
//! `#tags` (`vault_obsidian::tag_edit`), and answer the notes they changed.

use std::collections::BTreeMap;

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use facet::Facet;
use serde_json::{Value, json};
use tag_proto::TagService;
use vault::actions::{OpenVault, TagEdit, page_tags};

use crate::VaultTags;

pub mod ids {
    pub const LIST: &str = "TAGS_LIST";
    pub const NOTES: &str = "TAGS_NOTES";
    pub const ADD: &str = "TAGS_ADD";
    pub const REMOVE: &str = "TAGS_REMOVE";
    pub const CLEAR: &str = "TAGS_CLEAR";
    pub const RENAME: &str = "TAGS_RENAME";
    pub const DELETE: &str = "TAGS_DELETE";
}

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Tags",
        method_name: id,
        display_name: name,
        description,
        category: "Tags",
        group: "Vault",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static LIST: ActionMeta = meta(
    ids::LIST,
    "List tags",
    "Every tag the vault's notes use (frontmatter `tags` and inline #tags, nested ones as \
     `a/b`), how many notes use each, and its icon and colour when the tag registry \
     (Records/tags.json) has one.",
);
static NOTES: ActionMeta = meta(
    ids::NOTES,
    "Notes with a tag",
    "The notes tagged `tag` (with or without the #) or a tag nested under it, in path order.",
);

static ADD: ActionMeta = meta(
    ids::ADD,
    "Tag notes",
    "Adds `tag` to the notes at `paths`, in their frontmatter's `tags` list. A note that has \
     the tag, or a tag above it, is left as it is; the tags below it are taken off first.",
);
static REMOVE: ActionMeta = meta(
    ids::REMOVE,
    "Untag notes",
    "Takes `tag` (any case; not the tags below it) off the notes at `paths`: from their \
     frontmatter and their text.",
);
static CLEAR: ActionMeta = meta(
    ids::CLEAR,
    "Clear notes' tags",
    "Takes every tag off the notes at `paths`: their `tags` property and their inline #tags.",
);
static RENAME: ActionMeta = meta(
    ids::RENAME,
    "Rename a tag",
    "Renames tag `from` to `to` in every note, and the tags below it with it (`a/b` to `x` \
     makes `a/b/c` `x/c`), in frontmatter and text. Answers the notes it changed.",
);
static DELETE: ActionMeta = meta(
    ids::DELETE,
    "Delete a tag",
    "Takes `tag` and the tags below it off every note, from frontmatter and text. Answers the \
     notes it changed.",
);

#[derive(Facet)]
struct TagArgs {
    tag: String,
}

#[derive(Facet)]
struct NotesTagArgs {
    paths: Vec<String>,
    tag: String,
}

#[derive(Facet)]
struct NotesArgs {
    paths: Vec<String>,
}

#[derive(Facet)]
struct RenameArgs {
    from: String,
    to: String,
}

fn changed(paths: &[String]) -> RawJson {
    RawJson::from(json!({ "changed": paths }).to_string())
}

/// Whether `tag` is `want` or nested under it.
fn under(tag: &str, want: &str) -> bool {
    tag.eq_ignore_ascii_case(want)
        || tag
            .get(..want.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(want))
            && tag.as_bytes().get(want.len()) == Some(&b'/')
}

/// Puts the `TAGS_*` actions on `list`, over `vault`.
///
/// # Errors
///
/// An id is taken.
pub fn register(list: &ActionList, vault: &OpenVault) -> Result<(), ActionError> {
    let v = vault.clone();
    list.register_with(&LIST, Effect::ReadOnly, move |_: &ActionCaller, (): ()| {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        v.with(|vault| {
            for page in &vault.pages {
                for tag in page_tags(page) {
                    let n = counts.entry(tag).or_default();
                    *n = n.saturating_add(1);
                }
            }
        });
        let looks = VaultTags::new(v.root())
            .ok()
            .and_then(|r| r.list_tags().ok())
            .unwrap_or_default();
        let tags: Vec<Value> = counts
            .into_iter()
            .map(|(name, notes)| {
                let look = looks.iter().find(|t| t.name.eq_ignore_ascii_case(&name));
                json!({
                    "tag": name,
                    "notes": notes,
                    "icon": look.map(|t| &t.icon),
                    "color": look.and_then(|t| t.color.as_deref()),
                })
            })
            .collect();
        Ok::<_, String>(RawJson::from(Value::Array(tags).to_string()))
    })?;
    let v = vault.clone();
    list.register_with(
        &NOTES,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: TagArgs| {
            let want = a.tag.trim().trim_start_matches('#').to_owned();
            let mut notes: Vec<Value> = v.with(|vault| {
                vault
                    .pages
                    .iter()
                    .filter(|p| page_tags(p).iter().any(|t| under(t, &want)))
                    .map(|p| json!({ "path": p.rel_path, "tags": page_tags(p) }))
                    .collect()
            });
            notes.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
            Ok::<_, String>(RawJson::from(Value::Array(notes).to_string()))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &ADD,
        Effect::Mutating,
        move |_: &ActionCaller, a: NotesTagArgs| {
            Ok::<_, String>(changed(&v.edit_tags(&a.paths, &TagEdit::Add(&a.tag))?))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &REMOVE,
        Effect::Mutating,
        move |_: &ActionCaller, a: NotesTagArgs| {
            Ok::<_, String>(changed(&v.edit_tags(&a.paths, &TagEdit::Remove(&a.tag))?))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &CLEAR,
        Effect::Mutating,
        move |_: &ActionCaller, a: NotesArgs| {
            Ok::<_, String>(changed(&v.edit_tags(&a.paths, &TagEdit::Clear)?))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &RENAME,
        Effect::Mutating,
        move |_: &ActionCaller, a: RenameArgs| {
            let edit = TagEdit::Rename {
                from: &a.from,
                to: &a.to,
            };
            Ok::<_, String>(changed(&v.edit_tags_everywhere(&edit)?))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &DELETE,
        Effect::Destructive,
        move |_: &ActionCaller, a: TagArgs| {
            Ok::<_, String>(changed(&v.edit_tags_everywhere(&TagEdit::Delete(&a.tag))?))
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::under;

    #[test]
    fn a_tag_holds_the_tags_nested_under_it() {
        assert!(under("lab", "lab"));
        assert!(under("Lab/sample", "lab"));
        assert!(!under("labs", "lab"));
        assert!(!under("la", "lab"));
    }
}
