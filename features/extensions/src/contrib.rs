//! What an extension adds to the app's interface, past its actions,
//! settings and themes:
//!
//! openers for files, document kinds and links, document kinds, tabs,
//! status items, menu and context-menu entries, a phone's action-bar
//! buttons, toolbars ([`ToolbarDecl`]: in a dock or a pane's header, each
//! button an action, a toggle's showing it on or off), hooks, the shell's
//! slots it fills, and inline embeds that
//! draw a claimed block inside a document ([`EmbedDecl`])
//! (`docs/content/architecture/extensions.md`, "Contributions" and
//! "Inline embeds").
//!
//! A [`Contribution`] is the same data whichever way it arrives: a
//! manifest's tables (`[[tabs]]`, `[[status]]`, …, read into
//! [`Manifest::contributes`](crate::Manifest::contributes)), a process
//! guest's or a Luau script's at run time, an in-process extension's
//! builder. [`Contributions`] is the app's registry of them, by owner:
//! each owner's set is put as a diff ([`Contributions::put`]) and taken
//! off whole when it unloads ([`Contributions::withdraw`]), its runtime
//! state too ([`StateWrite`]).
//!
//! Runtime state (a status item's text, a sidebar badge, metadata) is
//! owned by its source (`ext:<id>`, `agent:<name>`), sequenced (a write
//! older than the last one kept is dropped), time-limited (`ttl_ms`), and
//! withdrawn with its source. The registry takes the clock as an
//! argument (`now_ms`): it reads none.
//!
//! The model is pure data, wasm-clean; the host puts manifests' sets in
//! as extensions load (`host::Host::contributions`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

/// The hook events an app raises (`[[hooks]] on = …`), past
/// `changed:<topic>` for any Action List topic.
pub const EVENTS: [&str; 7] = [
    "startup",
    "workspace.changed",
    "doc.opened",
    "theme.changed",
    "selection.changed",
    "extension.loaded",
    "extension.unloaded",
];

/// The shell's regions, as slots a view can fill (`[[slots]]`, a layout
/// file's `chrome`): the desk's ribbon, the left and right sidebars, the
/// title row's tab, the status bar, the title row.
pub const SLOTS: [&str; 6] = ["ribbon", "left", "right", "tabs", "status", "title"];

/// The targets a context-menu entry can be on, past `view:<kind>`,
/// `file:<extension>` and `doc:<kind>`.
pub const TARGETS: [&str; 6] = ["pane", "tab", "file", "doc", "link", "selection"];

/// Whether `on` is an event a hook can be on.
#[must_use]
pub fn known_event(on: &str) -> bool {
    EVENTS.contains(&on)
        || on
            .strip_prefix("changed:")
            .is_some_and(|topic| !topic.is_empty())
}

/// Whether `target` is one a context-menu entry can be on.
#[must_use]
pub fn known_target(target: &str) -> bool {
    TARGETS.contains(&target)
        || ["view:", "file:", "doc:"]
            .iter()
            .any(|p| target.strip_prefix(p).is_some_and(|rest| !rest.is_empty()))
}

/// An opener: ⌘O's (`kind`, `prefix` and `action`: a query in, items out;
/// `open`: an item picked).
///
/// Or one that claims files by extension, document kinds or links by
/// pattern (`files`, `doc_kinds`, `urls`) and opens what it claims in one of
/// its views (`view`) or with one of its actions (`action`, given `{path}` or
/// `{url}` and the invocation).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct OpenerDecl {
    /// Unique among its openers; a ⌘O opener's is its `kind` when unset.
    pub id: String,
    /// ⌘O: what its items are (`click`).
    pub kind: String,
    pub title: String,
    /// ⌘O: what a query starts with to ask only this opener (`click:`).
    pub prefix: String,
    /// What runs it: ⌘O's query action, or a claim's open action.
    ///
    /// ⌘O's is given `{query}` and answers `{items: [{id, title,
    /// detail?}]}`, best first.
    pub action: String,
    /// ⌘O: the action that opens an item picked, given `{id}`, and `in`
    /// (`here`, `tab` or `split`) when its params name it. Unset, `action`
    /// is, given `{open: id}` (and `in`).
    pub open: String,
    /// The file extensions it claims, without the dot (`kf`, `tar.gz`).
    pub files: Vec<String>,
    /// The document kinds it claims (`keyflow.chart`).
    pub doc_kinds: Vec<String>,
    /// The links it claims: regular expressions over the whole URL.
    pub urls: Vec<String>,
    /// Its view that shows what it opens, given the path or URL as the
    /// pane's state.
    pub view: Option<String>,
}

impl OpenerDecl {
    /// Its id: its own, else its `kind`.
    #[must_use]
    pub fn key(&self) -> &str {
        if self.id.is_empty() {
            &self.kind
        } else {
            &self.id
        }
    }

    /// Whether it claims files, document kinds or links (rather than
    /// answering ⌘O).
    #[must_use]
    pub const fn claims(&self) -> bool {
        !(self.files.is_empty() && self.doc_kinds.is_empty() && self.urls.is_empty())
    }

    /// Whether it claims the file at `path`, by its extension (any case).
    #[must_use]
    pub fn claims_path(&self, path: &str) -> bool {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let lower = name.to_ascii_lowercase();
        self.files.iter().any(|ext| {
            let ext = ext.trim_start_matches('.').to_ascii_lowercase();
            lower.len() > ext.len().saturating_add(1)
                && lower.ends_with(&ext)
                && lower
                    .as_bytes()
                    .get(lower.len().saturating_sub(ext.len().saturating_add(1)))
                    == Some(&b'.')
        })
    }
}

/// A kind of document it registers at run time: its stamp, its name, the
/// file extensions it keeps, the view that opens it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct DocKindDecl {
    /// The kind's stamp (`keyflow.chart`).
    pub id: String,
    pub name: String,
    pub files: Vec<String>,
    pub icon: String,
    /// Its view that shows one.
    pub view: Option<String>,
}

#[cfg(feature = "crdt")]
impl DocKindDecl {
    /// As the document layer takes it (no schema of its own yet: stamp 0).
    #[must_use]
    pub fn to_doc_kind(&self) -> crdt::DocKind {
        let kind = crdt::DocKind::new(&self.id, &self.name);
        match self.files.first() {
            Some(ext) => kind.extension(ext.trim_start_matches('.')),
            None => kind,
        }
    }
}

/// A tab (an app section) it adds: in a desk's ribbon, a tablet's
/// sidebar, a phone's tab bar; showing its view.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct TabDecl {
    pub id: String,
    pub title: String,
    /// A lucide icon's name (`puzzle`).
    pub icon: String,
    /// Its view the tab shows.
    pub view: String,
}

/// Which end of the status bar an item keeps to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusSide {
    Left,
    #[default]
    Right,
}

/// A status item it adds. Its text can change at run time: a
/// [`StateWrite`] with the item's id as its key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct StatusDecl {
    pub id: String,
    pub text: String,
    pub icon: String,
    pub hint: String,
    pub side: StatusSide,
    /// What a press on it runs.
    pub action: String,
}

/// An entry in a named menu of the menu bar (`File`, `View`, `Help`, or
/// one of the app's own), running one of its actions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct MenuDecl {
    /// The menu's title.
    pub menu: String,
    pub action: String,
}

