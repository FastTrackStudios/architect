//! An extension's manifest, `extension.toml`: who it is, what kind it is,
//! what it may reach, and what it adds — read before anything of it runs.
//!
//! ```toml
//! id = "fts.metronome"
//! name = "Metronome"
//! version = "0.2.0"
//! kind = "luau"
//! api = "0.1"
//! description = "A click, and the actions to count in."
//!
//! [capabilities]
//! actions = ["TRANSPORT_"]   # namespaces it may run, past its own
//! effect = "read_only"       # the most it may change
//! reads = ["selection"]      # app state it may read (org, selection, documents)
//!
//! [[actions]]
//! id = "FTS_METRONOME_COUNT_IN"
//! name = "Count in"
//! script = "count_in.luau"
//!
//! [[sections]]
//! id = "metronome"
//! title = "Metronome"
//! icon = "timer"
//!
//! [[sections.settings]]
//! key = "fts.metronome.accent"
//! title = "Accent the downbeat"
//! control = "switch"
//! default = true
//!
//! [[themes]]
//! path = "themes/metronome-night.json"
//!
//! [tokens]                     # its own tokens' defaults, in its namespace
//! "metronome.beat" = "$brand"
//! "metronome.off-beat" = "mix($ink-3, $brand, 0.25)"
//! ```
//!
//! What it adds to the interface — openers for files, document kinds and
//! links, document kinds, tabs, status items, menu and context-menu
//! entries, action-bar buttons, toolbars, hooks and slots — are the tables of a
//! [`Contribution`] (`crate::contrib`), read into
//! [`Manifest::contributes`].

use architect_settings::{Block, Choice, Control, Section, Setting, Value};
use semver::Version;
use serde::{Deserialize, Serialize};

pub use crate::contrib::{
    BarDecl, ContextDecl, Contribution, DocKindDecl, HookDecl, MenuDecl, OpenerDecl, Refused,
    Rules, SlotDecl, StatusDecl, TabDecl, ToolItem, ToolbarDecl, ToolbarPlace,
};

/// The extension API this build speaks: a manifest naming a newer one
/// (`api = "0.2"` here) is refused, an older one in the same `0.x` taken.
pub const API: &str = "0.1";

/// The manifest's file name in a package.
pub const MANIFEST: &str = "extension.toml";

/// How an extension runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A manifest and files, no code: settings, presets, keys, views.
    /// Everywhere, iOS included.
    Data,
    /// Luau scripts over the Action List (`architect-scripting`'s runtime).
    Luau,
    /// A WebAssembly component against the `architect:extension` world.
    Wasm,
    /// A program speaking vox, as a remote client does. Desk only.
    Process,
}

impl Kind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Luau => "luau",
            Self::Wasm => "wasm",
            Self::Process => "process",
        }
    }

    /// Whether it carries code: what a project can need and a phone may
    /// not run (a process).
    #[must_use]
    pub const fn runs_code(self) -> bool {
        !matches!(self, Self::Data)
    }
}

/// What an action does to the app, as a manifest says it: the same three
/// steps as the Action List's `Effect`.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    #[default]
    ReadOnly,
    Mutating,
    Destructive,
}

impl Reach {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Mutating => "mutating",
            Self::Destructive => "destructive",
        }
    }
}

/// What it asks to reach, granted when it is installed.
///
/// The person sees it then, and again on an update that asks for more
/// (`crate::consent`). What it may do through the host service
/// (`crate::hostapi`) is checked against what was granted of this.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Capabilities {
    /// The action id prefixes it may run, past its own namespace
    /// (`TRANSPORT_`); `*` for every action.
    pub actions: Vec<String>,
    /// The most an action it runs may change.
    pub effect: Reach,
    /// The network hosts it may reach (WASM and processes).
    pub hosts: Vec<String>,
    /// The folders it may read and write, past its own.
    pub folders: Vec<String>,
    /// What of the app's state it may read through the host service:
    /// `org` (the current organization), `selection` (what is selected
    /// and focused), `documents` (open and read the app's documents).
    pub reads: Vec<String>,
    /// What of the app's state it may change through the host service
    /// (`crate::capabilities::WRITES`): `documents` (create, edit, rename,
    /// move and delete the app's documents, as itself).
    pub writes: Vec<String>,
    /// The app's interface it may use through the host service
    /// (`crate::capabilities::UI`): `notify` (notifications), `clipboard`
    /// (read and write it), `pickers` (file and folder pickers, save
    /// dialogs), `prompts` (ask for text, a yes or no, one of a list),
    /// `menus` (a context menu of its own at a point), `badges` (icons
    /// and badges on file-tree rows and tabs).
    pub ui: Vec<String>,
    /// The app-defined events it may hear (`app:<name>` hooks;
    /// `crate::events`): names, or `<prefix>.*`.
    pub events: Vec<String>,
    /// The MIDI ports and OSC endpoints it may open (`crate::ports`):
    /// `midi:<port name>` (a trailing `*`: any it begins; `midi:*` any),
    /// `osc:<host>:<port>` (`osc:<host>:*`, `osc:*`) where it sends, and
    /// `osc:listen:<port>` where it hears.
    pub ports: Vec<String>,
    /// What of its languages (`[[languages]]`) the host may do past
    /// highlighting them (`crate::capabilities::LANGUAGES`): `servers`
    /// (run a language's server, a program on this computer, while a
    /// file of the language is open).
    pub languages: Vec<String>,
}

/// One action it adds.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ActionDecl {
    /// In the extension's namespace ([`Manifest::action_prefix`]).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub group: String,
    /// A key that runs it (`⇧⌘M`), for display and the keymap.
    #[serde(default)]
    pub shortcut: String,
    #[serde(default)]
    pub effect: Reach,
    /// What it takes, as JSON Schema text.
    #[serde(default)]
    pub params: Option<String>,
    /// A Luau file in the package that runs it (a `luau` extension).
    #[serde(default)]
    pub script: Option<String>,
    /// Another action it runs with fixed arguments: a preset, for any
    /// kind (a data extension's only code-free action).
    #[serde(default)]
    pub preset: Option<Preset>,
    /// A toggle: it says whether it is on, by writing runtime state of
    /// kind `checked` keyed by its id (`ext.state{kind = "checked", key
    /// = "…", text = "on"}`, a guest's `set_state`), off until it does.
    /// Its tick in a menu and ⌘K, and its toolbar buttons, follow it.
    #[serde(default)]
    pub toggle: bool,
}

/// An action run with fixed arguments.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Preset {
    pub action: String,
    /// Its arguments, as JSON text.
    #[serde(default)]
    pub args: Option<String>,
}

/// Where a view lives when nothing says otherwise: the main area, or a
/// dock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Home {
    #[default]
    Main,
    Left,
    Right,
    Bottom,
}

/// How a view opens (herdr's pane placements, on the workspace's own).
///
/// A tab, a split beside the active group, over the window (`overlay`: a
/// layer, `popup`: a modal one, sized), alone in its window (`zoomed`), or
/// floating.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewPlacement {
    #[default]
    Tab,
    Split,
    Overlay,
    Popup,
    Zoomed,
    Float,
}

/// What its open panes do when the extension unloads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OnUnload {
    /// Stay, saying it is gone, until it comes back or they are closed.
    #[default]
    Placeholder,
    /// Close (kept to reopen).
    Close,
}

/// A view it adds to the workspace: listed by the host, mounted by the
/// app.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ViewDecl {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: String,
    /// The view kind that draws it (`kind = "frame.surface"`;
    /// `docs/content/architecture/embedded-views.md`): `dioxus` when it
    /// says none — a process extension's own Dioxus view, drawn remotely.
    /// A kind the app has no provider for refuses the load
    /// (`host::LoadError::ViewKind`).
    #[serde(default = "dioxus")]
    pub kind: String,
    /// A frame document in the package that draws it, for a data view.
    #[serde(default)]
    pub document: Option<String>,
    /// A page file in the package that draws it (`page = "pages/board.tsx"`:
    /// TSX/JS on `QuickJS`, `.luau` elements or `.a2ui.json`;
    /// `docs/content/architecture/pages.md`), re-rendered live when it is
    /// saved. An app that runs pages (`architect-pages`) draws it; one
    /// that does not draws the extension's own view.
    #[serde(default)]
    pub page: Option<String>,
    /// Where it lives: the main area or a dock (`left`, `right`,
    /// `bottom`).
    #[serde(default)]
    pub home: Home,
    /// How it opens: `tab`, `split`, `overlay`, `popup`, `zoomed`,
    /// `float`.
    #[serde(default)]
    pub placement: ViewPlacement,
    /// A popup's, overlay's or float's size, points.
    #[serde(default)]
    pub width: Option<f32>,
    #[serde(default)]
    pub height: Option<f32>,
    /// What its panes do when it unloads.
    #[serde(default)]
    pub on_unload: OnUnload,
}

