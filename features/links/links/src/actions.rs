//! The links' actions (feature `actions`): the wikilink graph of an open
//! vault ([`vault::actions::OpenVault`]), read with
//! `vault_obsidian::LinkIndex` (links resolve by vault path first, then by
//! file name; a name two notes share resolves to neither).
//!
//! | action | does |
//! |---|---|
//! | `LINKS_GRAPH` | every resolved link between notes, as `{from, to}` |
//! | `LINKS_BACK {path}` | the notes that link to `path` |
//! | `LINKS_UNRESOLVED` | the links that name no note, with the note they are in |
//! | `LINKS_ORPHANS` | the notes nothing links to |
//!
//! A note's own links out are `VAULT_READ`'s. Typed links (a relation, a
//! confidence, a visibility, between any two nodes) are
//! [`crate::Store`]'s, served as `links_proto::LinksService`.

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use facet::Facet;
use serde_json::{Value, json};
use vault::actions::OpenVault;
use vault::obsidian::LinkIndex;

pub mod ids {
    pub const GRAPH: &str = "LINKS_GRAPH";
    pub const BACK: &str = "LINKS_BACK";
    pub const UNRESOLVED: &str = "LINKS_UNRESOLVED";
    pub const ORPHANS: &str = "LINKS_ORPHANS";
}

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Links",
        method_name: id,
        display_name: name,
        description,
        category: "Links",
        group: "Vault",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static GRAPH: ActionMeta = meta(
    ids::GRAPH,
    "Link graph",
    "Every wikilink between two of the vault's notes, as {from, to} vault paths, in path \
     order: what a graph view draws.",
);
static BACK: ActionMeta = meta(
    ids::BACK,
    "Backlinks",
    "The notes whose wikilinks resolve to the note at `path`.",
);
static UNRESOLVED: ActionMeta = meta(
    ids::UNRESOLVED,
    "Unresolved links",
    "The wikilinks that name no note in the vault (or a name two notes share), with the note \
     each is in.",
);
static ORPHANS: ActionMeta = meta(
    ids::ORPHANS,
    "Orphan notes",
    "The notes no other note links to.",
);

#[derive(Facet)]
struct PathArgs {
    path: String,
}

fn answer(v: &Value) -> RawJson {
    RawJson::from(v.to_string())
}

/// Puts the `LINKS_*` actions on `list`, over `vault`.
///
/// # Errors
///
/// An id is taken.
pub fn register(list: &ActionList, vault: &OpenVault) -> Result<(), ActionError> {
    let v = vault.clone();
    list.register_with(&GRAPH, Effect::ReadOnly, move |_: &ActionCaller, (): ()| {
        let edges = v.with(|vault| {
            let links = LinkIndex::build(vault);
            let mut paths: Vec<&str> = vault.pages.iter().map(|p| p.rel_path.as_str()).collect();
            paths.sort_unstable();
            let edges: Vec<Value> = paths
                .iter()
                .flat_map(|from| {
                    links
                        .outgoing(from)
                        .into_iter()
                        .filter_map(|l| l.resolved)
                        .map(move |to| json!({ "from": from, "to": to }))
                })
                .collect();
            Value::Array(edges)
        });
        Ok::<_, String>(answer(&edges))
    })?;
    let v = vault.clone();
    list.register_with(
        &BACK,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: PathArgs| {
            v.with(|vault| {
                if vault.page(&a.path).is_none() {
                    return Err(format!("{}: no such note", a.path));
                }
                Ok(answer(&json!(LinkIndex::build(vault).backlinks(&a.path))))
            })
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &UNRESOLVED,
        Effect::ReadOnly,
        move |_: &ActionCaller, (): ()| {
            let out = v.with(|vault| {
                let links = LinkIndex::build(vault);
                let all: Vec<Value> = links
                    .unresolved()
                    .iter()
                    .map(|u| json!({ "from": u.source, "link": u.linkpath }))
                    .collect();
                Value::Array(all)
            });
            Ok::<_, String>(answer(&out))
        },
    )?;
    let v = vault.clone();
    list.register_with(
        &ORPHANS,
        Effect::ReadOnly,
        move |_: &ActionCaller, (): ()| {
            let out = v.with(|vault| json!(LinkIndex::build(vault).orphans()));
            Ok::<_, String>(answer(&out))
        },
    )?;
    Ok(())
}