/// An entry in a target's context menu (`pane`, `tab`, `view:<kind>`,
/// `file`, `file:<extension>`, `doc:<kind>`, `link`, `selection`): its
/// action runs with the target in its invocation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ContextDecl {
    pub target: String,
    pub action: String,
    /// Its row's label; the action's name when unset.
    pub title: String,
}

impl ContextDecl {
    /// Whether it is offered on `target` (`view:files`) whose id is `id`
    /// (a pane id, a file's path): `pane` is on every pane, `view:<kind>`
    /// on that kind's; `file` on every file, `file:<ext>` on that
    /// extension's.
    #[must_use]
    pub fn on(&self, target: &str, id: &str) -> bool {
        let mine = self.target.as_str();
        if mine == target || (mine == "pane" && target.starts_with("view:")) {
            return true;
        }
        mine.strip_prefix("file:").is_some_and(|ext| {
            target == "file"
                && OpenerDecl {
                    files: vec![ext.to_owned()],
                    ..OpenerDecl::default()
                }
                .claims_path(id)
        })
    }
}

/// A button on a phone's action bar, running one of its actions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct BarDecl {
    pub action: String,
    pub title: String,
    pub icon: String,
}

pub use crate::toolbar::{
    DOCKS, MenuItem, MenuKind, ToolColors, ToolItem, ToolKind, ToolbarDecl, ToolbarPlace,
};

/// A hook: on an app event (`startup`, `workspace.changed`, `doc.opened`,
/// `theme.changed`, `selection.changed`, `changed:<topic>`).
///
/// It runs one of its actions, or a Luau script of its package (the whole
/// file, or `function` of the table it returns), with the event in its
/// invocation and as `args`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct HookDecl {
    pub on: String,
    pub action: Option<String>,
    pub script: Option<String>,
    pub function: Option<String>,
}

/// A shell region it fills with one of its views (`status`: the status
/// bar is its view). A layout file's own assignment wins over it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SlotDecl {
    pub slot: String,
    pub view: String,
}

/// What an inline embed claims: a fenced block's language, a note's
/// `type:`, an embed target's type. Matched without regard to case.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct EmbedClaims {
    /// Fence languages: the first word of a fenced block's info string
    /// (`kf` for ```` ```kf ````).
    pub fence: Vec<String>,
    /// A note's frontmatter `type:` (`song`): the embed draws at the
    /// note's head.
    pub note_type: Vec<String>,
    /// The type of what an embed (`![[…]]`) points at (`setlist`).
    pub embed_type: Vec<String>,
}

impl EmbedClaims {
    /// Whether it claims nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.fence.is_empty() && self.note_type.is_empty() && self.embed_type.is_empty()
    }

    /// Every claim it makes.
    #[must_use]
    pub fn all(&self) -> Vec<Claim> {
        self.fence
            .iter()
            .map(|f| Claim::Fence(f.clone()))
            .chain(self.note_type.iter().map(|t| Claim::NoteType(t.clone())))
            .chain(self.embed_type.iter().map(|t| Claim::EmbedType(t.clone())))
            .collect()
    }
}

/// A block of a document an embed may claim
/// (`docs/content/architecture/extensions.md`, "Inline embeds").
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum Claim {
    /// A fenced block, by its language (`kf`).
    Fence(String),
    /// A note, by its frontmatter `type:` (`song`).
    NoteType(String),
    /// An embed's target, by its type (`setlist`).
    EmbedType(String),
}

impl Claim {
    /// Its kind's word: `fence`, `note_type`, `embed_type`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Fence(_) => "fence",
            Self::NoteType(_) => "note_type",
            Self::EmbedType(_) => "embed_type",
        }
    }

    /// What it names (`kf`, `song`).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Fence(n) | Self::NoteType(n) | Self::EmbedType(n) => n,
        }
    }

    /// A fenced block's claim from its info string (```` ```kf title ````):
    /// its first word. `None` for a fence with no language.
    #[must_use]
    pub fn fence(info: &str) -> Option<Self> {
        info.split_whitespace()
            .next()
            .map(|lang| Self::Fence(lang.to_owned()))
    }

    /// Reads `fence:kf`, `note_type:song`, `embed_type:setlist`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (kind, name) = text.split_once(':')?;
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        match kind.trim() {
            "fence" => Some(Self::Fence(name.to_owned())),
            "note_type" | "type" => Some(Self::NoteType(name.to_owned())),
            "embed_type" | "embed" => Some(Self::EmbedType(name.to_owned())),
            _ => None,
        }
    }
}

impl std::fmt::Display for Claim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind(), self.name())
    }
}

/// An inline embed: one of its views, or a page file, drawn inside a
/// host's document where a block it claims is, sized to its content.
///
/// ```toml
/// [[embeds]]
/// id = "kf"
/// title = "Keyflow chart"
/// page = "pages/kf.tsx"             # or view = "chart" (one of its [[views]])
/// claims = { fence = ["kf"], note_type = ["song"] }
/// max_height = 480                  # past it, the embed scrolls
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct EmbedDecl {
    /// Unique among its embeds.
    pub id: String,
    pub title: String,
    /// One of its views that draws it (a linked view mounts in the host's
    /// document; a process's or a page's as a sub-document).
    pub view: Option<String>,
    /// A page file in its package that draws it (TSX/JS, `.luau`), as a
    /// sub-document.
    pub page: Option<String>,
    pub claims: EmbedClaims,
    /// The most it grows to, points; past it the embed scrolls.
    pub max_height: Option<u32>,
}

impl EmbedDecl {
    /// Whether it claims `claim` (any case).
    #[must_use]
    pub fn claims(&self, claim: &Claim) -> bool {
        let list = match claim {
            Claim::Fence(_) => &self.claims.fence,
            Claim::NoteType(_) => &self.claims.note_type,
            Claim::EmbedType(_) => &self.claims.embed_type,
        };
        list.iter().any(|n| n.eq_ignore_ascii_case(claim.name()))
    }

    /// The view name its page and meter go by: its `view`, else
    /// `embed.<id>`.
    #[must_use]
    pub fn view_name(&self) -> String {
        self.view
            .clone()
            .unwrap_or_else(|| format!("embed.{}", self.id))
    }
}

/// What a host gives an embed: the claim it drew, the block's text, and
/// the host's own properties (a note's frontmatter, a fence's info
/// string), as JSON.
///
/// A linked view reads it with
/// `linked::use_embed`, a process view the same, a page with
/// `useEmbed()` (Luau: `embed()`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct EmbedInput {
    /// The claim, as text (`fence:kf`).
    pub claim: String,
    /// The block's text: a fence's body, a note's.
    pub source: String,
    /// The host's properties for it (`null` when none).
    pub props: serde_json::Value,
}

/// Everything an extension adds to the interface past actions, settings,
/// views and themes: a manifest's tables, or the same sent at run time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Contribution {
    pub openers: Vec<OpenerDecl>,
    pub doc_kinds: Vec<DocKindDecl>,
    pub tabs: Vec<TabDecl>,
    pub status: Vec<StatusDecl>,
    pub menus: Vec<MenuDecl>,
    pub context: Vec<ContextDecl>,
    pub bar: Vec<BarDecl>,
    pub toolbars: Vec<ToolbarDecl>,
    pub hooks: Vec<HookDecl>,
    pub slots: Vec<SlotDecl>,
    pub embeds: Vec<EmbedDecl>,
}