impl ViewDecl {
    /// Its kind in the app's workspace: `<extension id>/<view id>`, so two
    /// extensions' views never meet, and a pane says whose it is.
    #[must_use]
    pub fn workspace_kind(&self, extension: &str) -> String {
        format!("{extension}/{}", self.id)
    }
}

/// A layer it draws over a kind of view (`session.arrange`).
///
/// A live Luau script with `Canvas` and `ImGui` (`architect-canvas`),
/// started while the extension is loaded, drawing in the view's
/// coordinates on one of its planes. The app's canvas host runs it; the
/// script may open tool windows too.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct OverlayDecl {
    /// Unique among its contributions.
    pub id: String,
    /// The kind of view it draws over.
    pub view: String,
    /// `under` the view's items, `over` them (the default), or `top`.
    #[serde(default = "over")]
    pub plane: String,
    /// The script that draws it, in the package.
    pub script: String,
}

/// A worker: heavy work in an instance of its own, off every view's
/// thread, spoken to by messages only.
///
/// A view spawns it by id and reads its progress; it is cancelled when
/// the view or the extension goes
/// (`docs/content/architecture/extensions.md`, "Responsive by design:
/// views and workers").
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct WorkerDecl {
    /// What a view spawns it by (`worker.spawn('primes')`). Unique among
    /// its workers.
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// A Luau script in the package (`luau` and `process` packages: a
    /// process's Luau windows spawn it too): run in a Luau state of its
    /// own on the host's pool (`architect_scripting::worker`).
    #[serde(default)]
    pub script: Option<String>,
    /// What its program calls it (`process`, `wasm`): the name a process
    /// spawns it under (`guest::spawn_work`); for a component, the entry
    /// its `work.run` is given (`Extension::worker`), or a separate
    /// component in the package (`*.wasm`) run as the worker
    /// (`architect_extensions::wasi`'s workers).
    #[serde(default)]
    pub entry: Option<String>,
}

fn dioxus() -> String {
    architect_workspace::DIOXUS.to_owned()
}

fn over() -> String {
    "over".to_owned()
}

/// A theme it contributes: a theme file (`architect_theme`'s format) in
/// the package, its id the file's name.
///
/// A host with the app's theme store (`host::Host::with_themes`) puts it
/// there while the extension is loaded; a file that does not read is a
/// theme problem, not a failed load.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeDecl {
    /// The file, relative to the package (`themes/night.json`).
    pub path: String,
}

/// Where in a project an extension keeps its own data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataScope {
    /// One container for the project.
    #[default]
    Project,
    /// One per object it annotates (a track, an item, a section).
    Object,
}

/// Document data it keeps, namespaced by its id.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DataDecl {
    pub name: String,
    #[serde(default)]
    pub scope: DataScope,
}

/// A settings section it adds, there while it is loaded: an item of the
/// settings page's point (`settings.sections`), in the shape that point's
/// items have.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SectionDecl {
    /// Unique on the settings page.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: String,
    /// The heading it sits under (`Extensions` when none is given; `""`
    /// for the first, untitled group, as a built-in's).
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingDecl>,
    /// One of its `[[views]]` that draws the section in place of
    /// settings: a page of its own on the settings page (the extensions
    /// manager's). The app's own sections name a page of the app's
    /// (`appearance`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
}

/// One setting, as an author writes it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SettingDecl {
    /// `<extension id>.<name>` (`fts.metronome.accent`).
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The titled block it sits in; untitled when none.
    #[serde(default)]
    pub block: Option<String>,
    /// `switch`, `select`, `segmented`, `cards`, `slider`, `stepper`,
    /// `text` or `shortcut`.
    pub control: String,
    pub default: toml::Value,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub choices: Vec<ChoiceDecl>,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Where its value lives: `device` (unset), `synced` (the person's,
    /// on every device) or `project`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// One choice of a select, segmented control or cards.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChoiceDecl {
    pub value: String,
    pub label: String,
    #[serde(default)]
    pub detail: String,
}

/// What a WASM or process extension runs: its entry point.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// The component (`extension.wasm`) or the program (`bin/tool`), in
    /// the package.
    pub path: String,
    /// A process's arguments.
    pub args: Vec<String>,
}

/// A WASM extension's component (`[wasm]`): what the `wasi` runner
/// instantiates against the `architect:extension` world.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct WasmDecl {
    /// The component (`extension.wasm`), in the package.
    pub component: String,
}

/// Where an extension's crate is (`[source]`): what `EXT_CLONE` of a
/// linked built-in writes to the development folder and builds as its
/// process (`crate::source`).
///
/// ```toml
/// [source]
/// git = "https://github.com/FastTrackStudios/architect"
/// rev = "main"
/// path = "apps/sandbox/board"
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SourceDecl {
    /// The repository it is in. An app that has that repository's sources
    /// (`Extensions::with_vendored`) copies from them; any other clones it.
    pub git: String,
    /// The commit, tag or branch (`main` when empty).
    pub rev: String,
    /// The crate's folder in the repository (`apps/sandbox/board`).
    pub path: String,
    /// The binary that serves it as a process (`Guest::extension(..).serve()`):
    /// its `[entry] path`'s file name when empty.
    pub bin: String,
}

/// `[[favourites]]` as written, read one declaration at a time: those that
/// read and keep their rules, and why each other one was left out (`#<n>`
/// when it has no readable `note_type`).
fn favourites_of(
    raw: Option<toml::Value>,
) -> (Vec<architect_favourites::TypeFavourites>, Vec<String>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    let list = match raw {
        None => return (kept, dropped),
        Some(toml::Value::Array(list)) => list,
        Some(_) => {
            dropped.push("favourites: not a list of tables ([[favourites]])".to_owned());
            return (kept, dropped);
        }
    };
    for (n, value) in list.into_iter().enumerate() {
        let name = value
            .get("note_type")
            .and_then(toml::Value::as_str)
            .map_or_else(|| format!("#{}", n.saturating_add(1)), str::to_owned);
        match value.try_into::<architect_favourites::TypeFavourites>() {
            Ok(decl) => match decl.check() {
                Ok(()) => kept.push(decl),
                Err(why) => dropped.push(format!("{name}: {why}")),
            },
            Err(why) => dropped.push(format!("{name}: {why}")),
        }
    }
    (kept, dropped)
}

