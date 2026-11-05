//! The tags' actions (feature `actions`), over an open vault
//! ([`vault::actions::OpenVault`]): a note's tags are its frontmatter
//! `tags` and its inline `#tags` ([`vault::actions::page_tags`]), and a
//! tag's look (icon, colour) is the registry's (`Records/tags.json`,
//! [`crate::VaultTags`]).
//!
//! | action | does |
//! |---|---|
//! | `TAGS_LIST` | every tag the notes use, with how many notes use it and its look from the registry |
//! | `TAGS_NOTES {tag}` | the notes tagged `tag` or a tag nested under it (`lab` holds `lab/sample`) |

use std::collections::BTreeMap;

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use facet::Facet;
use serde_json::{Value, json};
use tag_proto::TagService;
use vault::actions::{OpenVault, page_tags};

use crate::VaultTags;

pub mod ids {
    pub const LIST: &str = "TAGS_LIST";
    pub const NOTES: &str = "TAGS_NOTES";
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

#[derive(Facet)]
struct TagArgs {
    tag: String,
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
                    *counts.entry(tag).or_default() += 1;
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