/// What a contribution is checked against: its owner's namespace and
/// capabilities, its views, its kind.
#[derive(Clone, Debug)]
pub struct Rules<'a> {
    /// Its own action prefix (`LAB_CONTRIB_`).
    pub prefix: String,
    /// The action prefixes its capabilities name (`*` for all).
    pub reach: &'a [String],
    /// Its views' ids.
    pub views: &'a [String],
    /// Whether it runs Luau (a hook's script).
    pub luau: bool,
}

/// Why a contribution was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    /// What it was (`opener kf`, `hook startup`).
    pub what: String,
    pub why: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.what, self.why)
    }
}

impl std::error::Error for Refused {}

impl Contribution {
    /// Nothing in it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// `self` and `other` together (a manifest's set and what was added
    /// at run time): an id in both is `other`'s.
    #[must_use]
    pub fn merged(&self, other: &Self) -> Self {
        fn join<T: Clone>(a: &[T], b: &[T], key: impl Fn(&T) -> String) -> Vec<T> {
            let theirs: BTreeSet<String> = b.iter().map(&key).collect();
            a.iter()
                .filter(|x| !theirs.contains(&key(x)))
                .chain(b)
                .cloned()
                .collect()
        }
        Self {
            openers: join(&self.openers, &other.openers, |o| o.key().to_owned()),
            doc_kinds: join(&self.doc_kinds, &other.doc_kinds, |d| d.id.clone()),
            tabs: join(&self.tabs, &other.tabs, |t| t.id.clone()),
            status: join(&self.status, &other.status, |s| s.id.clone()),
            menus: join(&self.menus, &other.menus, |m| {
                format!("{}/{}", m.menu, m.action)
            }),
            context: join(&self.context, &other.context, |c| {
                format!("{}/{}", c.target, c.action)
            }),
            bar: join(&self.bar, &other.bar, |b| b.action.clone()),
            toolbars: join(&self.toolbars, &other.toolbars, |t| t.id.clone()),
            hooks: join(&self.hooks, &other.hooks, hook_key),
            slots: join(&self.slots, &other.slots, |s| s.slot.clone()),
            embeds: join(&self.embeds, &other.embeds, |e| e.id.clone()),
        }
    }

    /// Every item's key (`tab:board`, `status:clock`): what a diff
    /// compares.
    #[must_use]
    pub fn keys(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        out.extend(self.openers.iter().map(|o| format!("opener:{}", o.key())));
        out.extend(self.doc_kinds.iter().map(|d| format!("doc_kind:{}", d.id)));
        out.extend(self.tabs.iter().map(|t| format!("tab:{}", t.id)));
        out.extend(self.status.iter().map(|s| format!("status:{}", s.id)));
        out.extend(
            self.menus
                .iter()
                .map(|m| format!("menu:{}/{}", m.menu, m.action)),
        );
        out.extend(
            self.context
                .iter()
                .map(|c| format!("context:{}/{}", c.target, c.action)),
        );
        out.extend(self.bar.iter().map(|b| format!("bar:{}", b.action)));
        out.extend(self.toolbars.iter().map(|t| format!("toolbar:{}", t.id)));
        out.extend(self.hooks.iter().map(|h| format!("hook:{}", hook_key(h))));
        out.extend(self.slots.iter().map(|s| format!("slot:{}", s.slot)));
        out.extend(self.embeds.iter().map(|e| format!("embed:{}", e.id)));
        out
    }

    /// Checks it against its owner's rules: each id once, each action its
    /// own or one its capabilities reach, each view one it has, each
    /// event, target, slot and pattern one there is.
    ///
    /// # Errors
    ///
    /// The first thing it gets wrong.
    pub fn check(&self, rules: &Rules<'_>) -> Result<(), Refused> {
        let refuse = |what: String, why: String| Err(Refused { what, why });
        let action = |what: &str, id: &str| -> Result<(), Refused> {
            if id.is_empty() {
                return refuse(what.to_owned(), "it names no action".to_owned());
            }
            let reached = id.starts_with(rules.prefix.as_str())
                || rules
                    .reach
                    .iter()
                    .any(|p| p == "*" || id.starts_with(p.as_str()));
            if reached {
                Ok(())
            } else {
                refuse(
                    what.to_owned(),
                    format!(
                        "{id} is neither its own ({}…) nor one its capabilities reach",
                        rules.prefix
                    ),
                )
            }
        };
        let view = |what: &str, id: &str| -> Result<(), Refused> {
            if rules.views.iter().any(|v| v == id) {
                Ok(())
            } else {
                refuse(
                    what.to_owned(),
                    format!("it has no view {id:?} ([[views]])"),
                )
            }
        };
        let keys = self.keys();
        let count = [
            self.openers.len(),
            self.doc_kinds.len(),
            self.tabs.len(),
            self.status.len(),
            self.menus.len(),
            self.context.len(),
            self.bar.len(),
            self.toolbars.len(),
            self.hooks.len(),
            self.slots.len(),
            self.embeds.len(),
        ]
        .into_iter()
        .fold(0_usize, usize::saturating_add);
        if keys.len() != count {
            return refuse(
                "its contributions".to_owned(),
                "two of one kind share an id".to_owned(),
            );
        }
        for o in &self.openers {
            let what = format!("opener {}", o.key());
            if o.key().is_empty() {
                return refuse(what, "it has no id (or kind)".to_owned());
            }
            if o.claims() {
                match (&o.view, o.action.is_empty()) {
                    (Some(v), _) => view(&what, v)?,
                    (None, false) => action(&what, &o.action)?,
                    (None, true) => {
                        return refuse(what, "it opens with neither a view nor an action".into());
                    }
                }
                for p in &o.urls {
                    if let Err(e) = regex::Regex::new(p) {
                        return refuse(what, format!("{p:?} is not a pattern: {e}"));
                    }
                }
                if o.files.iter().any(|f| f.trim_start_matches('.').is_empty()) {
                    return refuse(what, "an empty file extension".to_owned());
                }
            } else {
                if o.kind.is_empty() {
                    return refuse(
                        what,
                        "a ⌘O opener needs a kind; one that claims files, documents or links \
                         names them (files, doc_kinds, urls)"
                            .to_owned(),
                    );
                }
                action(&what, &o.action)?;
                if !o.open.is_empty() {
                    action(&what, &o.open)?;
                }
            }
        }
        for d in &self.doc_kinds {
            let what = format!("doc_kind {}", d.id);
            if !d.id.contains('.') || d.id.starts_with('.') || d.id.ends_with('.') {
                return refuse(what, "its id is `<app>.<kind>` (keyflow.chart)".to_owned());
            }
            if let Some(v) = &d.view {
                view(&what, v)?;
            }
        }
        for t in &self.tabs {
            let what = format!("tab {}", t.id);
            if t.id.is_empty() || t.title.is_empty() {
                return refuse(what, "it needs an id and a title".to_owned());
            }
            view(&what, &t.view)?;
        }
        for s in &self.status {
            let what = format!("status {}", s.id);
            if s.id.is_empty() {
                return refuse(what, "it needs an id".to_owned());
            }
            if !s.action.is_empty() {
                action(&what, &s.action)?;
            }
        }
        for m in &self.menus {
            let what = format!("menu {}", m.menu);
            if m.menu.is_empty() {
                return refuse(what, "it names no menu".to_owned());
            }
            action(&what, &m.action)?;
        }
        for c in &self.context {
            let what = format!("context {}", c.target);
            if !known_target(&c.target) {
                return refuse(
                    what,
                    format!(
                        "not a target ({}, view:<kind>, file:<ext>, doc:<kind>)",
                        TARGETS.join(", ")
                    ),
                );
            }
            action(&what, &c.action)?;
        }
        for b in &self.bar {
            action(&format!("bar {}", b.action), &b.action)?;
        }
        for t in &self.toolbars {
            t.check(
                |id| action(&format!("toolbar {}", t.id), id).map_err(|r| r.why),
                |id| view(&format!("toolbar {}", t.id), id).map_err(|r| r.why),
            )
            .map_err(|why| Refused {
                what: format!("toolbar {}", t.id),
                why,
            })?;
        }
        for h in &self.hooks {
            let what = format!("hook {}", h.on);
            if !known_event(&h.on) {
                return refuse(
                    what,
                    format!("not an event ({}, changed:<topic>)", EVENTS.join(", ")),
                );
            }
            match (&h.action, &h.script) {
                (Some(a), None) => action(&what, a)?,
                (None, Some(path)) if rules.luau => {
                    if !inside(path) {
                        return refuse(what, format!("{path} is outside the package"));
                    }
                }
                (None, Some(_)) => {
                    return refuse(what, "a script hook needs a luau extension".to_owned());
                }
                _ => return refuse(what, "it runs one action or one script".to_owned()),
            }
        }
        for s in &self.slots {
            let what = format!("slot {}", s.slot);
            if !SLOTS.contains(&s.slot.as_str()) {
                return refuse(what, format!("not a slot ({})", SLOTS.join(", ")));
            }
            view(&what, &s.view)?;
        }
        for e in &self.embeds {
            let what = format!("embed {}", e.id);
            if e.id.is_empty() {
                return refuse(what, "it needs an id".to_owned());
            }
            match (&e.view, &e.page) {
                (Some(v), None) => view(&what, v)?,
                (None, Some(page)) => {
                    if !inside(page) {
                        return refuse(what, format!("{page} is outside the package"));
                    }
                }
                _ => return refuse(what, "it is drawn by one view or one page".to_owned()),
            }
            if e.claims.is_empty() {
                return refuse(
                    what,
                    "it claims nothing (claims = { fence, note_type, embed_type })".to_owned(),
                );
            }
            if e.claims.all().iter().any(|c| c.name().trim().is_empty()) {
                return refuse(what, "an empty claim".to_owned());
            }
            if e.max_height == Some(0) {
                return refuse(what, "its max_height is 0".to_owned());
            }
        }
        Ok(())
    }
}

