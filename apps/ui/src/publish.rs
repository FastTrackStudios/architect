//! `PUBLISH_EXPORT` (#374): the app's vault, or one part of it, built as
//! a static site into a folder, for any static host. The same scoped
//! site a share link serves (`architect_sites::Scope`), as a snapshot:
//! no audience, expiry or live updates. `docs/content/guide/deploy.md`
//! ("Its own deployment").

use std::path::PathBuf;
use std::sync::Arc;

use architect::action::{ActionList, ActionMeta, Effect};
use architect_sites::Scope;
use facet::Facet;

static PUBLISH_EXPORT: ActionMeta = ActionMeta {
    id: "PUBLISH_EXPORT",
    trait_name: "",
    method_name: "",
    display_name: "Export as a site…",
    description: "Builds the app's vault as a static site into folder `out`: one part of it with `folder` (rooted there), `tag`, `note` / `notes` or `base` (a .base file), else the whole vault. `base_url` is where it will be served (default /), `url` its address for the sitemap, `title` its name. A link to a note outside the part is plain text. Answers the pages and files written and any problems.",
    category: "Share",
    group: "Share",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// What [`PUBLISH_EXPORT`] takes.
#[derive(Debug, Clone, Facet)]
pub struct ExportArgs {
    pub out: String,
    pub folder: Option<String>,
    pub tag: Option<String>,
    pub note: Option<String>,
    pub notes: Option<Vec<String>>,
    pub base: Option<String>,
    pub base_url: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
}

/// What [`PUBLISH_EXPORT`] answers.
#[derive(Debug, Clone, Facet)]
pub struct ExportOut {
    pub out: String,
    pub pages: u32,
    pub files: u32,
    pub problems: Vec<String>,
}

fn scope(args: &ExportArgs) -> Scope {
    let given = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    given(&args.folder)
        .map(Scope::Folder)
        .or_else(|| given(&args.tag).map(Scope::Tag))
        .or_else(|| given(&args.note).map(|n| Scope::Notes(vec![n])))
        .or_else(|| {
            args.notes
                .clone()
                .filter(|n| !n.is_empty())
                .map(Scope::Notes)
        })
        .or_else(|| given(&args.base).map(Scope::Base))
        .unwrap_or(Scope::Vault)
}

/// Put `PUBLISH_EXPORT` on `list`, over the vault at `open`.
pub fn register(list: &Arc<ActionList>, open: &vault::actions::OpenVault) {
    let open = open.clone();
    let _ = list.register(
        &PUBLISH_EXPORT,
        Effect::Mutating,
        move |args: ExportArgs| {
            if args.out.trim().is_empty() {
                return Err("say where: `out`, a folder".to_owned());
            }
            let out = PathBuf::from(args.out.trim());
            let mut opts =
                architect_sites::Options::new(open.root(), args.title.clone().unwrap_or_default());
            opts.base = args.base_url.clone().unwrap_or_else(|| "/".to_owned());
            opts.url.clone_from(&args.url);
            opts.scope = scope(&args);
            let report = architect_sites::build(&opts, &architect_sites::Blocks::new(), &out)
                .map_err(|e| e.to_string())?;
            let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
            Ok(ExportOut {
                out: report.out.display().to_string(),
                pages: count(report.pages),
                files: count(report.files),
                problems: report.problems.iter().map(ToString::to_string).collect(),
            })
        },
    );
}
