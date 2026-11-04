//! The pages' actions (feature `runner`), for a person in ⌘K and an agent
//! over the control socket or MCP:
//!
//! | action | does |
//! |---|---|
//! | `PAGES_STATUS` | every running page: its file, whether it is the page as saved (or why not), its renders, its last render and reload times, its log's tail |
//! | `PAGES_CHECK {file}` | loads a page file and renders it once: how many nodes it drew, or why not (with its line) |
//! | `PAGES_CLONE {extension, view}` | copies a built-in's read-only page to the override's place, to edit: the path |
//!
//! `PAGES_STATUS`'s problems are what the app's status (`APP_STATUS`,
//! #270) lists for pages; `PAGES_CHECK` is `EXT_CHECK`'s page half.

use std::path::PathBuf;

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use facet::Facet;
use serde_json::{Value, json};

use crate::files::{self, Origin, PageFile};
use crate::runner::Pages;

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Pages",
        method_name: id,
        display_name: name,
        description,
        category: "Pages",
        group: "Pages",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static STATUS: ActionMeta = meta(
    "PAGES_STATUS",
    "Pages' status",
    "Every running page (a view drawn from a TSX, JS, Luau or A2UI file): its file, whether it \
     is the page as saved or why not (the last good page shows, and <file>.error says why), \
     its renders, its last render and reload times, and its log's last lines.",
);
static CHECK: ActionMeta = meta(
    "PAGES_CHECK",
    "Check a page",
    "Loads a page file and renders it once, off the window: how many nodes it drew and how \
     long it took, or why it would not (with the file, line and column).",
);
static CLONE: ActionMeta = meta(
    "PAGES_CLONE",
    "Edit a built-in page",
    "Copies a built-in's read-only page file to <app dir>/pages/<extension>/<view>, where it \
     overrides the built-in live; delete it to bring the built-in back. Answers its path.",
);

#[derive(Facet)]
struct CheckArgs {
    /// The page file.
    file: String,
}

#[derive(Facet)]
struct CloneArgs {
    extension: String,
    view: String,
}

fn ms(d: std::time::Duration) -> f64 {
    (d.as_secs_f64() * 1000.0 * 100.0).round() / 100.0
}

/// Puts the `PAGES_*` actions on `list`, over `pages`.
///
/// # Errors
///
/// An id is taken.
pub fn install(list: &ActionList, pages: &Pages) -> Result<(), ActionError> {
    let p = pages.clone();
    list.register_with(&STATUS, Effect::ReadOnly, move |_: &ActionCaller, (): ()| {
        let running: Vec<Value> = p
            .running()
            .iter()
            .map(|s| {
                let log = s.log.lines();
                let tail = log.len().saturating_sub(10);
                json!({
                    "extension": s.extension,
                    "view": s.view,
                    "file": s.page.path,
                    "editable": s.page.editable(),
                    "error": s.error(),
                    "renders": s.renders(),
                    "reloads": s.reloads(),
                    "last_render_ms": ms(s.last_render()),
                    "last_reload_ms": ms(s.last_reload()),
                    "log": log.get(tail..).unwrap_or_default(),
                })
            })
            .collect();
        let problems: Vec<Value> = p
            .problems()
            .into_iter()
            .map(|pr| json!({ "extension": pr.extension, "view": pr.view, "file": pr.file, "error": pr.error }))
            .collect();
        Ok::<_, String>(RawJson::from(
            json!({ "running": running, "problems": problems, "app_dir": p.app_dir() }).to_string(),
        ))
    })?;
    let p = pages.clone();
    list.register_with(
        &CHECK,
        Effect::ReadOnly,
        move |_: &ActionCaller, args: CheckArgs| {
            let page =
                PageFile::new(PathBuf::from(&args.file), Origin::Override).ok_or_else(|| {
                    format!(
                        "{}: not a page file (.tsx, .ts, .jsx, .js, .luau)",
                        args.file
                    )
                })?;
            let checked = p.check(&page)?;
            Ok::<_, String>(RawJson::from(
                json!({ "file": args.file, "nodes": checked.nodes, "took_ms": ms(checked.took) })
                    .to_string(),
            ))
        },
    )?;
    let p = pages.clone();
    list.register_with(
        &CLONE,
        Effect::Mutating,
        move |_: &ActionCaller, args: CloneArgs| {
            let app = p
                .app_dir()
                .ok_or("this app keeps no pages folder")?
                .to_path_buf();
            let page = p.declared(&args.extension, &args.view).ok_or_else(|| {
                format!(
                    "{}'s view {} has no built-in page",
                    args.extension, args.view
                )
            })?;
            let cloned = files::clone_to_edit(&page, &app, &args.extension, &args.view)
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(RawJson::from(json!({ "path": cloned.path }).to_string()))
        },
    )?;
    Ok(())
}