fn hook_key(h: &HookDecl) -> String {
    format!(
        "{}/{}",
        h.on,
        h.action
            .clone()
            .or_else(|| {
                h.script
                    .as_ref()
                    .map(|s| format!("{s}#{}", h.function.as_deref().unwrap_or("")))
            })
            .unwrap_or_default()
    )
}

/// A relative path that stays in its package.
fn inside(path: &str) -> bool {
    !path.is_empty()
        && std::path::Path::new(path)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// What a put or a withdrawal changed, by item key (`tab:board`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Diff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub kept: Vec<String>,
}

impl Diff {
    fn between(before: &BTreeSet<String>, after: &BTreeSet<String>) -> Self {
        Self {
            added: after.difference(before).cloned().collect(),
            removed: before.difference(after).cloned().collect(),
            kept: before.intersection(after).cloned().collect(),
        }
    }
}

/// What kind of runtime state a write is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StateKind {
    /// A status item's text (its key the item's id: one it declared, or a
    /// new one).
    #[default]
    Status,
    /// A badge on a target (a tab, a pane, a sidebar row: `target`).
    Badge,
    /// Metadata on a target, for its views to read.
    Meta,
    /// Whether one of its toggles is on: its key the action's id (one it
    /// declared `toggle = true`), its text `on` (or `true`) for on,
    /// anything else for off. What the action's tick in a menu and its
    /// toolbar button show ([`Contributions::checked`]).
    Checked,
}

/// One write of runtime state, by its source.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct StateWrite {
    /// Who owns it: `ext:<id>` (an extension's, withdrawn when it
    /// unloads), `agent:<name>`, `app`.
    pub source: String,
    pub key: String,
    pub kind: StateKind,
    /// What it says: normalized to one line of at most [`MAX_TEXT`]
    /// characters.
    pub text: String,
    pub icon: String,
    /// What it is on (`tab:inbox`, `pane:note-1`), for a badge or meta.
    pub target: String,
    /// Its place in its source's writes: one at or below the last kept
    /// for its key is stale, and dropped. `0` (unset) always applies.
    pub seq: u64,
    /// How long it lasts, ms; `0` for until it is cleared.
    pub ttl_ms: u64,
}

/// The longest text runtime state keeps.
pub const MAX_TEXT: usize = 80;

/// The most keys one source keeps.
pub const MAX_KEYS: usize = 64;

/// Runtime state as kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StateEntry {
    pub source: String,
    pub key: String,
    pub kind: StateKind,
    pub text: String,
    pub icon: String,
    pub target: String,
    pub seq: u64,
    /// When it lapses (ms, the registry's clock); `None` for never.
    pub expires_ms: Option<u64>,
}

/// What a state write did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Applied {
    /// Kept.
    Kept,
    /// Older than what is kept for its key: dropped.
    Stale,
}

/// Why a state write was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateError {
    /// No source, or no key.
    Unnamed,
    /// Its source keeps [`MAX_KEYS`] already.
    Full { source: String },
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unnamed => write!(f, "runtime state needs a source and a key"),
            Self::Full { source } => write!(f, "{source} keeps {MAX_KEYS} keys already"),
        }
    }
}

impl std::error::Error for StateError {}

/// Text as runtime state keeps it: one line, spaces collapsed, at most
/// [`MAX_TEXT`] characters.
#[must_use]
pub fn normalize(text: &str) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(MAX_TEXT).collect()
}

/// One owner's contributions, as kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Owned {
    /// The extension's id (or another owner's name).
    pub owner: String,
    /// What it declared (its manifest's tables, its builder's).
    pub declared: Contribution,
    /// What it added at run time, on top.
    pub runtime: Contribution,
    /// The order owners came in: a later one's slot claim wins.
    pub order: u64,
}

impl Owned {
    /// Its contribution now: declared, with the runtime one over it.
    #[must_use]
    pub fn now(&self) -> Contribution {
        self.declared.merged(&self.runtime)
    }
}

/// An item and who added it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct By<T> {
    pub owner: String,
    pub item: T,
}

type Listener = Arc<dyn Fn(u64) + Send + Sync>;

#[derive(Default)]
struct Inner {
    owners: BTreeMap<String, Owned>,
    state: BTreeMap<(String, String), StateEntry>,
    revision: u64,
    next_order: u64,
    listeners: Vec<(u64, Listener)>,
    next_listener: u64,
}