/// `extension.toml`, read.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Manifest {
    /// Reverse-DNS-ish, lower case (`fts.metronome`): unique in every
    /// registry, the owner of everything it adds.
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "version")]
    pub version: Version,
    pub kind: Kind,
    /// The extension API it was written against ([`API`]).
    #[serde(default = "api")]
    pub api: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub entry: Entry,
    /// A `wasm` extension's component (`[wasm] component`; `[entry] path`
    /// still names one).
    #[serde(default)]
    pub wasm: WasmDecl,

    /// Where its crate is, for a development copy built from source
    /// (`[source]`).
    #[serde(default)]
    pub source: Option<SourceDecl>,
    #[serde(default)]
    pub actions: Vec<ActionDecl>,
    #[serde(default)]
    pub sections: Vec<SectionDecl>,
    #[serde(default)]
    pub views: Vec<ViewDecl>,
    #[serde(default)]
    pub overlays: Vec<OverlayDecl>,
    /// Its `[[workers]]`: its heavy work, off its views' threads.
    #[serde(default)]
    pub workers: Vec<WorkerDecl>,
    #[serde(default)]
    pub data: Vec<DataDecl>,
    #[serde(default)]
    pub themes: Vec<ThemeDecl>,
    /// Its own theme tokens' defaults, by name: literals or expressions
    /// over the app's tokens (`"clicker.face" = "mix($card, $brand,
    /// 0.12)"`). Each name is in its namespace
    /// ([`Self::token_namespace`]). Loaded, they are the theme store's
    /// extension layer, so a theme, the app or the person still says the
    /// last word.
    #[serde(default)]
    pub tokens: std::collections::BTreeMap<String, String>,
    /// The windows it owns (`[[windows]]`, `crate::interop`).
    #[serde(default)]
    pub windows: Vec<crate::interop::WindowDecl>,
    /// The contribution points it declares (`[[points]]`).
    #[serde(default)]
    pub points: Vec<crate::interop::PointDecl>,
    /// Its items for others' points (`[[contribute."<point>"]]`), by point.
    #[serde(default)]
    pub contribute: std::collections::BTreeMap<String, Vec<toml::Value>>,
    /// The services it provides, each at a version (`[provides]`).
    #[serde(default)]
    pub provides: std::collections::BTreeMap<String, String>,
    /// The services it needs, each a semver requirement (`[requires]`).
    #[serde(default)]
    pub requires: std::collections::BTreeMap<String, String>,
    /// The services it calls when one is loaded, each a semver requirement
    /// (`[uses]`): an optional `[requires]`. None loaded is no problem (it
    /// loads, and its calls answer that no provider is there), so an
    /// extension that integrates with another still runs alone.
    #[serde(default)]
    pub uses: std::collections::BTreeMap<String, String>,
    /// App action ids (`APP_…`) it owns, when the app allows it
    /// (`Host::allow_reserved`): the settings sheet's `APP_SETTINGS`.
    #[serde(default)]
    pub reserved: Vec<String>,
    /// What it adds to the interface: its `[[openers]]`, `[[doc_kinds]]`,
    /// `[[tabs]]`, `[[status]]`, `[[menus]]`, `[[context]]`, `[[bar]]`,
    /// `[[toolbars]]`, `[[hooks]]`, `[[slots]]`, `[[embeds]]` and
    /// `[[base_views]]`.
    #[serde(flatten)]
    pub contributes: Contribution,
    /// `[background]`: started on its first event, stopped when idle
    /// (`crate::events::BackgroundDecl`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<crate::events::BackgroundDecl>,
    /// `[[secrets]]`: what the person enters once and the host keeps (an
    /// API key), put into its requests by reference (`crate::net`).
    #[serde(default)]
    pub secrets: Vec<SecretDecl>,
    /// `[[share_layers]]` (#528): which layer of a share link each file a
    /// note of its types attaches is in
    /// ([`crate::share_layers::ShareLayersDecl`]).
    #[serde(default)]
    pub share_layers: Vec<crate::share_layers::ShareLayersDecl>,
    /// `[[favourites]]`: the tabs and the primary action a shell shows
    /// while a note of its types is open (a share link's phone shell,
    /// the app's bottom bar): [`architect_favourites::TypeFavourites`].
    /// Read from [`Self::favourites_raw`] one declaration at a time by
    /// [`Self::parse`].
    #[serde(skip)]
    pub favourites: Vec<architect_favourites::TypeFavourites>,
    /// `[[favourites]]` as written: any TOML, so a declaration of the wrong
    /// shape (a misspelt key, an unknown kind of target) cannot fail the
    /// whole manifest; [`Self::parse`] reads each and drops the rest.
    #[serde(default, rename = "favourites")]
    favourites_raw: Option<toml::Value>,
    /// The `[[favourites]]` declarations [`Self::parse`] left out, each
    /// with why: data a shell draws must never make the manifest unreadable
    /// (an installed extension's unreadable manifest holds back a node's
    /// shared files), so a bad one is dropped and reported (`EXT_CHECK`).
    #[serde(skip)]
    pub favourites_dropped: Vec<String>,
}

/// One of an extension's `[[secrets]]`: an API key or a token the person
/// enters once.
///
/// The host keeps it (`crate::net::Secrets`), never in the extension's
/// folder, and never hands it to the extension. A request names it in a
/// header's value (`"Bearer {{secret:api_key}}"`); the host puts it in on
/// a request to one of its `hosts`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SecretDecl {
    /// What a request names it by (`a-z 0-9 _ - .`).
    pub id: String,
    /// What the person is asked for (`OpenAI API key`).
    pub title: String,
    /// A line under it: where to find one.
    pub description: Option<String>,
    /// The hosts it is sent to, each within `capabilities.hosts`; none:
    /// every one of those.
    pub hosts: Vec<String>,
}

fn api() -> String {
    API.to_owned()
}

fn version<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Version, D::Error> {
    let text = String::deserialize(d)?;
    Version::parse(&text).map_err(serde::de::Error::custom)
}

