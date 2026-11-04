//! Pages in the extensions manager, and in `EXT_CHECK` (feature
//! `manager`).
//!
//! [`PagesKind`] is the manager's `pages` kind
//! (`architect_extensions::manager::ItemKind`): every view's page — a
//! built-in's read-only copy, an extension's, an override in the app's
//! folder — with how it runs, its problems and its frame times, and the
//! ops `check` (render it once), `clone` (a built-in's page to the
//! override's place, to edit) and `restore` (remove the override: the
//! view's own page, or its compiled view, shows again).
//! [`Pages::page_checker`] is `EXT_CHECK`'s page half
//! (`architect_extensions::check::Checker::with_page_checker`).

use architect_extensions::check::PageChecker;
use architect_extensions::manager::{Details, Item, ItemKind, Op, Part};
use architect_workspace::Meters;
use serde_json::{Value, json};

use crate::files::{self, Origin};
use crate::runner::{PageEntry, Pages};

fn ms(d: std::time::Duration) -> f64 {
    (d.as_secs_f64() * 1000.0 * 100.0).round() / 100.0
}

impl Pages {
    /// `EXT_CHECK`'s page half: a page file loaded and rendered once by
    /// these pages' runners, how many nodes it drew.
    #[must_use]
    pub fn page_checker(&self) -> PageChecker {
        let me = self.clone();
        PageChecker::new(move |path| {
            me.check_file(path)
                .map(|c| u32::try_from(c.nodes).unwrap_or(u32::MAX))
        })
    }
}

/// Every view's page (`pages`).
pub struct PagesKind {
    pages: Pages,
    meters: Option<Meters>,
}

impl PagesKind {
    #[must_use]
    pub const fn new(pages: Pages) -> Self {
        Self {
            pages,
            meters: None,
        }
    }

    /// Each page's frame times from `meters` (the app's): its page gets a
    /// "Frame health" part.
    #[must_use]
    pub fn with_meters(mut self, meters: Meters) -> Self {
        self.meters = Some(meters);
        self
    }

    fn entry(&self, id: &str) -> Option<PageEntry> {
        self.pages.entries().into_iter().find(|e| e.name() == id)
    }
}

const fn origin(e: &PageEntry) -> &'static str {
    match (e.overridden, e.shown.origin) {
        (true, _) | (_, Origin::Override) => "an override",
        (_, Origin::Builtin) => "built in",
        (_, Origin::Extension) => "its extension's",
    }
}

fn item(e: &PageEntry) -> Item {
    let mut i = Item::new("pages", e.name(), e.name());
    origin(e).clone_into(&mut i.source);
    let running = e.status.as_ref().map_or_else(
        || "not open".to_owned(),
        |s| format!("open, {} render(s)", s.renders()),
    );
    let file = e
        .shown
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    i.summary = format!("{} page · {file} · {running}", e.shown.kind.extension());
    i.problems = e.error.iter().cloned().collect();
    i.ops.push(Op::new("check", "Check"));
    if !e.overridden && !e.shown.editable() {
        i.ops.push(Op::new("clone", "Clone to edit"));
    }
    if e.overridden {
        let label = if e.own.is_some() {
            "Remove the override (back to its own page)"
        } else {
            "Remove the override"
        };
        i.ops.push(Op::new("restore", label).destructive());
    }
    i
}

impl ItemKind for PagesKind {
    fn id(&self) -> &'static str {
        "pages"
    }

    fn title(&self) -> &'static str {
        "Pages"
    }

    fn list(&self) -> Vec<Item> {
        self.pages.entries().iter().map(item).collect()
    }

    fn get(&self, id: &str) -> Option<Details> {
        let e = self.entry(id)?;
        let mut parts = vec![Part::new(
            "File",
            [
                Some(format!("{} ({})", e.shown.path.display(), origin(&e))),
                Some(if e.shown.editable() {
                    "Editable: saving it re-renders the view".to_owned()
                } else {
                    "Read-only: clone it to edit".to_owned()
                }),
                e.own
                    .as_ref()
                    .filter(|_| e.overridden)
                    .map(|own| format!("Overrides {}", own.path.display())),
            ]
            .into_iter()
            .flatten()
            .collect(),
        )];
        if let Some(error) = &e.error {
            parts.push(Part::new(
                "Why it is not the page as saved",
                vec![error.clone(), "The last good page shows".to_owned()],
            ));
        }
        parts.push(Part::new(
            "How it runs",
            e.status.as_ref().map_or_else(
                || vec!["Not open in a pane".to_owned()],
                |s| {
                    vec![format!(
                        "{} render(s) · last {} ms · {} reload(s) · last reload {} ms",
                        s.renders(),
                        ms(s.last_render()),
                        s.reloads(),
                        ms(s.last_reload())
                    )]
                },
            ),
        ));
        if let Some(meters) = &self.meters {
            let name = e.name();
            let lines: Vec<String> = meters
                .summaries()
                .into_iter()
                .filter(|(n, _)| n == &name)
                .map(|(_, s)| {
                    format!(
                        "p50 {} ms · p99 {} ms · {} slow of {} frames (budget {} ms){}",
                        ms(s.p50),
                        ms(s.p99),
                        s.slow,
                        s.frames,
                        ms(s.budget),
                        if s.busy { " · busy now" } else { "" }
                    )
                })
                .collect();
            parts.push(Part::new(
                "Frame health",
                if lines.is_empty() {
                    vec!["Not measured yet".to_owned()]
                } else {
                    lines
                },
            ));
        }
        if let Some(s) = &e.status {
            let log = s.log.lines();
            let tail = log.len().saturating_sub(8);
            parts.push(Part::new(
                "Recent log",
                log.get(tail..).unwrap_or_default().to_vec(),
            ));
        }
        let data = json!({
            "extension": e.extension,
            "view": e.view,
            "file": e.shown.path,
            "editable": e.shown.editable(),
            "overridden": e.overridden,
            "own": e.own.as_ref().map(|p| &p.path),
            "open": e.status.is_some(),
            "error": e.error,
        });
        Some(Details {
            item: item(&e),
            parts,
            data,
        })
    }

    fn run(&self, id: &str, op: &str, _args: &Value) -> Result<Value, String> {
        let e = self
            .entry(id)
            .ok_or_else(|| format!("no page {id} (MANAGER_LIST lists them)"))?;
        match op {
            "check" => {
                let checked = self.pages.check(&e.shown)?;
                Ok(
                    json!({ "file": e.shown.path, "nodes": checked.nodes, "took_ms": ms(checked.took) }),
                )
            }
            "clone" => {
                let app = self
                    .pages
                    .app_dir()
                    .ok_or("this app keeps no pages folder")?;
                if e.overridden || e.shown.editable() {
                    return Err(format!("{id}'s page is editable where it is"));
                }
                let cloned = files::clone_to_edit(&e.shown, app, &e.extension, &e.view)
                    .map_err(|err| err.to_string())?;
                Ok(json!({ "path": cloned.path }))
            }
            "restore" => {
                if !e.overridden {
                    return Err(format!("{id} has no override"));
                }
                files::remove_override(&e.shown).map_err(|err| err.to_string())?;
                Ok(json!({ "removed": e.shown.path, "shows": e.own.map(|p| p.path) }))
            }
            _ => Err(format!("pages have no op {op} (check, clone, restore)")),
        }
    }
}