/// The app's contributions, by owner, and the runtime state beside them.
/// Cheap to clone; `Send`, so a host on any thread puts into it and the
/// window reads it.
#[derive(Clone, Default)]
pub struct Contributions {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for Contributions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("Contributions")
            .field("owners", &inner.owners.keys().collect::<Vec<_>>())
            .field("revision", &inner.revision)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Contributions {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Calls its listener while it is held.
pub struct Subscription {
    inner: std::sync::Weak<Mutex<Inner>>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .listeners
                .retain(|(id, _)| *id != self.id);
        }
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Subscription").field(&self.id).finish()
    }
}

impl Contributions {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Bumps the revision and tells the listeners, outside the lock.
    fn changed(mut inner: std::sync::MutexGuard<'_, Inner>) {
        inner.revision = inner.revision.wrapping_add(1);
        let revision = inner.revision;
        let listeners: Vec<Listener> = inner.listeners.iter().map(|(_, l)| l.clone()).collect();
        drop(inner);
        for l in listeners {
            l(revision);
        }
    }

    /// Calls `listener` with the revision after every change, while the
    /// subscription is held.
    #[must_use = "the listener is called while the subscription is held"]
    pub fn subscribe(&self, listener: impl Fn(u64) + Send + Sync + 'static) -> Subscription {
        let mut inner = self.lock();
        let id = inner.next_listener;
        inner.next_listener = id.wrapping_add(1);
        inner.listeners.push((id, Arc::new(listener)));
        drop(inner);
        Subscription {
            inner: Arc::downgrade(&self.inner),
            id,
        }
    }