/// Why a manifest was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// It is not TOML, or not a manifest.
    Parse(String),
    /// Its id is empty, or has characters other than `a-z 0-9 . - _`.
    BadId(String),
    /// It needs a newer extension API than this build's.
    Api { wants: String, have: &'static str },
    /// An action outside its namespace (`expected` is the prefix).
    ActionNamespace { action: String, expected: String },
    /// A setting key outside its namespace (`<id>.`).
    SettingNamespace(String),
    /// An action with no way to run: no script, no preset, and a kind
    /// that has no code of its own.
    NoRun(String),
    /// A script in a kind that does not run Luau.
    ScriptKind(String),
    /// A file outside its package (`..`, an absolute path).
    BadPath(String),
    /// A setting that cannot be drawn: an unknown control, or a default
    /// of the wrong type.
    BadSetting { key: String, why: String },
    /// Two of its contributions share an id.
    Duplicate(String),
    /// A worker that names no script and no entry, both, or one its kind
    /// does not run.
    BadWorker { worker: String, why: String },
    /// An overlay on a plane that is not `under`, `over` or `top`.
    BadPlane { overlay: String, plane: String },
    /// A theme token outside its namespace, not a token name, or whose
    /// value does not read.
    BadToken { token: String, why: String },
    /// One of its interface contributions breaks a rule (an unknown view,
    /// event, target or slot; an action outside its reach; a pattern that
    /// does not compile).
    Contribution(Refused),
    /// A capability it asks for is not one there is.
    BadCapability(String),
    /// A window, point, contribution to a point, service or reserved id
    /// breaks a rule (`crate::interop::check`): the field, and why.
    Interop { field: String, why: String },
    /// A `[[share_layers]]` rule with no note type or a pattern that is
    /// not a relative path.
    BadShareLayers(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{MANIFEST} is not a manifest: {e}"),
            Self::BadId(id) => write!(f, "“{id}” is not an extension id (a-z, 0-9, . - _)"),
            Self::Api { wants, have } => {
                write!(f, "it needs extension API {wants}; this app has {have}")
            }
            Self::ActionNamespace { action, expected } => {
                write!(f, "action {action} is not in its namespace ({expected}…)")
            }
            Self::SettingNamespace(key) => write!(f, "setting {key} is not in its namespace"),
            Self::NoRun(id) => write!(f, "action {id} has no script and no preset"),
            Self::ScriptKind(id) => write!(f, "{id} has a script, but this is not Luau"),
            Self::BadPath(p) => write!(f, "{p} is outside the package"),
            Self::BadSetting { key, why } => write!(f, "setting {key}: {why}"),
            Self::Duplicate(id) => write!(f, "{id} is declared twice"),
            Self::BadToken { token, why } => write!(f, "theme token {token}: {why}"),
            Self::BadWorker { worker, why } => write!(f, "worker {worker}: {why}"),
            Self::Contribution(r) => r.fmt(f),
            Self::BadCapability(why) => write!(f, "capabilities: {why}"),
            Self::Interop { field, why } => write!(f, "{field}: {why}"),
            Self::BadShareLayers(why) => write!(f, "share_layers: {why}"),
            Self::BadPlane { overlay, plane } => {
                write!(
                    f,
                    "overlay {overlay}: {plane:?} is not a plane (under, over, top)"
                )
            }
        }
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    /// Each worker: a unique id, and one way to run that its kind has.
    fn check_workers(&self) -> Result<(), ManifestError> {
        let mut seen = std::collections::BTreeSet::new();
        for w in &self.workers {
            let bad = |why: &str| ManifestError::BadWorker {
                worker: w.id.clone(),
                why: why.to_owned(),
            };
            if w.id.is_empty() {
                return Err(bad("it has no id"));
            }
            if !seen.insert(w.id.as_str()) {
                return Err(ManifestError::Duplicate(w.id.clone()));
            }
            match (&w.script, &w.entry) {
                (Some(_), Some(_)) => return Err(bad("it names a script and an entry")),
                (None, None) => return Err(bad("it names no script and no entry")),
                (Some(path), None) => {
                    if !matches!(self.kind, Kind::Luau | Kind::Process) {
                        return Err(bad("a script worker needs a luau or process package"));
                    }
                    inside(path)?;
                }
                (None, Some(entry)) => {
                    if !matches!(self.kind, Kind::Process | Kind::Wasm) {
                        return Err(bad("an entry worker needs a process or wasm package"));
                    }
                    // A separate component (`wasm`): a file in the package.
                    if std::path::Path::new(entry)
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("wasm"))
                    {
                        inside(entry)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// A `wasm` extension's component, in its package: `[wasm] component`,
    /// else `[entry] path`; `None` when it names neither.
    #[must_use]
    pub fn component(&self) -> Option<&str> {
        [&self.wasm.component, &self.entry.path]
            .into_iter()
            .find(|p| !p.is_empty())
            .map(String::as_str)
    }

    /// Reads and checks `extension.toml`'s text.
    ///
    /// # Errors
    ///
    /// It is not a manifest, or breaks a rule [`Self::check`] keeps.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let mut manifest: Self =
            toml::from_str(text).map_err(|e| ManifestError::Parse(e.to_string()))?;
        // A declaration of the wrong shape, or that breaks a rule, is
        // dropped and said, never fatal.
        let (favourites, dropped) = favourites_of(manifest.favourites_raw.take());
        manifest.favourites = favourites;
        manifest.favourites_dropped = dropped;
        manifest.check()?;
        Ok(manifest)
    }

    /// The prefix every action it adds starts with: its id upper-cased,
    /// each run of other characters one `_` (`fts.metronome` →
    /// `FTS_METRONOME_`).
    #[must_use]
    pub fn action_prefix(&self) -> String {
        action_prefix(&self.id)
    }

    /// Checks what can be checked without its files: its id, its API,
    /// its namespaces, that each action can run, that each setting can be
    /// drawn.
    ///
    /// # Errors
    ///
    /// The first rule it breaks.
    pub fn check(&self) -> Result<(), ManifestError> {
        if self.id.is_empty()
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ".-_".contains(c))
        {
            return Err(ManifestError::BadId(self.id.clone()));
        }
        if !api_ok(&self.api) {
            return Err(ManifestError::Api {
                wants: self.api.clone(),
                have: API,
            });
        }
        if let Some(bad) = self
            .capabilities
            .reads
            .iter()
            .find(|r| !crate::capabilities::READS.contains(&r.as_str()))
        {
            return Err(ManifestError::BadCapability(format!(
                "reads {bad:?}: not one of {}",
                crate::capabilities::READS.join(", ")
            )));
        }
        if let Some(bad) = self
            .capabilities
            .writes
            .iter()
            .find(|w| !crate::capabilities::WRITES.contains(&w.as_str()))
        {
            return Err(ManifestError::BadCapability(format!(
                "writes {bad:?}: not one of {}",
                crate::capabilities::WRITES.join(", ")
            )));
        }
        if let Some(bad) = self
            .capabilities
            .ui
            .iter()
            .find(|u| !crate::capabilities::UI.contains(&u.as_str()))
        {
            return Err(ManifestError::BadCapability(format!(
                "ui {bad:?}: not one of {}",
                crate::capabilities::UI.join(", ")
            )));
        }
        if let Some(bad) = self
            .capabilities
            .languages
            .iter()
            .find(|l| !crate::capabilities::LANGUAGES.contains(&l.as_str()))
        {
            return Err(ManifestError::BadCapability(format!(
                "languages {bad:?}: not one of {}",
                crate::capabilities::LANGUAGES.join(", ")
            )));
        }
        if let Some(bad) = self.capabilities.events.iter().find(|e| {
            let name = e.strip_suffix(".*").unwrap_or(e);
            *e != "*" && !crate::events::valid_name(name)
        }) {
            return Err(ManifestError::BadCapability(format!(
                "events {bad:?}: an app event's name, <prefix>.* or *"
            )));
        }
        for entry in &self.capabilities.ports {
            crate::capabilities::check_port_entry(entry)
                .map_err(|why| ManifestError::BadCapability(format!("ports {entry:?}: {why}")))?;
        }
        for rule in &self.share_layers {
            rule.check().map_err(ManifestError::BadShareLayers)?;
        }
        let mut secrets = std::collections::BTreeSet::new();
        for secret in &self.secrets {
            let bad = |why: String| {
                Err(ManifestError::BadCapability(format!(
                    "secret {:?}: {why}",
                    secret.id
                )))
            };
            if secret.id.is_empty()
                || !secret
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c))
            {
                return bad("an id of a-z, 0-9, . _ -".to_owned());
            }
            if !secrets.insert(secret.id.as_str()) {
                return Err(ManifestError::Duplicate(format!("secret {}", secret.id)));
            }
            if self.capabilities.hosts.is_empty() {
                return bad("it is sent to no host: capabilities.hosts names none".to_owned());
            }
            if let Some(h) = secret
                .hosts
                .iter()
                .find(|h| !crate::capabilities::reaches_host(&self.capabilities.hosts, h, None))
            {
                return bad(format!("{h} is not one of capabilities.hosts"));
            }
        }
        if let Some(bg) = &self.background {
            if self.kind == Kind::Data {
                return Err(ManifestError::BadCapability(
                    "background: a data extension has nothing to start".to_owned(),
                ));
            }
            bg.idle()
                .map_err(|why| ManifestError::BadCapability(format!("background: {why}")))?;
        }
        let prefix = self.action_prefix();
        let mut seen = std::collections::BTreeSet::new();
        for a in &self.actions {
            if !a.id.starts_with(&prefix) && !self.reserved.contains(&a.id) {
                return Err(ManifestError::ActionNamespace {
                    action: a.id.clone(),
                    expected: prefix,
                });
            }
            if !seen.insert(a.id.as_str()) {
                return Err(ManifestError::Duplicate(a.id.clone()));
            }
            match (&a.script, &a.preset) {
                (Some(_), _) if self.kind != Kind::Luau => {
                    return Err(ManifestError::ScriptKind(a.id.clone()));
                }
                (Some(path), _) => inside(path)?,
                (None, None)
                    if matches!(self.kind, Kind::Data | Kind::Luau)
                        && self.window_opened_by(&a.id).is_none() =>
                {
                    return Err(ManifestError::NoRun(a.id.clone()));
                }
                _ => {}
            }
        }
        if !self.entry.path.is_empty() {
            inside(&self.entry.path)?;
        }
        if !self.wasm.component.is_empty() {
            inside(&self.wasm.component)?;
        }
        for view in &self.views {
            if let Some(doc) = &view.document {
                inside(doc)?;
            }
            if let Some(page) = &view.page {
                inside(page)?;
            }
        }
        for overlay in &self.overlays {
            if self.kind != Kind::Luau {
                return Err(ManifestError::ScriptKind(overlay.id.clone()));
            }
            if !seen.insert(overlay.id.as_str()) {
                return Err(ManifestError::Duplicate(overlay.id.clone()));
            }
            if !matches!(overlay.plane.as_str(), "under" | "over" | "top") {
                return Err(ManifestError::BadPlane {
                    overlay: overlay.id.clone(),
                    plane: overlay.plane.clone(),
                });
            }
            inside(&overlay.script)?;
        }
        self.check_workers()?;
        if let Some(source) = &self.source {
            inside(&source.path)?;
        }
        for theme in &self.themes {
            inside(&theme.path)?;
            if !std::path::Path::new(&theme.path)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            {
                return Err(ManifestError::BadPath(theme.path.clone()));
            }
        }
        let namespace = format!("{}.", self.token_namespace());
        for (token, value) in &self.tokens {
            let bad = |why: String| ManifestError::BadToken {
                token: token.clone(),
                why,
            };
            if !architect_theme::valid_token(token) {
                return Err(bad(architect_theme::TOKEN_NAME_RULE.to_owned()));
            }
            if !token.starts_with(&namespace) {
                return Err(bad(format!("not in its namespace ({namespace}…)")));
            }
            if value.trim().is_empty() {
                return Err(bad("empty".to_owned()));
            }
            architect_theme::expr::parse(value).map_err(bad)?;
        }
        self.contributes
            .check(&self.rules(&self.view_ids()))
            .map_err(ManifestError::Contribution)?;
        let actions: Vec<String> = self.actions.iter().map(|a| a.id.clone()).collect();
        crate::interop::check(&crate::interop::Decls {
            id: &self.id,
            actions: &actions,
            views: &self.view_ids(),
            windows: &self.windows,
            points: &self.points,
            contribute: &self.contribute,
            provides: &self.provides,
            requires: &self.requires,
            uses: &self.uses,
            reserved: &self.reserved,
        })
        .map_err(|b| ManifestError::Interop {
            field: b.field,
            why: b.why,
        })?;
        let keys = format!("{}.", self.id);
        for section in &self.sections {
            if !seen.insert(section.id.as_str()) {
                return Err(ManifestError::Duplicate(section.id.clone()));
            }
            if let Some(view) = &section.view
                && !self.views.iter().any(|v| v.id == *view)
            {
                return Err(ManifestError::Contribution(Refused {
                    what: format!("section {}", section.id),
                    why: format!("it has no view {view:?} ([[views]])"),
                }));
            }
            for s in &section.settings {
                if !s.key.starts_with(&keys) {
                    return Err(ManifestError::SettingNamespace(s.key.clone()));
                }
                setting(s)?;
            }
        }
        Ok(())
    }

    /// The window action `action` opens, when it is one's `opens`.
    #[must_use]
    pub fn window_opened_by(&self, action: &str) -> Option<&crate::interop::WindowDecl> {
        self.windows.iter().find(|w| w.opens == action)
    }

    /// Whether the host runs `action` itself, not its code: a preset, or
    /// the action that opens one of its windows.
    #[must_use]
    pub fn host_runs(&self, action: &ActionDecl) -> bool {
        action.preset.is_some() || self.window_opened_by(&action.id).is_some()
    }

    /// Its views' ids.
    #[must_use]
    pub fn view_ids(&self) -> Vec<String> {
        self.views.iter().map(|v| v.id.clone()).collect()
    }

    /// Whether its code may draw view `view`: one of its `[[views]]`, or a
    /// `[[base_views]]` type its code draws (`base.<id>`,
    /// `linked::Builder::base_view`).
    #[must_use]
    pub fn draws(&self, view: &str) -> bool {
        self.views.iter().any(|d| d.id == view)
            || self
                .contributes
                .base_views
                .iter()
                .any(|b| b.view.is_none() && b.page.is_none() && b.view_name() == view)
    }

    /// What its contributions are checked against: its namespace, its
    /// capabilities, `views`, its kind. A contribution made at run time
    /// (a guest's, a script's) is checked against the same.
    #[must_use]
    pub fn rules<'a>(&'a self, views: &'a [String]) -> Rules<'a> {
        Rules {
            prefix: self.action_prefix(),
            reach: &self.capabilities.actions,
            views,
            luau: self.kind == Kind::Luau,
            events: &self.capabilities.events,
            reads: &self.capabilities.reads,
            writes: &self.capabilities.writes,
        }
    }

    /// Its tours (`[[tours]]`), each with its scenario: the text `read`
    /// gives for its path in the package, read and checked against its
    /// rules ([`crate::contrib::TourDecl::read`]). What a load checks, and
    /// what a test plays.
    ///
    /// # Errors
    ///
    /// The first tour that is missing, does not read or reaches past it.
    pub fn tours(
        &self,
        read: impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<(crate::contrib::TourDecl, architect_guide::Scenario)>, Refused> {
        let views = self.view_ids();
        let rules = self.rules(&views);
        self.contributes
            .tours
            .iter()
            .map(|t| {
                let mut t = t.clone();
                t.scenario_text = read(&t.scenario);
                let scenario = t.read(&rules)?;
                Ok((t, scenario))
            })
            .collect()
    }

    /// Its secret `id` (`[[secrets]]`).
    #[must_use]
    pub fn secret(&self, id: &str) -> Option<&SecretDecl> {
        self.secrets.iter().find(|s| s.id == id)
    }

    /// Whether `secret` is sent to `host` (on `port`): one of its hosts, or
    /// of `capabilities.hosts` when it names none.
    #[must_use]
    pub fn secret_reaches(&self, secret: &SecretDecl, host: &str, port: Option<u16>) -> bool {
        let hosts = if secret.hosts.is_empty() {
            &self.capabilities.hosts
        } else {
            &secret.hosts
        };
        crate::capabilities::reaches_host(hosts, host, port)
            && crate::capabilities::reaches_host(&self.capabilities.hosts, host, port)
    }

    /// The namespace of its theme tokens: the last part of its id, `_`
    /// as `-` (`lab.clicker` → `clicker`, so `clicker.face`).
    #[must_use]
    pub fn token_namespace(&self) -> String {
        self.id
            .rsplit('.')
            .next()
            .unwrap_or(&self.id)
            .replace('_', "-")
    }

    /// The theme files it contributes, relative to its package.
    #[must_use]
    pub fn theme_files(&self) -> Vec<String> {
        self.themes.iter().map(|t| t.path.clone()).collect()
    }

    /// Its settings sections as the settings page takes them, each owned
    /// by this extension (removed with it).
    #[must_use]
    pub fn sections(&self) -> Vec<Section> {
        self.sections
            .iter()
            .map(|decl| section_of(&self.id, decl))
            .collect()
    }

    /// What it contributes to each point, as JSON items: its
    /// `[[contribute."<point>"]]` tables, and its `[[sections]]` as items
    /// of `settings.sections` (the sugar), first.
    #[must_use]
    pub fn point_items(&self) -> std::collections::BTreeMap<String, Vec<serde_json::Value>> {
        let mut out: std::collections::BTreeMap<String, Vec<serde_json::Value>> = self
            .contribute
            .iter()
            .map(|(point, items)| {
                (
                    point.clone(),
                    items.iter().map(crate::interop::toml_json).collect(),
                )
            })
            .collect();
        if !self.sections.is_empty() {
            let sugar = self
                .sections
                .iter()
                .filter_map(|s| serde_json::to_value(s).ok());
            let list = out
                .entry(crate::interop::SETTINGS_SECTIONS.to_owned())
                .or_default();
            let rest = std::mem::take(list);
            list.extend(sugar);
            list.extend(rest);
        }
        out
    }
}

/// A `settings.sections` item (`decl`) from `owner` as the settings page
/// draws it.
///
/// `owner` is an extension's id, or `app`. Its view, when it names one, is
/// `<owner>/<view>`; its group `Extensions` unless it says (`""` for the
/// first, untitled group). A setting that cannot be drawn is left out.
#[must_use]
pub fn section_of(owner: &str, decl: &SectionDecl) -> Section {
    let mut section = Section::new(&decl.id, &decl.title);
    if owner != crate::interop::APP {
        section = section.owned_by(owner);
    }
    let default_group = if owner == crate::interop::APP {
        ""
    } else {
        "Extensions"
    };
    section = match decl.group.as_deref().unwrap_or(default_group) {
        "" => section.ungrouped(),
        group => section.group(group),
    };
    if let Some(view) = &decl.view {
        section = section.view(format!("{owner}/{view}"));
    }
    if !decl.icon.is_empty() {
        section = section.icon(&decl.icon);
    }
    for s in &decl.settings {
        let Ok(made) = setting(s) else { continue };
        let title = s.block.clone();
        match section.blocks.iter_mut().find(|b| b.title == title) {
            Some(block) => block.settings.push(made),
            None => section.blocks.push(Block {
                title,
                settings: vec![made],
            }),
        }
    }
    section
}

/// A section as a `settings.sections` item: the app's own, contributed
/// as `app`. Its `view` is `page` when the app draws it with a page of
/// its own (`page`: true).
#[must_use]
pub fn decl_of(section: &Section, page: bool) -> SectionDecl {
    let text = |v: &Value| -> toml::Value {
        match v {
            Value::Bool(b) => toml::Value::Boolean(*b),
            Value::Int(i) => toml::Value::Integer(*i),
            Value::Float(f) => toml::Value::Float(*f),
            Value::Text(t) => toml::Value::String(t.clone()),
        }
    };
    let choice_text = |v: &Value| match v {
        Value::Text(t) => t.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
    };
    let mut settings = Vec::new();
    for block in &section.blocks {
        for s in &block.settings {
            let mut d = SettingDecl {
                key: s.key.clone(),
                title: s.title.clone(),
                description: s.description.clone(),
                block: block.title.clone(),
                control: String::new(),
                default: text(&s.default),
                min: None,
                max: None,
                step: None,
                unit: String::new(),
                placeholder: String::new(),
                choices: Vec::new(),
                aliases: s.aliases.clone(),
                scope: match s.scope {
                    architect_settings::Scope::Device => None,
                    architect_settings::Scope::Synced => Some("synced".to_owned()),
                    architect_settings::Scope::Project => Some("project".to_owned()),
                },
            };
            let choices = |c: &[Choice]| -> Vec<ChoiceDecl> {
                c.iter()
                    .map(|c| ChoiceDecl {
                        value: choice_text(&c.value),
                        label: c.label.clone(),
                        detail: c.detail.clone(),
                    })
                    .collect()
            };
            match &s.control {
                Control::Switch => "switch".clone_into(&mut d.control),
                Control::Select(c) => {
                    "select".clone_into(&mut d.control);
                    d.choices = choices(c);
                }
                Control::Segmented(c) => {
                    "segmented".clone_into(&mut d.control);
                    d.choices = choices(c);
                }
                Control::Cards(c) => {
                    "cards".clone_into(&mut d.control);
                    d.choices = choices(c);
                }
                Control::Slider {
                    min,
                    max,
                    step,
                    unit,
                } => {
                    "slider".clone_into(&mut d.control);
                    (d.min, d.max, d.step, d.unit) =
                        (Some(*min), Some(*max), Some(*step), unit.clone());
                }
                Control::Stepper {
                    min,
                    max,
                    step,
                    unit,
                } => {
                    "stepper".clone_into(&mut d.control);
                    #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
                    {
                        (d.min, d.max, d.step) =
                            (Some(*min as f64), Some(*max as f64), Some(*step as f64));
                    }
                    d.unit.clone_from(unit);
                }
                Control::Text { placeholder } => {
                    "text".clone_into(&mut d.control);
                    d.placeholder.clone_from(placeholder);
                }
                Control::Shortcut => "shortcut".clone_into(&mut d.control),
            }
            settings.push(d);
        }
    }
    SectionDecl {
        id: section.id.clone(),
        title: section.title.clone(),
        icon: section.icon.clone(),
        group: Some(section.group.clone().unwrap_or_default()),
        settings,
        view: page.then(|| "page".to_owned()),
    }
}

/// The action namespace of extension `id` ([`Manifest::action_prefix`]).
#[must_use]
pub fn action_prefix(id: &str) -> String {
    let mut out = String::with_capacity(id.len().saturating_add(1));
    for c in id.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    if !out.ends_with('_') {
        out.push('_');
    }
    out
}

/// Whether a manifest's `api` is one this build runs: the same major,
/// and — while it is `0.x` — the same minor, no newer than ours.
fn api_ok(wants: &str) -> bool {
    let parse = |v: &str| -> Option<(u64, u64)> {
        let mut parts = v.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        Some((major, minor))
    };
    match (parse(wants), parse(API)) {
        (Some((0, w)), Some((0, h))) => w == h,
        (Some((wm, wn)), Some((hm, hn))) => wm == hm && wn <= hn,
        _ => false,
    }
}

/// A relative path that stays in its package.
fn inside(path: &str) -> Result<(), ManifestError> {
    let p = std::path::Path::new(path);
    let ok = !path.is_empty()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
    if ok {
        Ok(())
    } else {
        Err(ManifestError::BadPath(path.to_owned()))
    }
}

/// A declared setting as the settings page takes it.
fn setting(s: &SettingDecl) -> Result<Setting, ManifestError> {
    let bad = |why: &str| ManifestError::BadSetting {
        key: s.key.clone(),
        why: why.to_owned(),
    };
    let choices = || -> Vec<Choice> {
        s.choices
            .iter()
            .map(|c| Choice::text(&c.value, &c.label).detail(&c.detail))
            .collect()
    };
    let (control, default) = match s.control.as_str() {
        "switch" => (
            Control::Switch,
            Value::Bool(
                s.default
                    .as_bool()
                    .ok_or_else(|| bad("its default is not true or false"))?,
            ),
        ),
        "select" | "segmented" | "cards" => {
            let control = match s.control.as_str() {
                "select" => Control::Select(choices()),
                "segmented" => Control::Segmented(choices()),
                _ => Control::Cards(choices()),
            };
            let default = s
                .default
                .as_str()
                .ok_or_else(|| bad("its default is not text"))?;
            if !s.choices.iter().any(|c| c.value == default) {
                return Err(bad("its default is not one of its choices"));
            }
            (control, Value::Text(default.to_owned()))
        }
        "slider" => (
            Control::Slider {
                min: s.min.unwrap_or(0.0),
                max: s.max.unwrap_or(100.0),
                step: s.step.unwrap_or(1.0),
                unit: s.unit.clone(),
            },
            Value::Float(number(&s.default).ok_or_else(|| bad("its default is not a number"))?),
        ),
        "stepper" => (
            Control::Stepper {
                min: whole(s.min.unwrap_or(0.0)),
                max: whole(s.max.unwrap_or(100.0)),
                step: whole(s.step.unwrap_or(1.0)),
                unit: s.unit.clone(),
            },
            Value::Int(
                s.default
                    .as_integer()
                    .ok_or_else(|| bad("its default is not a whole number"))?,
            ),
        ),
        "text" => (
            Control::Text {
                placeholder: s.placeholder.clone(),
            },
            Value::Text(
                s.default
                    .as_str()
                    .ok_or_else(|| bad("its default is not text"))?
                    .to_owned(),
            ),
        ),
        "shortcut" => (
            Control::Shortcut,
            Value::Text(
                s.default
                    .as_str()
                    .ok_or_else(|| bad("its default is not a chord"))?
                    .to_owned(),
            ),
        ),
        other => return Err(bad(&format!("no control called “{other}”"))),
    };
    let scope = match s.scope.as_deref() {
        None | Some("device") => architect_settings::Scope::Device,
        Some("synced") => architect_settings::Scope::Synced,
        Some("project") => architect_settings::Scope::Project,
        Some(other) => return Err(bad(&format!("no scope called “{other}”"))),
    };
    Ok(Setting::new(&s.key, &s.title, control, default)
        .describe(&s.description)
        .aliases(s.aliases.iter().cloned())
        .scope(scope))
}

fn number(v: &toml::Value) -> Option<f64> {
    v.as_float().or_else(|| {
        v.as_integer()
            .and_then(|i| i32::try_from(i).ok())
            .map(f64::from)
    })
}

/// A whole number from a manifest's `min`/`max`/`step`.
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn whole(f: f64) -> i64 {
    // Saturating: `as` clamps a float into i64 and maps NaN to 0.
    f.round() as i64
}

#[cfg(test)]
mod tests {
    use super::{Kind, Manifest, ManifestError, action_prefix};
    use architect_settings::{Control, Owner, Value};

    const METRONOME: &str = r#"
id = "fts.metronome"
name = "Metronome"
version = "0.1.0"
kind = "luau"

[[actions]]
id = "FTS_METRONOME_COUNT_IN"
name = "Count in"
script = "count_in.luau"

[[sections]]
id = "metronome"
title = "Metronome"
icon = "timer"

[[sections.settings]]
key = "fts.metronome.accent"
title = "Accent the downbeat"
control = "switch"
default = true

[[sections.settings]]
key = "fts.metronome.volume"
title = "Click volume"
control = "slider"
max = 100
unit = "%"
default = 70

[[sections.settings]]
key = "fts.metronome.sound"
title = "Sound"
block = "Sound"
control = "select"
default = "wood"
choices = [{ value = "wood", label = "Woodblock" }, { value = "beep", label = "Beep" }]
"#;

    #[test]
    fn a_manifest_reads_into_settings_sections_it_owns() {
        let m = Manifest::parse(METRONOME).unwrap();
        assert_eq!(m.kind, Kind::Luau);
        assert_eq!(m.version.to_string(), "0.1.0");
        let sections = m.sections();
        assert_eq!(sections.len(), 1);
        let s = &sections[0];
        assert_eq!(s.owner, Owner::Extension("fts.metronome".into()));
        assert_eq!(s.group.as_deref(), Some("Extensions"));
        assert_eq!(s.blocks.len(), 2, "untitled, then Sound");
        let volume = s
            .settings()
            .find(|t| t.key == "fts.metronome.volume")
            .unwrap();
        assert_eq!(volume.default, Value::Float(70.0));
        assert!(
            matches!(volume.control, Control::Slider { max, .. } if (max - 100.0).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn a_section_drawn_by_one_of_its_views() {
        const PAGE: &str = r#"
id = "manager"
name = "Extensions"
version = "0.1.0"
kind = "process"

[[views]]
id = "section"
title = "Extensions"

[[sections]]
id = "extensions"
title = "Extensions"
icon = "puzzle"
group = ""
view = "section"
"#;
        let m = Manifest::parse(PAGE).unwrap();
        let s = &m.sections()[0];
        assert_eq!(s.view.as_deref(), Some("manager/section"));
        assert_eq!(s.group, None, "an empty group: the first, untitled one");
        assert!(s.blocks.is_empty());
        let e =
            Manifest::parse(&PAGE.replace("view = \"section\"", "view = \"gone\"")).unwrap_err();
        assert!(e.to_string().contains("no view \"gone\""), "{e}");
    }

    #[test]
    fn a_worker_has_one_way_to_run_that_its_kind_has() {
        let with = |worker: &str| Manifest::parse(&format!("{METRONOME}\n[[workers]]\n{worker}"));
        let m = with("id = \"primes\"\nscript = \"workers/primes.luau\"").unwrap();
        assert_eq!(m.workers[0].script.as_deref(), Some("workers/primes.luau"));
        for (bad, why) in [
            ("id = \"p\"", "no script"),
            (
                "id = \"p\"\nscript = \"a.luau\"\nentry = \"p\"",
                "and an entry",
            ),
            ("id = \"p\"\nentry = \"p\"", "process or wasm"),
        ] {
            let e = with(bad).unwrap_err();
            assert!(e.to_string().contains(why), "{bad}: {e}");
        }
        assert!(matches!(
            with("id = \"p\"\nscript = \"../p.luau\""),
            Err(ManifestError::BadPath(_))
        ));
        let twice =
            with("id = \"p\"\nscript = \"a.luau\"\n[[workers]]\nid = \"p\"\nscript = \"b.luau\"");
        assert!(matches!(twice, Err(ManifestError::Duplicate(_))));
    }

    #[test]
    fn themes_are_json_files_inside_the_package() {
        let with = |theme: &str| Manifest::parse(&format!("{METRONOME}\n[[themes]]\n{theme}"));
        let m = with("path = \"themes/night.json\"").unwrap();
        assert_eq!(m.theme_files(), ["themes/night.json"]);
        assert!(matches!(
            with("path = \"../night.json\""),
            Err(ManifestError::BadPath(_))
        ));
        assert!(matches!(
            with("path = \"themes/night.toml\""),
            Err(ManifestError::BadPath(_))
        ));
        assert!(with("path = \"night.json\"\nlabel = \"x\"").is_err());
    }

    #[test]
    fn a_manifest_keeps_to_its_namespaces_and_its_api() {
        let bad = |from: &str, to: &str| Manifest::parse(&METRONOME.replacen(from, to, 1));
        assert!(matches!(
            bad("FTS_METRONOME_COUNT_IN", "TRANSPORT_PLAY"),
            Err(ManifestError::ActionNamespace { .. })
        ));
        assert!(matches!(
            bad(
                "key = \"fts.metronome.accent\"",
                "key = \"appearance.theme\""
            ),
            Err(ManifestError::SettingNamespace(_))
        ));
        assert!(matches!(
            bad("kind = \"luau\"", "kind = \"data\""),
            Err(ManifestError::ScriptKind(_))
        ));
        assert!(matches!(
            bad("count_in.luau", "../outside.luau"),
            Err(ManifestError::BadPath(_))
        ));
        assert!(matches!(
            bad("kind = \"luau\"", "kind = \"luau\"\napi = \"0.9\""),
            Err(ManifestError::Api { .. })
        ));
        assert!(matches!(
            bad("default = \"wood\"", "default = \"cowbell\""),
            Err(ManifestError::BadSetting { .. })
        ));
        assert!(matches!(
            bad("id = \"fts.metronome\"", "id = \"FTS\""),
            Err(ManifestError::BadId(_))
        ));
    }

    #[test]
    fn an_overlay_is_a_script_on_a_plane_of_a_view() {
        let with =
            |overlay: &str| Manifest::parse(&format!("{METRONOME}\n[[overlays]]\n{overlay}"));
        let m =
            with("id = \"clicks\"\nview = \"session.arrange\"\nscript = \"clicks.luau\"").unwrap();
        let o = &m.overlays[0];
        assert_eq!(
            (o.view.as_str(), o.plane.as_str()),
            ("session.arrange", "over")
        );
        assert!(matches!(
            with("id = \"c\"\nview = \"v\"\nplane = \"side\"\nscript = \"c.luau\""),
            Err(ManifestError::BadPlane { .. })
        ));
        assert!(matches!(
            with("id = \"c\"\nview = \"v\"\nscript = \"/etc/c.luau\""),
            Err(ManifestError::BadPath(_))
        ));
        assert!(matches!(
            with("id = \"metronome\"\nview = \"v\"\nscript = \"c.luau\""),
            Err(ManifestError::Duplicate(_))
        ));
    }

    const CONTRIB: &str = r#"
id = "lab.contrib"
name = "Board"
version = "0.1.0"
kind = "process"

[capabilities]
actions = ["WORKSPACE_"]

[entry]
path = "architect-lab-contrib"

[[actions]]
id = "LAB_CONTRIB_PIN"
name = "Pin to the board"

[[views]]
id = "board"
title = "Board"
placement = "split"
on_unload = "close"

[[views]]
id = "chart"
title = "Chart"
home = "right"

[[views]]
id = "bar"
title = "Board status bar"

[[openers]]
id = "kf"
title = "Keyflow chart"
files = ["kf"]
view = "chart"

[[openers]]
id = "issue"
title = "GitHub issue"
urls = ['^https://github\.com/[^/]+/[^/]+/issues/\d+$']
action = "LAB_CONTRIB_PIN"

[[doc_kinds]]
id = "keyflow.chart"
name = "Chart"
files = ["kf"]
view = "chart"

[[tabs]]
id = "board"
title = "Board"
icon = "layout-dashboard"
view = "board"

[[status]]
id = "pins"
text = "No pins"
action = "LAB_CONTRIB_PIN"

[[menus]]
menu = "View"
action = "LAB_CONTRIB_PIN"

[[context]]
target = "view:note"
action = "LAB_CONTRIB_PIN"
title = "Pin to the board"

[[bar]]
action = "LAB_CONTRIB_PIN"
title = "Pin"
icon = "pin"

[[hooks]]
on = "startup"
action = "LAB_CONTRIB_PIN"

[[hooks]]
on = "changed:workspace"
action = "WORKSPACE_LAYOUT"

[[slots]]
slot = "status"
view = "bar"

[[embeds]]
id = "kf"
title = "Keyflow chart"
view = "chart"
claims = { fence = ["kf"], note_type = ["song"] }
max_height = 320

[[base_views]]
id = "board"
name = "Board"
icon = "kanban"
options = [{ key = "columnBy", name = "Columns", kind = "property" }]

[[base_views]]
id = "timeline"
name = "Timeline"
view = "chart"

[[navigator_providers]]
id = "songs"
title = "Setlists"
icon = "list-music"
views = ["chart"]
actions = "LAB_CONTRIB_NAV"

[[deck_templates]]
context = "song"
screenset = "Song"
layout = "layouts/song.json"

[[tab_kinds]]
id = "song"
title = "Song"
scope = true
width = "wide"
subtitle = "{key} · {tempo} bpm"

[[tab_kinds]]
id = "chart-tab"
title = "Chart"
views = ["chart"]
menu = [{ label = "Transpose", action = "LAB_CONTRIB_PIN" }]
"#;

    #[test]
    fn a_manifest_reads_what_it_adds_to_the_interface() {
        use super::{Home, OnUnload, ViewPlacement};
        let m = Manifest::parse(CONTRIB).unwrap();
        let c = &m.contributes;
        assert_eq!(c.openers.len(), 2);
        assert_eq!(c.openers[0].files, ["kf"]);
        assert!(c.openers[1].urls[0].starts_with("^https"));
        assert_eq!(
            (c.tabs.len(), c.status.len(), c.menus.len(), c.context.len()),
            (1, 1, 1, 1)
        );
        assert_eq!((c.bar.len(), c.hooks.len(), c.slots.len()), (1, 2, 1));
        assert_eq!(c.embeds[0].claims.fence, ["kf"]);
        assert_eq!(c.embeds[0].claims.note_type, ["song"]);
        assert_eq!(c.base_views.len(), 2);
        assert_eq!(c.base_views[0].options[0].key, "columnBy");
        // Drawn by its code: `base.board`, which it may draw; the other by
        // its view.
        assert_eq!(c.base_views[0].view_name(), "base.board");
        assert!(m.draws("base.board") && !m.draws("base.timeline"));
        assert_eq!(c.base_views[1].view_name(), "chart");
        assert_eq!(c.navigator_providers[0].views, ["chart"]);
        assert_eq!(
            c.navigator_providers[0].snapshot_action(),
            "LAB_CONTRIB_NAV_SNAPSHOT"
        );
        assert_eq!(c.deck_templates[0].context, "song");
        assert_eq!(c.tab_kinds.len(), 2);
        assert!(c.tab_kinds[0].scope);
        assert_eq!(c.tab_kinds[0].width.as_deref(), Some("wide"));
        assert_eq!(
            c.tab_kinds[0].subtitle.as_deref(),
            Some("{key} · {tempo} bpm")
        );
        assert_eq!(c.tab_kinds[1].views, ["chart"]);
        assert_eq!(c.tab_kinds[1].menu[0].action, "LAB_CONTRIB_PIN");
        assert_eq!(
            c.deck_templates[0].layout.as_deref(),
            Some("layouts/song.json")
        );
        assert_eq!(c.embeds[0].max_height, Some(320));
        assert_eq!(m.views[0].placement, ViewPlacement::Split);
        assert_eq!(m.views[0].on_unload, OnUnload::Close);
        assert_eq!(m.views[1].home, Home::Right);
        assert_eq!(m.views[1].workspace_kind(&m.id), "lab.contrib/chart");

        let bad = |from: &str, to: &str| Manifest::parse(&CONTRIB.replacen(from, to, 1));
        assert!(matches!(
            bad("view = \"board\"", "view = \"nope\""),
            Err(ManifestError::Contribution(_))
        ));
        // A provider goes with views it has, and its actions are its own.
        assert!(matches!(
            bad("views = [\"chart\"]", "views = [\"nope\"]"),
            Err(ManifestError::Contribution(_))
        ));
        assert!(matches!(
            bad("actions = \"LAB_CONTRIB_NAV\"", "actions = \"VAULT_NAV\""),
            Err(ManifestError::Contribution(_))
        ));
        assert!(matches!(
            bad("on = \"startup\"", "on = \"teatime\""),
            Err(ManifestError::Contribution(_))
        ));
        assert!(matches!(
            bad("actions = [\"WORKSPACE_\"]", "actions = []"),
            Err(ManifestError::Contribution(_))
        ));
    }

    #[test]
    fn an_id_makes_an_action_namespace() {
        assert_eq!(action_prefix("fts.metronome"), "FTS_METRONOME_");
        assert_eq!(action_prefix("a--b"), "A_B_");
    }
}

#[cfg(test)]
mod favourites_tests {
    /// `[[favourites]]` reads into the manifest, and one that breaks a
    /// rule makes the manifest fail to read.
    #[test]
    fn a_manifest_reads_its_favourites_and_refuses_a_bad_one() {
        let text = "id = \"fts.studio\"\nname = \"Studio\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n\
                    [[favourites]]\nnote_type = \"setlist\"\n\
                    [[favourites.tabs]]\nid = \"setlist\"\nlabel = \"Setlist\"\nicon = \"list-music\"\ntarget = { view = \"page\" }\n\
                    [favourites.primary]\nid = \"play\"\nlabel = \"Play\"\nicon = \"play\"\ntarget = { action = \"PLAYER_TOGGLE\" }\n\
                    state = \"playing\"\non = { label = \"Pause\", icon = \"pause\" }\n";
        let manifest = super::Manifest::parse(text).expect("it reads");
        assert_eq!(manifest.favourites.len(), 1);
        assert!(manifest.favourites[0].is_for("setlist"));
        assert_eq!(manifest.favourites[0].tabs[0].label, "Setlist");
        // One that breaks a rule is dropped and said, not fatal: the rest
        // of the manifest (its share layers) still reads.
        let bad = text.replace(
            "{ view = \"page\" }",
            "{ place = \"https://evil.example/\" }",
        ) + "\n[[share_layers]]\nnote_type = \"song\"\nfull = [\"Media/**\"]\n";
        let manifest = super::Manifest::parse(&bad).expect("it still reads");
        assert!(manifest.favourites.is_empty());
        assert_eq!(
            manifest.favourites_dropped.len(),
            1,
            "{:?}",
            manifest.favourites_dropped
        );
        assert!(manifest.favourites_dropped[0].starts_with("setlist: tab"));
        assert_eq!(manifest.share_layers.len(), 1);
    }

    /// A declaration of the wrong shape (a misspelt key, an unknown kind of
    /// target, not a table at all) is dropped and said like one that breaks
    /// a rule; the good ones beside it and the rest of the manifest read.
    #[test]
    fn a_misshapen_favourites_declaration_is_dropped_never_the_manifest() {
        let head = "id = \"fts.studio\"\nname = \"Studio\"\nversion = \"0.1.0\"\nkind = \"data\"\n";
        let good = "[[favourites]]\nnote_type = \"setlist\"\n[[favourites.tabs]]\nid = \"setlist\"\nlabel = \"Setlist\"\nicon = \"list-music\"\ntarget = { view = \"page\" }\n";
        let layers = "\n[[share_layers]]\nnote_type = \"song\"\nfull = [\"Media/**\"]\n";
        for bad in [
            // A misspelt key.
            "[[favourites]]\nnote_type = \"song\"\n[[favourites.tabs]]\nid = \"song\"\nlable = \"Song\"\nicon = \"music\"\ntarget = { view = \"page\" }\n",
            // An unknown kind of target.
            "[[favourites]]\nnote_type = \"song\"\n[[favourites.tabs]]\nid = \"song\"\nlabel = \"Song\"\nicon = \"music\"\ntarget = { teleport = \"x\" }\n",
            // A field of the wrong type.
            "[[favourites]]\nnote_type = 3\n",
        ] {
            let text = format!("{head}{good}{bad}{layers}");
            let manifest = super::Manifest::parse(&text).unwrap_or_else(|e| panic!("{bad}: {e}"));
            assert_eq!(manifest.favourites.len(), 1, "{bad}");
            assert!(manifest.favourites[0].is_for("setlist"));
            assert_eq!(manifest.favourites_dropped.len(), 1, "{bad}");
            assert_eq!(manifest.share_layers.len(), 1);
        }
        // Not a list of tables at all.
        let manifest = super::Manifest::parse(&format!("{head}favourites = 3\n{layers}")).unwrap();
        assert!(manifest.favourites.is_empty());
        assert_eq!(manifest.favourites_dropped.len(), 1);
        assert_eq!(manifest.share_layers.len(), 1);
    }
}