    /// Moves on with every change.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// Puts what `owner` declared, in place of what it declared before:
    /// what changed. Its runtime additions stay.
    // What changed is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn put(&self, owner: &str, declared: Contribution) -> Diff {
        let mut inner = self.lock();
        let order = inner.next_order;
        let before = inner
            .owners
            .get(owner)
            .map(|o| o.now().keys())
            .unwrap_or_default();
        let entry = inner
            .owners
            .entry(owner.to_owned())
            .or_insert_with(|| Owned {
                owner: owner.to_owned(),
                declared: Contribution::default(),
                runtime: Contribution::default(),
                order,
            });
        entry.declared = declared;
        let after = entry.now().keys();
        if entry.order == order {
            inner.next_order = order.wrapping_add(1);
        }
        let diff = Diff::between(&before, &after);
        Self::changed(inner);
        diff
    }

    /// Puts what `owner` adds at run time (a process guest's, a script's),
    /// in place of what it added before: what changed. `None` when it is
    /// not loaded (nothing is put).
    // What changed is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    // The guard goes to `changed`, which tells the listeners after it.
    #[allow(clippy::significant_drop_tightening)]
    pub fn put_runtime(&self, owner: &str, runtime: Contribution) -> Option<Diff> {
        let mut inner = self.lock();
        let entry = inner.owners.get_mut(owner)?;
        let before = entry.now().keys();
        entry.runtime = runtime;
        let diff = Diff::between(&before, &entry.now().keys());
        Self::changed(inner);
        Some(diff)
    }

    /// Takes everything of `owner` off, and its runtime state
    /// (`ext:<owner>`'s): what went.
    // What changed is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn withdraw(&self, owner: &str) -> Diff {
        let mut inner = self.lock();
        let before = inner
            .owners
            .remove(owner)
            .map(|o| o.now().keys())
            .unwrap_or_default();
        let source = format!("ext:{owner}");
        inner.state.retain(|(s, _), _| *s != source);
        let diff = Diff::between(&before, &BTreeSet::new());
        Self::changed(inner);
        diff
    }

    /// The owners there are, in the order they came.
    #[must_use]
    pub fn owners(&self) -> Vec<Owned> {
        let mut out: Vec<Owned> = self.lock().owners.values().cloned().collect();
        out.sort_by_key(|o| o.order);
        out
    }

    /// `owner`'s contribution now.
    #[must_use]
    pub fn of(&self, owner: &str) -> Option<Contribution> {
        self.lock().owners.get(owner).map(Owned::now)
    }

    /// Every item of one kind, with its owner, in the order owners came.
    fn all<T: Clone>(&self, pick: impl Fn(&Contribution) -> Vec<T>) -> Vec<By<T>> {
        self.owners()
            .into_iter()
            .flat_map(|o| {
                let owner = o.owner.clone();
                pick(&o.now()).into_iter().map(move |item| By {
                    owner: owner.clone(),
                    item,
                })
            })
            .collect()
    }

    #[must_use]
    pub fn tabs(&self) -> Vec<By<TabDecl>> {
        self.all(|c| c.tabs.clone())
    }

    /// Every status item, its text the latest runtime state's for its key
    /// (`now_ms` drops a lapsed one), then the runtime-only ones.
    #[must_use]
    pub fn status(&self, now_ms: u64) -> Vec<By<StatusDecl>> {
        let state = self.state(now_ms);
        let mut out = self.all(|c| c.status.clone());
        for by in &mut out {
            let source = format!("ext:{}", by.owner);
            if let Some(s) = state
                .iter()
                .find(|s| s.kind == StateKind::Status && s.source == source && s.key == by.item.id)
            {
                by.item.text.clone_from(&s.text);
                if !s.icon.is_empty() {
                    by.item.icon.clone_from(&s.icon);
                }
            }
        }
        for s in state.iter().filter(|s| s.kind == StateKind::Status) {
            let declared = out
                .iter()
                .any(|b| format!("ext:{}", b.owner) == s.source && b.item.id == s.key);
            if !declared {
                out.push(By {
                    owner: s.source.clone(),
                    item: StatusDecl {
                        id: format!("{}/{}", s.source, s.key),
                        text: s.text.clone(),
                        icon: s.icon.clone(),
                        ..StatusDecl::default()
                    },
                });
            }
        }
        out
    }

    #[must_use]
    pub fn menus(&self, menu: &str) -> Vec<By<MenuDecl>> {
        self.all(|c| c.menus.clone())
            .into_iter()
            .filter(|m| m.item.menu == menu)
            .collect()
    }

    /// The context-menu entries on `target` (`view:files`, `file`) whose
    /// id is `id`.
    #[must_use]
    pub fn context(&self, target: &str, id: &str) -> Vec<By<ContextDecl>> {
        self.all(|c| c.context.clone())
            .into_iter()
            .filter(|c| c.item.on(target, id))
            .collect()
    }

    #[must_use]
    pub fn bar(&self) -> Vec<By<BarDecl>> {
        self.all(|c| c.bar.clone())
    }

    /// Every toolbar, its owner's.
    #[must_use]
    pub fn toolbars(&self) -> Vec<By<ToolbarDecl>> {
        self.all(|c| c.toolbars.clone())
    }

    /// The toolbars in the header of a pane of view kind `kind` (`note`):
    /// the ones on every pane, and that kind's.
    #[must_use]
    pub fn header_toolbars(&self, kind: &str) -> Vec<By<ToolbarDecl>> {
        self.all(|c| c.toolbars.clone())
            .into_iter()
            .filter(|t| t.item.in_header_of(kind))
            .collect()
    }

    /// Whether `owner`'s toggle `action` is on now: its last
    /// [`StateKind::Checked`] write for the action said `on` (or `true`);
    /// off when it wrote none, or that write lapsed.
    #[must_use]
    pub fn checked(&self, owner: &str, action: &str, now_ms: u64) -> bool {
        let source = format!("ext:{owner}");
        self.lock()
            .state
            .get(&(source, action.to_owned()))
            .filter(|e| e.kind == StateKind::Checked)
            .filter(|e| e.expires_ms.is_none_or(|at| at > now_ms))
            .is_some_and(|e| matches!(e.text.as_str(), "on" | "true"))
    }

    #[must_use]
    pub fn hooks(&self, on: &str) -> Vec<By<HookDecl>> {
        self.all(|c| c.hooks.clone())
            .into_iter()
            .filter(|h| h.item.on == on)
            .collect()
    }

    #[must_use]
    pub fn doc_kinds(&self) -> Vec<By<DocKindDecl>> {
        self.all(|c| c.doc_kinds.clone())
    }

    /// The document kind whose files `path` is, and who registered it.
    #[must_use]
    pub fn doc_kind_of(&self, path: &str) -> Option<By<DocKindDecl>> {
        self.doc_kinds().into_iter().find(|d| {
            OpenerDecl {
                files: d.item.files.clone(),
                ..OpenerDecl::default()
            }
            .claims_path(path)
        })
    }

    /// ⌘O's openers (not the claims), each answering queries with its
    /// `action`: what the palette asks.
    #[must_use]
    pub fn quick_open(&self) -> Vec<By<OpenerDecl>> {
        self.openers()
            .into_iter()
            .filter(|o| !o.item.claims())
            .collect()
    }

    /// Every opener (⌘O's and the claims).
    #[must_use]
    pub fn openers(&self) -> Vec<By<OpenerDecl>> {
        self.all(|c| c.openers.clone())
    }

    /// What opens the file at `path`, best first: the openers that claim
    /// its extension, then those that claim its document kind.
    #[must_use]
    pub fn open_path(&self, path: &str) -> Vec<By<OpenerDecl>> {
        let kind = self.doc_kind_of(path);
        let openers = self.openers();
        let by_file = openers.iter().filter(|o| o.item.claims_path(path));
        let by_kind = openers.iter().filter(|o| {
            !o.item.claims_path(path)
                && kind
                    .as_ref()
                    .is_some_and(|k| o.item.doc_kinds.contains(&k.item.id))
        });
        by_file.chain(by_kind).cloned().collect()
    }

    /// What opens a document of kind `kind`.
    #[must_use]
    pub fn open_doc_kind(&self, kind: &str) -> Vec<By<OpenerDecl>> {
        self.openers()
            .into_iter()
            .filter(|o| o.item.doc_kinds.iter().any(|k| k == kind))
            .collect()
    }

    /// What opens the link `url`: the openers with a pattern that matches
    /// it whole or in part, in the order they came.
    #[must_use]
    pub fn open_url(&self, url: &str) -> Vec<By<OpenerDecl>> {
        self.openers()
            .into_iter()
            .filter(|o| {
                o.item
                    .urls
                    .iter()
                    .any(|p| regex::Regex::new(p).is_ok_and(|r| r.is_match(url)))
            })
            .collect()
    }

    /// Every inline embed, in the order its owner came.
    #[must_use]
    pub fn embeds(&self) -> Vec<By<EmbedDecl>> {
        self.all(|c| c.embeds.clone())
    }

    /// The embed that draws `claim`: the first loaded that claims it.
    #[must_use]
    pub fn embed_for(&self, claim: &Claim) -> Option<By<EmbedDecl>> {
        self.embeds().into_iter().find(|e| e.item.claims(claim))
    }

    /// Who fills each slot (an extension's claim, the latest that came
    /// winning): slot → (owner, view).
    #[must_use]
    pub fn slots(&self) -> BTreeMap<String, (String, String)> {
        let mut out = BTreeMap::new();
        for by in self.all(|c| c.slots.clone()) {
            out.insert(by.item.slot, (by.owner, by.item.view));
        }
        out
    }

    /// Writes runtime state, at `now_ms` on the registry's clock: kept,
    /// or stale (dropped).
    ///
    /// # Errors
    ///
    /// It names no source or key, or its source is full.
    // The guard goes to `changed`, which tells the listeners after it.
    #[allow(clippy::significant_drop_tightening)]
    pub fn set_state(&self, write: StateWrite, now_ms: u64) -> Result<Applied, StateError> {
        if write.source.is_empty() || write.key.is_empty() {
            return Err(StateError::Unnamed);
        }
        let mut inner = self.lock();
        inner
            .state
            .retain(|_, e| e.expires_ms.is_none_or(|at| at > now_ms));
        let id = (write.source.clone(), write.key.clone());
        if let Some(kept) = inner.state.get(&id)
            && write.seq != 0
            && write.seq <= kept.seq
        {
            return Ok(Applied::Stale);
        }
        let held = inner
            .state
            .keys()
            .filter(|(s, _)| *s == write.source)
            .count();
        if !inner.state.contains_key(&id) && held >= MAX_KEYS {
            return Err(StateError::Full {
                source: write.source,
            });
        }
        let entry = StateEntry {
            source: write.source,
            key: write.key,
            kind: write.kind,
            text: normalize(&write.text),
            icon: write.icon,
            target: write.target,
            seq: write.seq,
            expires_ms: (write.ttl_ms > 0).then(|| now_ms.saturating_add(write.ttl_ms)),
        };
        inner.state.insert(id, entry);
        Self::changed(inner);
        Ok(Applied::Kept)
    }

    /// Clears `source`'s state: one key, or all of it. How many went.
    // What changed is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn clear_state(&self, source: &str, key: Option<&str>) -> usize {
        let mut inner = self.lock();
        let before = inner.state.len();
        inner
            .state
            .retain(|(s, k), _| !(s == source && key.is_none_or(|key| key == k)));
        let gone = before.saturating_sub(inner.state.len());
        if gone > 0 {
            Self::changed(inner);
        }
        gone
    }

    /// The runtime state at `now_ms`: what has not lapsed, by source and
    /// key.
    #[must_use]
    pub fn state(&self, now_ms: u64) -> Vec<StateEntry> {
        self.lock()
            .state
            .values()
            .filter(|e| e.expires_ms.is_none_or(|at| at > now_ms))
            .cloned()
            .collect()
    }

    /// When the next state lapses (ms), so a window can redraw then.
    #[must_use]
    pub fn next_expiry(&self) -> Option<u64> {
        self.lock()
            .state
            .values()
            .filter_map(|e| e.expires_ms)
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_open_opener_queries_and_opens_with_its_own_actions() {
        let views: Vec<String> = Vec::new();
        let ok = Contribution {
            openers: vec![OpenerDecl {
                kind: "theme".into(),
                title: "Themes".into(),
                action: "LAB_CONTRIB_FIND".into(),
                open: "LAB_CONTRIB_PICK".into(),
                ..OpenerDecl::default()
            }],
            ..Contribution::default()
        };
        assert_eq!(ok.check(&rules(&views, &[])), Ok(()));
        let reg = Contributions::new();
        reg.put("lab.contrib", ok.clone());
        assert_eq!(reg.quick_open().len(), 1);
        let mut foreign = ok;
        foreign.openers[0].open = "THEME_SET".into();
        let refused = foreign.check(&rules(&views, &[])).unwrap_err();
        assert!(refused.why.contains("THEME_SET"), "{refused}");
        assert_eq!(foreign.check(&rules(&views, &["THEME_".into()])), Ok(()));
    }

    fn rules<'a>(views: &'a [String], reach: &'a [String]) -> Rules<'a> {
        Rules {
            prefix: "LAB_CONTRIB_".to_owned(),
            reach,
            views,
            luau: false,
        }
    }

    fn sample() -> Contribution {
        Contribution {
            openers: vec![
                OpenerDecl {
                    id: "chart".into(),
                    title: "Keyflow chart".into(),
                    files: vec!["kf".into()],
                    view: Some("chart".into()),
                    ..OpenerDecl::default()
                },
                OpenerDecl {
                    id: "issue".into(),
                    title: "GitHub issue".into(),
                    urls: vec![r"^https://github\.com/[^/]+/[^/]+/issues/\d+$".into()],
                    action: "LAB_CONTRIB_ISSUE".into(),
                    ..OpenerDecl::default()
                },
            ],
            tabs: vec![TabDecl {
                id: "board".into(),
                title: "Board".into(),
                icon: "layout-dashboard".into(),
                view: "board".into(),
            }],
            status: vec![StatusDecl {
                id: "clock".into(),
                text: "Ready".into(),
                ..StatusDecl::default()
            }],
            context: vec![ContextDecl {
                target: "view:note".into(),
                action: "LAB_CONTRIB_PIN".into(),
                title: "Pin to the board".into(),
            }],
            hooks: vec![HookDecl {
                on: "startup".into(),
                action: Some("LAB_CONTRIB_HELLO".into()),
                ..HookDecl::default()
            }],
            slots: vec![SlotDecl {
                slot: "status".into(),
                view: "bar".into(),
            }],
            ..Contribution::default()
        }
    }

    #[test]
    fn a_contribution_keeps_to_its_views_actions_events_and_slots() {
        let views = ["chart".to_owned(), "board".to_owned(), "bar".to_owned()];
        let c = sample();
        c.check(&rules(&views, &[])).unwrap();

        let mut bad = c.clone();
        bad.tabs[0].view = "nope".into();
        assert!(
            bad.check(&rules(&views, &[]))
                .unwrap_err()
                .why
                .contains("no view")
        );
        let mut bad = c.clone();
        bad.hooks[0].on = "lunch".into();
        assert!(
            bad.check(&rules(&views, &[]))
                .unwrap_err()
                .why
                .contains("not an event")
        );
        let mut bad = c.clone();
        bad.slots[0].slot = "footer".into();
        assert!(
            bad.check(&rules(&views, &[]))
                .unwrap_err()
                .why
                .contains("not a slot")
        );
        let mut bad = c.clone();
        bad.context[0].action = "WORKSPACE_CLOSE".into();
        assert!(bad.check(&rules(&views, &[])).is_err());
        bad.check(&rules(&views, &["WORKSPACE_".to_owned()]))
            .unwrap();
        let mut bad = c.clone();
        bad.openers[1].urls = vec!["(".into()];
        assert!(
            bad.check(&rules(&views, &[]))
                .unwrap_err()
                .why
                .contains("pattern")
        );
        let mut bad = c;
        bad.tabs.push(bad.tabs[0].clone());
        assert!(bad.check(&rules(&views, &[])).is_err(), "a duplicate id");
    }

    #[test]
    fn openers_claim_files_kinds_and_links() {
        let reg = Contributions::new();
        let mut c = sample();
        c.doc_kinds.push(DocKindDecl {
            id: "keyflow.chart".into(),
            name: "Chart".into(),
            files: vec!["kfc".into()],
            ..DocKindDecl::default()
        });
        c.openers.push(OpenerDecl {
            id: "by-kind".into(),
            doc_kinds: vec!["keyflow.chart".into()],
            view: Some("chart".into()),
            ..OpenerDecl::default()
        });
        reg.put("lab.contrib", c);
        let found = |p: &str| -> Vec<String> {
            reg.open_path(p)
                .into_iter()
                .map(|o| o.item.key().to_owned())
                .collect()
        };
        assert_eq!(found("songs/Intro.KF"), ["chart"]);
        assert_eq!(found("/a/b.kfc"), ["by-kind"], "by its document kind");
        assert!(found("notes/kf").is_empty(), "a name, not an extension");
        assert!(found("x.rpp").is_empty());
        assert_eq!(
            reg.doc_kind_of("a.kfc").map(|d| d.item.id),
            Some("keyflow.chart".to_owned())
        );
        assert_eq!(reg.open_doc_kind("keyflow.chart").len(), 1);
        let url = |u: &str| reg.open_url(u).len();
        assert_eq!(url("https://github.com/fts/architect/issues/179"), 1);
        assert_eq!(url("https://github.com/fts/architect/pull/179"), 0);
    }

    #[test]
    fn a_put_is_a_diff_and_a_withdrawal_takes_everything() {
        let reg = Contributions::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&seen);
        let _sub = reg.subscribe(move |r| heard.lock().unwrap().push(r));
        let first = reg.put("lab.contrib", sample());
        assert!(first.added.contains(&"tab:board".to_owned()));
        let mut next = sample();
        next.tabs.clear();
        next.bar.push(BarDecl {
            action: "LAB_CONTRIB_HELLO".into(),
            title: "Hello".into(),
            icon: String::new(),
        });
        let diff = reg.put("lab.contrib", next);
        assert_eq!(diff.removed, ["tab:board"]);
        assert_eq!(diff.added, ["bar:LAB_CONTRIB_HELLO"]);
        assert!(diff.kept.contains(&"status:clock".to_owned()));

        let runtime = reg
            .put_runtime(
                "lab.contrib",
                Contribution {
                    tabs: vec![TabDecl {
                        id: "live".into(),
                        title: "Live".into(),
                        view: "board".into(),
                        ..TabDecl::default()
                    }],
                    ..Contribution::default()
                },
            )
            .unwrap();
        assert_eq!(runtime.added, ["tab:live"]);
        assert!(reg.put_runtime("nobody", Contribution::default()).is_none());
        assert_eq!(reg.slots()["status"], ("lab.contrib".into(), "bar".into()));

        reg.set_state(
            StateWrite {
                source: "ext:lab.contrib".into(),
                key: "clock".into(),
                text: "12:00".into(),
                ..StateWrite::default()
            },
            0,
        )
        .unwrap();
        let gone = reg.withdraw("lab.contrib");
        assert!(gone.removed.contains(&"tab:live".to_owned()));
        assert!(gone.removed.contains(&"slot:status".to_owned()));
        assert!(reg.tabs().is_empty() && reg.slots().is_empty());
        assert!(reg.state(0).is_empty(), "its state went with it");
        assert_eq!(seen.lock().unwrap().len(), 5);
    }

    #[test]
    fn runtime_state_is_owned_sequenced_and_time_limited() {
        let reg = Contributions::new();
        reg.put("lab.contrib", sample());
        let write = |seq: u64, text: &str, ttl_ms: u64| StateWrite {
            source: "ext:lab.contrib".into(),
            key: "clock".into(),
            text: text.into(),
            seq,
            ttl_ms,
            ..StateWrite::default()
        };
        assert_eq!(reg.set_state(write(2, "two", 0), 0), Ok(Applied::Kept));
        assert_eq!(reg.set_state(write(1, "one", 0), 0), Ok(Applied::Stale));
        assert_eq!(
            reg.status(0)[0].item.text,
            "two",
            "the declared item's text"
        );
        assert_eq!(
            reg.set_state(write(3, "three", 1000), 10),
            Ok(Applied::Kept)
        );
        assert_eq!(reg.next_expiry(), Some(1010));
        assert_eq!(reg.status(500)[0].item.text, "three");
        assert_eq!(
            reg.status(1010)[0].item.text,
            "Ready",
            "lapsed: the declared text"
        );

        let agent = StateWrite {
            source: "agent:claude".into(),
            key: "turn".into(),
            text: "  thinking\n about it  ".into(),
            ..StateWrite::default()
        };
        reg.set_state(agent, 0).unwrap();
        let status = reg.status(0);
        let theirs = status.iter().find(|s| s.owner == "agent:claude").unwrap();
        assert_eq!(theirs.item.text, "thinking about it");
        assert_eq!(reg.clear_state("agent:claude", None), 1);
        assert_eq!(normalize(&"x".repeat(200)).len(), MAX_TEXT);
        assert_eq!(
            reg.set_state(StateWrite::default(), 0),
            Err(StateError::Unnamed)
        );
        for i in 0..MAX_KEYS {
            reg.set_state(
                StateWrite {
                    source: "agent:busy".into(),
                    key: format!("k{i}"),
                    ..StateWrite::default()
                },
                0,
            )
            .unwrap();
        }
        assert!(matches!(
            reg.set_state(
                StateWrite {
                    source: "agent:busy".into(),
                    key: "one-more".into(),
                    ..StateWrite::default()
                },
                0
            ),
            Err(StateError::Full { .. })
        ));
    }

    #[test]
    fn a_toggle_is_on_while_its_last_checked_state_says_so() {
        let reg = Contributions::new();
        reg.put("lab.x", Contribution::default());
        assert!(!reg.checked("lab.x", "LAB_X_LOOP", 0));
        let write = |text: &str, ttl_ms: u64| StateWrite {
            source: "ext:lab.x".into(),
            key: "LAB_X_LOOP".into(),
            kind: StateKind::Checked,
            text: text.into(),
            ttl_ms,
            ..StateWrite::default()
        };
        reg.set_state(write("on", 0), 0).unwrap();
        assert!(reg.checked("lab.x", "LAB_X_LOOP", 1));
        reg.set_state(write("off", 0), 0).unwrap();
        assert!(!reg.checked("lab.x", "LAB_X_LOOP", 1));
        reg.set_state(write("true", 10), 0).unwrap();
        assert!(reg.checked("lab.x", "LAB_X_LOOP", 5));
        assert!(!reg.checked("lab.x", "LAB_X_LOOP", 20), "lapsed");
        // Another's state is not its.
        assert!(!reg.checked("lab.y", "LAB_X_LOOP", 5));
        // A status item of the same key is not a tick.
        reg.set_state(
            StateWrite {
                kind: StateKind::Status,
                ..write("on", 0)
            },
            0,
        )
        .unwrap();
        assert!(!reg.checked("lab.x", "LAB_X_LOOP", 1));
    }

    #[test]
    fn context_entries_find_their_targets() {
        let on = |target: &str| ContextDecl {
            target: target.into(),
            action: "A".into(),
            title: String::new(),
        };
        assert!(on("pane").on("view:note", "note-1"));
        assert!(on("view:note").on("view:note", "note-1"));
        assert!(!on("view:note").on("view:files", "files"));
        assert!(on("file").on("file", "a/b.kf"));
        assert!(on("file:kf").on("file", "a/b.kf"));
        assert!(!on("file:kf").on("file", "a/b.rpp"));
        assert!(!on("tab").on("view:note", "x"));
        assert!(known_target("doc:keyflow.chart"));
        assert!(!known_target("view:"));
    }

    fn embed(id: &str, view: Option<&str>, page: Option<&str>, fence: &[&str]) -> EmbedDecl {
        EmbedDecl {
            id: id.into(),
            view: view.map(Into::into),
            page: page.map(Into::into),
            claims: EmbedClaims {
                fence: fence.iter().map(|f| (*f).to_owned()).collect(),
                ..EmbedClaims::default()
            },
            ..EmbedDecl::default()
        }
    }

    #[test]
    fn an_embed_is_drawn_by_one_of_its_views_or_a_page_and_claims_something() {
        let views = vec!["chart".to_owned()];
        let check = |e: EmbedDecl| {
            Contribution {
                embeds: vec![e],
                ..Contribution::default()
            }
            .check(&rules(&views, &[]))
        };
        assert!(check(embed("kf", Some("chart"), None, &["kf"])).is_ok());
        assert!(check(embed("kf", None, Some("pages/kf.tsx"), &["kf"])).is_ok());
        let why = |e| check(e).unwrap_err().why;
        assert!(why(embed("kf", Some("nope"), None, &["kf"])).contains("no view"));
        assert!(why(embed("kf", None, Some("../x.tsx"), &["kf"])).contains("outside"));
        assert!(why(embed("kf", Some("chart"), Some("p.tsx"), &["kf"])).contains("one view"));
        assert!(why(embed("kf", Some("chart"), None, &[])).contains("claims nothing"));
        assert!(why(embed("kf", Some("chart"), None, &[" "])).contains("empty claim"));
        let mut tall = embed("kf", Some("chart"), None, &["kf"]);
        tall.max_height = Some(0);
        assert!(why(tall).contains("max_height"));
    }

    #[test]
    fn a_claim_resolves_to_the_first_loaded_embed_and_goes_with_it() {
        let reg = Contributions::new();
        let claim = Claim::fence("KF chorus").unwrap();
        assert_eq!(claim, Claim::Fence("KF".into()));
        assert_eq!(
            Claim::parse("note_type:song"),
            Some(Claim::NoteType("song".into()))
        );
        assert_eq!(Claim::parse("fence:"), None);
        assert_eq!(Claim::NoteType("song".into()).to_string(), "note_type:song");
        assert!(reg.embed_for(&claim).is_none());
        reg.put(
            "a",
            Contribution {
                embeds: vec![embed("kf", Some("chart"), None, &["kf"])],
                ..Contribution::default()
            },
        );
        reg.put(
            "b",
            Contribution {
                embeds: vec![embed("other", Some("x"), None, &["kf", "abc"])],
                ..Contribution::default()
            },
        );
        assert_eq!(reg.embed_for(&claim).unwrap().owner, "a");
        assert_eq!(
            reg.embed_for(&Claim::Fence("abc".into())).unwrap().owner,
            "b"
        );
        assert!(reg.embed_for(&Claim::NoteType("kf".into())).is_none());
        let gone = reg.withdraw("a");
        assert_eq!(gone.removed, vec!["embed:kf".to_owned()]);
        assert_eq!(reg.embed_for(&claim).unwrap().owner, "b");
        reg.withdraw("b");
        assert!(reg.embed_for(&claim).is_none());
    }
}
