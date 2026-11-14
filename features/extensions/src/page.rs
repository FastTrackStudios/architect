//! The extensions page (#181, feature `page`): find, install, update,
//! pin, turn off, remove — and see what each one adds, asks for and runs
//! as — in the app's theme, with architect-ui's components.
//!
//! - An extension's page (the `extension` kind's): what it
//!   adds, the capabilities it asks for (granted or not, each grant
//!   revocable), its versions and pin, where it runs, its recent log, and
//!   "built in, overridden by 1.4.0-dev" with the way back.
//! - [`ConsentPrompt`]: what an install (or an update that asks for more)
//!   will let it do, granted on this device or in the organization.
//! - [`MissingExtensions`]: a project's "This project uses …" with
//!   "Install all", the same consent prompt for all of them.
//! - [`ManagerPage`]: the extensions manager's page (`crate::manager`):
//!   every kind it manages, searched together, each item's page and ops;
//!   an extension's page is the one above.
//! - [`use_manager_page`]: what the manager's views read. The `manager`
//!   extension's manifest makes it a section of the settings page
//!   ([`ManagerSection`], its `section` view) and ⌘O's `ext:` opener; an
//!   app with a workspace puts [`ManagerView`] in a view kind of its own
//!   as well (it is the `manager` extension's `manager` view too).
//!
//! Every colour is the theme's (`var(--…)`, architect-ui's classes over
//! the store's CSS variables); nothing here names one.

// `rsx!` clones what each closure takes (clippy reads them as redundant);
// an extension's info is a `serde_json::Value`, whose indexing gives
// `Null` for what is not there and never panics; a hook takes what its
// context keeps.
#![allow(
    clippy::redundant_clone,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use architect_ui::prelude::*;
use dioxus::prelude::*;
use semver::VersionReq;

use crate::browse::{Browse, Row, Where};
use crate::consent::Scope;
use crate::manage::{Installed, Preview};
use crate::manager::{Item, Manager};
use crate::project::Plan;

/// The settings section the page is.
pub const SECTION: &str = "extensions";

/// What a consent prompt is for.
#[derive(Clone, Debug, PartialEq)]
enum For {
    Install { pin: bool },
    Update,
    Grant,
}

#[derive(Clone, Debug, PartialEq)]
struct Asking {
    previews: Vec<Preview>,
    purpose: For,
}

/// The page's state, shared by its parts.
#[derive(Clone, Copy)]
struct State {
    query: Signal<String>,
    /// The kind shown (every kind when `None`).
    kind: Signal<Option<String>>,
    /// The item picked: its kind and id.
    selected: Signal<Option<(String, String)>>,
    /// Bumped after every change: what reads the items reads again.
    rev: Signal<u64>,
    asking: Signal<Option<Asking>>,
    note: Signal<Option<(bool, String)>>,
}

fn state() -> State {
    use_context::<State>()
}

fn bump(s: State) {
    let mut rev = s.rev;
    let next = rev.peek().wrapping_add(1);
    rev.set(next);
}

fn say(s: State, ok: bool, text: impl Into<String>) {
    let mut note = s.note;
    note.set(Some((ok, text.into())));
}

/// What the manager's views read, in context, and the page opened at an
/// item when `MANAGER_SHOW` (⌘O's `ext:`) asks.
///
/// The manager, `browse` (the extensions' catalog, for the extension
/// kind's own page) and the page's state.
///
/// Its settings section and ⌘O's `ext:` are the `manager` extension's
/// (`manager.toml`'s `[[sections]]` and `[[openers]]`): call it under the
/// settings provider, above the view the section is drawn by.
pub fn use_manager_page(manager: Manager, browse: Browse) {
    use_context_provider(|| browse.clone());
    use_context_provider(|| manager.clone());
    let s = use_context_provider(|| State {
        query: Signal::new(String::new()),
        kind: Signal::new(None),
        selected: Signal::new(None),
        rev: Signal::new(0),
        asking: Signal::new(None),
        note: Signal::new(None),
    });
    let settings = architect_settings::try_use_settings();
    let shown = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(Some(manager.shown()))));
    use_future(move || {
        let shown = shown.borrow_mut().take();
        async move {
            use futures::StreamExt as _;
            let Some(mut shown) = shown else { return };
            while let Some((kind, id)) = shown.next().await {
                let mut selected = s.selected;
                selected.set(Some((kind, id)));
                if let Some(h) = settings {
                    h.show(Some(SECTION));
                }
            }
        }
    });
}

/// The manager as the settings page's section: in the page's own scroll.
#[component]
pub fn ManagerSection() -> Element {
    let manager = use_context::<Manager>();
    rsx! { ManagerPage { manager } }
}

/// What stays above the list's scrolled-out rows: Blitz clips what a
/// scroller draws, not what it hit-tests, so a row scrolled up under the
/// search field would take its clicks (`skills/architect/references/blitz-quirks.md`).
const LIFT: &str = "position: relative; z-index: 1; background: var(--background);";

/// Below this width (CSS px) the manager in a pane shows the list or one
/// item's page, not both.
pub const NARROW: f64 = 620.0;

/// The manager in a workspace pane (and the `manager` extension's view):
/// it fills the pane, the list and the item's page each scroll on their
/// own, side by side — or, in a pane narrower than [`NARROW`], the list
/// and then the item picked, with the way back.
#[component]
pub fn ManagerView() -> Element {
    let manager = use_context::<Manager>();
    let mut width = use_signal(|| None::<f64>);
    let mut mounted = use_signal(|| None::<std::rc::Rc<MountedData>>);
    // Its width, measured as the pane is resized (Blitz has no `onresize`):
    // `peek`s, so the loop is not restarted by what it sets.
    use_future(move || async move {
        loop {
            let at = mounted.peek().clone();
            if let Some(at) = at
                && let Ok(r) = at.get_client_rect().await
            {
                let w = r.size.width;
                if w > 0.0 && width.peek().is_none_or(|was| (was - w).abs() > 0.5) {
                    width.set(Some(w));
                }
            }
            futures_timer::Delay::new(std::time::Duration::from_millis(200)).await;
        }
    });
    let narrow = width().is_some_and(|w| w < NARROW);
    rsx! {
        div {
            style: "height: 100%; width: 100%; box-sizing: border-box; display: flex; flex-direction: column; overflow: hidden; padding: 12px 14px; background: var(--background); color: var(--foreground);",
            onmounted: move |e| mounted.set(Some(e.data())),
            ManagerPage { manager, fill: true, narrow }
        }
    }
}

/// The manager's page: kinds, search across them, the list, and the
/// page of the item picked.
///
/// `fill`: it fills its parent (a flex column of a set height, as
/// [`ManagerView`]'s), the list and the item's page each scrolling on
/// their own; otherwise it is as tall as it is, in its parent's scroll
/// (the settings page). `narrow`, with `fill`: the list, or the item
/// picked with the way back.
#[component]
pub fn ManagerPage(
    manager: Manager,
    #[props(default)] fill: bool,
    #[props(default)] narrow: bool,
) -> Element {
    let s = state();
    let _ = (s.rev)();
    let query = (s.query)();
    let kind = (s.kind)();
    let items = manager.list(kind.as_deref(), &query);
    let picked = (s.selected)().filter(|(k, i)| items.iter().any(|x| &x.kind == k && &x.id == i));
    let master_detail = fill && narrow;
    let selected = if master_detail {
        picked.clone()
    } else {
        picked
            .clone()
            .or_else(|| items.first().map(|i| (i.kind.clone(), i.id.clone())))
    };
    let mut kinds = vec![(String::new(), "All".to_owned())];
    kinds.extend(
        manager
            .kinds()
            .iter()
            .map(|k| (k.id().to_owned(), k.title().to_owned())),
    );
    let browse = try_use_context::<Browse>();
    let org = browse
        .as_ref()
        .and_then(|b| b.extensions().host().grants())
        .and_then(|g| g.org());
    let (root, body, list_col, list_box, page_col) = match (fill, narrow) {
        (true, false) => (
            "flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 12px;",
            "flex: 1; min-height: 0; display: flex; gap: 16px;",
            "flex: 0 0 300px; min-height: 0; display: flex; flex-direction: column; gap: 8px;",
            "flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: 10px;",
            "flex: 1; min-width: 0; min-height: 0; overflow-y: auto;",
        ),
        (true, true) => (
            "flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 10px;",
            "flex: 1; min-height: 0; display: flex; flex-direction: column;",
            "flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 8px;",
            "flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: 10px;",
            "flex: 1; min-width: 0; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 8px;",
        ),
        // In the parent's scroll: no scroller of its own (a scroller inside
        // the page's scroller loses its clicks in Blitz). Side by side when
        // there is room; the page under the list when not.
        (false, _) => (
            "display: flex; flex-direction: column; gap: 14px;",
            "display: flex; flex-wrap: wrap; gap: 16px; align-items: flex-start;",
            "flex: 1 1 260px; max-width: 360px; display: flex; flex-direction: column; gap: 8px;",
            "display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: 10px;",
            "flex: 999 1 380px; min-width: 0;",
        ),
    };
    let show_list = !master_detail || picked.is_none();
    let show_page = !master_detail || picked.is_some();
    rsx! {
        style { {architect_ui::UTILITIES_CSS} }
        div { style: "{root} font-family: inherit; color: var(--foreground);",
            if let (Some(asking), Some(browse)) = ((s.asking)(), browse.clone()) {
                div { style: "{LIFT} flex: none;",
                    ConsentPrompt {
                        previews: asking.previews.clone(),
                        org,
                        action: match asking.purpose { For::Install { .. } => "Install", For::Update => "Update", For::Grant => "Grant" }.to_owned(),
                        on_answer: move |scope: Option<Scope>| answer(&browse, s, &asking, scope),
                    }
                }
            }
            if let Some((ok, text)) = (s.note)() {
                div { style: "{LIFT} flex: none;",
                    Alert { variant: if ok { AlertVariant::Default } else { AlertVariant::Destructive },
                        AlertDescription { "{text}" }
                    }
                }
            }
            if let Some(browse) = browse.clone().filter(|b| b.extensions().registries().is_some()) {
                div { style: "{LIFT} flex: none;",
                    RegistriesPanel { browse }
                }
            }
            div { style: "{LIFT} flex: none; max-width: 100%; overflow-x: auto;",
                SegmentedControl {
                    value: kind.clone().unwrap_or_default(),
                    options: kinds,
                    on_change: move |v: String| {
                        let mut k = s.kind;
                        k.set((!v.is_empty()).then_some(v));
                    },
                }
            }
            div { style: "{body}",
                if show_list {
                    div { style: "{list_col}",
                        div { style: "{LIFT} flex: none;",
                            Input { value: s.query, placeholder: "Search every kind".to_owned() }
                        }
                        div { style: "{LIFT} flex: none; font-size: 12px; color: var(--muted-foreground);",
                            "{items.len()} item(s) · by name, kind, source"
                        }
                        // A new list (a scroller at its top) for each search
                        // and kind: Blitz keeps a scroller's offset when what
                        // it holds shrinks, and would show the rows below none.
                        for shown in std::iter::once(format!("{}\u{1f}{query}", kind.clone().unwrap_or_default())) {
                            div { key: "{shown}", class: "manager-list", style: "{list_box}",
                                if items.is_empty() {
                                    EmptyState { message: "Nothing matches".to_owned() }
                                }
                                for i in items.iter().cloned() {
                                    ItemRow {
                                        key: "{i.kind}/{i.id}",
                                        on: !master_detail && selected.as_ref().is_some_and(|(k, id)| k == &i.kind && id == &i.id),
                                        item: i,
                                    }
                                }
                            }
                        }
                    }
                }
                if show_page && master_detail {
                    div { style: "{LIFT} flex: none; padding-bottom: 6px;",
                        Button {
                            variant: ButtonVariant::Ghost,
                            on_click: move |_| {
                                let mut selected = s.selected;
                                selected.set(None);
                            },
                            "← Every item"
                        }
                    }
                }
                if show_page {
                    // A new scroller (at its top) for each item picked.
                    for shown in std::iter::once(selected.clone().map(|(k, id)| format!("{k}\u{1f}{id}")).unwrap_or_default()) {
                        div { key: "{shown}", class: "manager-item", style: "{page_col}",
                            match (selected.clone(), browse.clone()) {
                                (Some((k, id)), Some(browse)) if k == "extension" => {
                                    let row = browse.rows("").into_iter().find(|r| r.id == id);
                                    rsx! {
                                        if let Some(row) = row {
                                            Detail { key: "{row.id}", browse, row }
                                        }
                                    }
                                }
                                (Some((k, id)), _) => rsx! { ItemPage { key: "{k}/{id}", manager: manager.clone(), kind: k, id } },
                                (None, _) => rsx! { EmptyState { message: "Pick an item".to_owned() } },
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ItemRow(on: bool, item: Item) -> Element {
    use std::fmt::Write as _;
    let s = state();
    let key = (item.kind.clone(), item.id.clone());
    let mut detail = item
        .version
        .clone()
        .map(|v| format!("{v} · "))
        .unwrap_or_default();
    detail.push_str(&item.source);
    if item.enabled == Some(false) {
        detail.push_str(" · off");
    }
    if !item.problems.is_empty() {
        let _ = write!(detail, " · {} problem(s)", item.problems.len());
    }
    let ground = if on { "background: var(--accent);" } else { "" };
    rsx! {
        div { style: "{ground} border-bottom: 1px solid var(--border);",
            ListRow {
                label: item.name.clone(),
                detail,
                tag: item.kind.clone(),
                on_click: move |()| {
                    let mut selected = s.selected;
                    selected.set(Some(key.clone()));
                },
            }
        }
    }
}

/// Any kind's item page: its parts, problems and ops.
#[component]
fn ItemPage(manager: Manager, kind: String, id: String) -> Element {
    let s = state();
    let _ = (s.rev)();
    // Read again each second while it is open: what it shows changes on
    // its own (a page's status, a worker's).
    let mut tick = use_signal(|| 0_u64);
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_secs(1)).await;
            let next = tick.peek().wrapping_add(1);
            tick.set(next);
        }
    });
    let _ = tick();
    let Some(d) = manager.get(&kind, &id) else {
        return rsx! { EmptyState { message: "It is gone".to_owned() } };
    };
    let item = d.item.clone();
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 14px;",
            Card {
                CardHeader {
                    CardTitle { "{item.name}" }
                    CardDescription { "{item.summary}" }
                }
                CardContent {
                    div { style: "display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 10px;",
                        Badge { variant: BadgeVariant::Secondary, "{item.kind}" }
                        if let Some(v) = &item.version { Badge { variant: BadgeVariant::Outline, "{v}" } }
                        if item.enabled == Some(true) { Badge { "On" } }
                        if item.enabled == Some(false) { Badge { variant: BadgeVariant::Outline, "Off" } }
                    }
                    KeyValueRow { label: "Id".to_owned(), value: item.id.clone() }
                    KeyValueRow { label: "Source".to_owned(), value: item.source.clone(), mono: false }
                }
                CardFooter {
                    div { style: "display: flex; flex-wrap: wrap; gap: 8px;",
                        for op in item.ops.iter().cloned() {
                            Button {
                                key: "{op.id}",
                                variant: if op.destructive { ButtonVariant::Destructive } else { ButtonVariant::Outline },
                                on_click: {
                                    let (manager, kind, id, op) = (manager.clone(), kind.clone(), id.clone(), op.clone());
                                    move |_| {
                                        match manager.run(&kind, &id, &op.id, &serde_json::Value::Null) {
                                            Ok(_) => say(s, true, format!("{}: done.", op.label)),
                                            Err(e) => say(s, false, e),
                                        }
                                        bump(s);
                                    }
                                },
                                "{op.label}"
                            }
                        }
                    }
                }
            }
            if !item.problems.is_empty() {
                Alert { variant: AlertVariant::Destructive,
                    AlertTitle { "Problems" }
                    AlertDescription {
                        for (i, p) in item.problems.iter().enumerate() { div { key: "{i}", "{p}" } }
                    }
                }
            }
            for part in d.parts.iter().cloned() {
                Section { key: "{part.title}", title: part.title.clone(),
                    if part.lines.is_empty() { Muted { text: "Nothing yet.".to_owned() } }
                    for (i, line) in part.lines.iter().enumerate() {
                        div { key: "{i}", style: "font-size: 12px; color: var(--foreground); white-space: pre-wrap; font-family: ui-monospace, monospace;", "{line}" }
                    }
                }
            }
        }
    }
}

/// What the person said to the consent prompt.
fn answer(browse: &Browse, s: State, asking: &Asking, scope: Option<Scope>) {
    let mut open = s.asking;
    open.set(None);
    let Some(scope) = scope else {
        say(s, true, "Nothing was installed or granted.");
        return;
    };
    let ext = browse.extensions();
    let mut done = Vec::new();
    for p in &asking.previews {
        let exact = VersionReq::parse(&format!("={}", p.version)).unwrap_or(VersionReq::STAR);
        let result = match asking.purpose {
            For::Grant => {
                if let Some(g) = ext.host().grants() {
                    g.grant(
                        &p.id,
                        &p.version,
                        p.capabilities.clone(),
                        scope.clone(),
                        "person",
                    );
                }
                Ok(format!("granted {}", p.name))
            }
            For::Install { pin } => ext
                .install_with(&p.id, &exact, Some(scope.clone()), "person", pin)
                .map(|i| outcome(&i, &p.name)),
            For::Update => ext
                .install_with(&p.id, &exact, Some(scope.clone()), "person", false)
                .map(|i| outcome(&i, &p.name)),
        };
        match result {
            Ok(text) => done.push(text),
            Err(e) => {
                say(s, false, format!("{}: {e}", p.name));
                bump(s);
                return;
            }
        }
    }
    say(s, true, done.join("; "));
    bump(s);
}

fn outcome(installed: &Installed, name: &str) -> String {
    match installed {
        Installed::Done {
            version, loaded, ..
        } => format!(
            "{name} {version} installed{}",
            if *loaded { " and running" } else { "" }
        ),
        Installed::NeedsConsent { .. } => format!("{name} still needs a yes"),
    }
}

/// The page of one extension.
#[component]
fn Detail(browse: Browse, row: Row) -> Element {
    let s = state();
    let _ = (s.rev)();
    let ext = browse.extensions().clone();
    let info = ext.info(&row.id);
    // What a registry's version would be, read once when picked.
    let preview = use_memo({
        let (browse, row) = (browse.clone(), row.clone());
        move || {
            let _ = (s.rev)();
            if row.at != Where::Available {
                return None;
            }
            browse.preview(&row.id).or_else(|| {
                let p = browse
                    .extensions()
                    .preview(&row.id, &VersionReq::STAR)
                    .ok()?;
                browse.remember(p.clone());
                Some(p)
            })
        }
    });
    let about: Option<Preview> = preview.read().clone().or_else(|| browse.about(&row.id));
    let versions: Vec<String> = info["versions"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter_map(|x| x["version"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let installed = info["installed"]["version"].as_str().map(str::to_owned);
    let pin = info["installed"]["pin"].as_str().map(str::to_owned);
    let may: Option<crate::manifest::Capabilities> =
        serde_json::from_value(info["may_reach"].clone()).ok();
    let grants: Vec<crate::consent::Grant> =
        serde_json::from_value(info["grants"].clone()).unwrap_or_default();
    let logs = ext.host().logs().since(&row.id, None, 6);
    let id = row.id.clone();
    // Its views' frame times (`EXT_VIEWS`'s meters), read again each
    // second while its page is open.
    let meters = try_use_context::<architect_workspace::Meters>();
    let mut tick = use_signal(|| 0_u64);
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_secs(1)).await;
            let next = tick.peek().wrapping_add(1);
            tick.set(next);
        }
    });
    let _ = tick();
    let health = meters
        .as_ref()
        .map(|m| crate::views::health_lines(m, &row.id));
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 14px;",
            Card {
                CardHeader {
                    CardTitle { "{row.name}" }
                    CardDescription { "{row.description}" }
                }
                CardContent {
                    div { style: "display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 10px;",
                        Badge { variant: BadgeVariant::Secondary, "{row.kind}" }
                        Badge { variant: BadgeVariant::Outline, "{row.version}" }
                        Badge { variant: BadgeVariant::Outline, "{row.at.label()}" }
                        if row.loaded { Badge { "Running" } }
                        if row.disabled { Badge { variant: BadgeVariant::Destructive, "Off" } }
                        if row.not_here.is_some() { Badge { variant: BadgeVariant::Secondary, "Can't run here" } }
                        if let Some(p) = &pin { Badge { variant: BadgeVariant::Outline, "Pinned {p}" } }
                    }
                    KeyValueRow { label: "Id".to_owned(), value: row.id.clone() }
                    if let Some(runs) = &row.runs {
                        KeyValueRow { label: "Runs".to_owned(), value: runs.clone(), mono: false }
                    }
                    if let Some(why) = &row.not_here {
                        KeyValueRow {
                            label: "Here".to_owned(),
                            value: if row.at == Where::Available {
                                format!("{why}. It cannot be installed here.")
                            } else {
                                format!("{why}. It is kept, with its data; it runs where it can.")
                            },
                            mono: false,
                        }
                    }
                    if let Some(a) = &about {
                        KeyValueRow { label: "Where it runs".to_owned(), value: a.runner.clone(), mono: false }
                        if !a.location.is_empty() {
                            KeyValueRow { label: "From".to_owned(), value: a.location.clone() }
                        }
                    }
                }
                CardFooter {
                    div { style: "display: flex; flex-wrap: wrap; gap: 8px;",
                        Actions { browse: browse.clone(), row: row.clone(), installed: installed.clone(), about: about.clone() }
                    }
                }
            }
            if row.overrides_builtin {
                Alert {
                    AlertTitle { "Overrides the built-in copy" }
                    AlertDescription {
                        div { style: "display: flex; align-items: center; gap: 10px;",
                            span { style: "flex: 1;", "{row.runs.clone().unwrap_or_default()}" }
                            Button { variant: ButtonVariant::Outline, size: ButtonSize::Small,
                                on_click: {
                                    let (ext, id) = (ext.clone(), id.clone());
                                    move |_| {
                                        back_to_builtin(&ext, &id);
                                        say(s, true, "Back to the built-in copy.");
                                        bump(s);
                                    }
                                },
                                "Use the built-in"
                            }
                        }
                    }
                }
            }
            if let Some(a) = &about {
                Section { title: "What it adds",
                    if a.adds.is_empty() { Muted { text: "Nothing to the interface.".to_owned() } }
                    for line in a.adds.iter() { Line { key: "{line}", text: line.clone() } }
                    for h in a.hooks.iter() { Line { key: "{h}", text: format!("Hook {h}") } }
                }
                Section { title: "What it asks for",
                    if a.asks.is_empty() { Muted { text: "Nothing past its own actions, settings and data.".to_owned() } }
                    for ask in a.asks.iter() {
                        AskLine {
                            key: "{ask.kind}{ask.what}",
                            text: ask.says.clone(),
                            granted: row.at == Where::Builtin || row.at == Where::Dev || may.as_ref().is_some_and(|m| m.grows_from(&crate::manifest::Capabilities::default()).iter().any(|g| g == ask)),
                        }
                    }
                    if row.at == Where::Builtin || row.at == Where::Dev {
                        Muted { text: format!("{}: it runs with what it asks.", row.at.label()) }
                    }
                    for g in grants.iter().cloned() {
                        GrantLine { key: "{g.scope.describe()}", ext: ext.clone(), grant: g }
                    }
                }
            }
            if !versions.is_empty() {
                Section { title: "Versions",
                    for v in versions.iter() {
                        Line { key: "{v}", text: if installed.as_deref() == Some(v.as_str()) { format!("{v} (installed)") } else { v.clone() } }
                    }
                    if installed.is_some() {
                        PinPicker { ext: ext.clone(), id: id.clone(), versions: versions.clone(), pin: pin.clone() }
                    }
                }
            }
            if let Some(health) = health {
                Section { title: "Frame health",
                    if health.is_empty() { Muted { text: "No view of it measured yet.".to_owned() } }
                    for (i, line) in health.iter().enumerate() {
                        div { key: "{i}", style: "font-size: 12px; font-family: ui-monospace, monospace;", "{line}" }
                    }
                }
            }
            Section { title: "Recent log",
                if logs.is_empty() { Muted { text: "Nothing logged yet.".to_owned() } }
                for (i, l) in logs.iter().enumerate() {
                    div { key: "{i}", style: "font-family: ui-monospace, monospace; font-size: 11px; color: var(--muted-foreground);",
                        "{l.level.name()} {l.source}: {l.text}"
                    }
                }
            }
        }
    }
}

/// Back to the built-in copy: the installed copy removed, or the
/// development one's session ended.
fn back_to_builtin(ext: &crate::extensions::Extensions, id: &str) {
    let Some(chosen) = ext.chosen().into_iter().find(|c| c.runs.id == id) else {
        return;
    };
    match chosen.runs.origin {
        crate::catalog::Origin::Installed => {
            let _ = ext.uninstall(id);
        }
        crate::catalog::Origin::Dev => {
            if let Some(dir) = chosen.runs.package.dir() {
                let _ = ext.end_dev(dir);
            }
        }
        crate::catalog::Origin::Builtin => {}
    }
}

/// Install, update, turn off and on, remove.
#[component]
fn Actions(browse: Browse, row: Row, installed: Option<String>, about: Option<Preview>) -> Element {
    let s = state();
    let ext = browse.extensions().clone();
    let id = row.id.clone();
    let install = {
        let (ext, id) = (ext.clone(), id.clone());
        move |pin: bool| {
            match ext.install_with(&id, &VersionReq::STAR, None, "person", pin) {
                Ok(Installed::NeedsConsent { preview }) => {
                    let mut asking = s.asking;
                    asking.set(Some(Asking {
                        previews: vec![*preview],
                        purpose: For::Install { pin },
                    }));
                }
                Ok(done) => say(s, true, outcome(&done, &id)),
                Err(e) => say(s, false, e.to_string()),
            }
            bump(s);
        }
    };
    rsx! {
        if row.at == Where::Available && row.not_here.is_none() {
            Button { on_click: { let install = install.clone(); move |_| install(false) }, "Install" }
            Button { variant: ButtonVariant::Outline, on_click: move |_| install(true), "Install and pin" }
        }
        if installed.is_some() {
            Button { variant: ButtonVariant::Outline,
                on_click: {
                    let (ext, id) = (ext.clone(), id.clone());
                    move |_| {
                        let updated = ext.update(Some(&id));
                        if let Some(p) = updated.needs_consent.first().cloned() {
                            let mut asking = s.asking;
                            asking.set(Some(Asking { previews: vec![p], purpose: For::Update }));
                        } else if !updated.updated.is_empty() {
                            say(s, true, format!("Updated to {}", updated.updated[0]["to"].as_str().unwrap_or("")));
                        } else if !updated.unchanged.is_empty() {
                            say(s, true, "Already the newest its pin allows.");
                        } else if let Some(f) = updated.failed.first() {
                            say(s, false, f["why"].as_str().unwrap_or("").to_owned());
                        }
                        bump(s);
                    }
                },
                "Update"
            }
        }
        if row.at == Where::Installed && about.as_ref().is_some_and(|a| !a.needs_consent.is_empty()) {
            Button { variant: ButtonVariant::Outline,
                on_click: {
                    let about = about.clone();
                    move |_| {
                        if let Some(a) = about.clone() {
                            let mut asking = s.asking;
                            asking.set(Some(Asking { previews: vec![a], purpose: For::Grant }));
                        }
                    }
                },
                "Grant what it asks"
            }
        }
        if row.at != Where::Available {
            Button { variant: ButtonVariant::Outline,
                on_click: {
                    let (ext, id, off) = (ext.clone(), id.clone(), row.disabled);
                    move |_| {
                        let _ = if off { ext.enable(&id) } else { ext.disable(&id) };
                        say(s, true, if off { "Turned on." } else { "Turned off." });
                        bump(s);
                    }
                },
                if row.disabled { "Turn on" } else { "Turn off" }
            }
        }
        // Its tour (`[[tours]]`), while it runs: the app's tour layer plays it.
        if !row.disabled && ext.host().contributions().tours().iter().any(|t| t.owner == row.id) {
            Button { variant: ButtonVariant::Outline,
                on_click: {
                    let (ext, id) = (ext.clone(), id.clone());
                    move |_| {
                        if let Err(e) = ext.host().start_tour(&id, None) {
                            say(s, false, e);
                        }
                    }
                },
                "Take the tour"
            }
        }
        if row.at == Where::Installed {
            Button { variant: ButtonVariant::Destructive,
                on_click: {
                    let (ext, id) = (ext.clone(), id.clone());
                    move |_| {
                        match ext.uninstall(&id) {
                            Ok(_) => say(s, true, "Removed."),
                            Err(e) => say(s, false, e.to_string()),
                        }
                        bump(s);
                    }
                },
                "Remove"
            }
        }
    }
}

#[component]
fn PinPicker(
    ext: crate::extensions::Extensions,
    id: String,
    versions: Vec<String>,
    pin: Option<String>,
) -> Element {
    let s = state();
    let value = use_signal(|| pin.clone().unwrap_or_default());
    rsx! {
        div { style: "display: flex; align-items: center; gap: 8px; margin-top: 6px;",
            span { style: "font-size: 12px; color: var(--muted-foreground);", "Pin to" }
            NativeSelect { value,
                on_change: move |e: FormEvent| {
                    let v = e.value();
                    let done = if v.is_empty() { ext.pin(&id, None, true) } else { ext.pin(&id, Some(&v), false) };
                    match done {
                        Ok(Some(p)) => say(s, true, format!("Pinned to {p}.")),
                        Ok(None) => say(s, true, "Not pinned: it updates."),
                        Err(e) => say(s, false, e.to_string()),
                    }
                    bump(s);
                },
                NativeSelectOption { value: String::new(), "Not pinned" }
                for v in versions.iter() {
                    NativeSelectOption { key: "{v}", value: format!("={v}"), "{v}" }
                }
            }
        }
    }
}

#[component]
fn GrantLine(ext: crate::extensions::Extensions, grant: crate::consent::Grant) -> Element {
    let s = state();
    let where_ = match &grant.scope {
        Scope::Device => "on this device".to_owned(),
        Scope::Org(o) => format!("in organization {o}"),
    };
    rsx! {
        div { style: "display: flex; align-items: center; gap: 8px; padding-top: 6px;",
            span { style: "flex: 1; font-size: 12px; color: var(--muted-foreground);",
                "Granted {where_} at {grant.version} by {grant.by}"
            }
            Button { variant: ButtonVariant::Outline, size: ButtonSize::Small,
                on_click: move |_| {
                    if let Some(g) = ext.host().grants() {
                        let n = g.revoke(&grant.id, Some(&grant.scope));
                        say(s, true, format!("Revoked {n} grant(s): it may no longer reach what it was granted."));
                    }
                    bump(s);
                },
                "Revoke"
            }
        }
    }
}

/// Settings → Extensions' registries (#493): the store's and the ones
/// the person added, each with its publishers' keys; a git repository or
/// a folder added with a key; development installs and a folder loaded
/// as a development copy. Shown where the app keeps registries.
#[component]
fn RegistriesPanel(browse: Browse) -> Element {
    let s = state();
    let _ = (s.rev)();
    let ext = browse.extensions().clone();
    let Some(registries) = ext.registries() else {
        return rsx! {};
    };
    let mut open = use_signal(|| false);
    let name = use_signal(String::new);
    let place = use_signal(String::new);
    let key_name = use_signal(String::new);
    let key = use_signal(String::new);
    let dev_folder = use_signal(String::new);
    let dev_on = use_signal(|| registries.dev_installs());
    let listed = registries.list();
    let forced = registries.dev_installs() && !registries.dev_setting();
    let add = {
        let (ext, registries) = (ext.clone(), registries.clone());
        move |_| {
            let (n, p, kn, k) = (
                name.peek().trim().to_owned(),
                place.peek().trim().to_owned(),
                key_name.peek().trim().to_owned(),
                key.peek().trim().to_owned(),
            );
            let is_git = p.contains("://") || p.starts_with("git@");
            let mut r = if is_git {
                crate::registries::Registry::git(n.clone(), p)
            } else {
                crate::registries::Registry::folder(n.clone(), p)
            };
            if !k.is_empty() {
                r = r.trusting(if kn.is_empty() { n.clone() } else { kn }, k);
            }
            if let Err(e) = registries.add(r) {
                say(s, false, e);
            } else {
                say(s, true, format!("Added {n}; fetching its index…"));
                bump(s);
                let ext = ext.clone();
                let (tx, rx) = futures::channel::oneshot::channel();
                let _ = std::thread::Builder::new()
                    .name("ext-registry-add".to_owned())
                    .spawn(move || drop(tx.send(ext.refresh_registries(Some(&n)))));
                spawn(async move {
                    let failed = rx.await.unwrap_or_default();
                    match failed.first() {
                        Some((n, e)) => say(s, false, format!("{n}: {e}")),
                        None => say(s, true, "Its index is fetched: its extensions are listed."),
                    }
                    bump(s);
                });
            }
        }
    };
    let load_dev = {
        let ext = ext.clone();
        move |_| match ext.dev_install(dev_folder.peek().trim()) {
            Ok(v) => {
                say(
                    s,
                    v["loaded"] == true,
                    format!(
                        "{}: {}",
                        v["id"].as_str().unwrap_or("?"),
                        if v["loaded"] == true {
                            "loaded as a development copy"
                        } else {
                            "did not load (see its log)"
                        }
                    ),
                );
                bump(s);
            }
            Err(e) => say(s, false, e),
        }
    };
    let set_dev = {
        let registries = registries.clone();
        move |on: bool| {
            if let Err(e) = registries.set_dev_installs(on) {
                say(s, false, e);
            }
            bump(s);
        }
    };
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 8px;",
            Button {
                variant: ButtonVariant::Ghost,
                on_click: move |_| { let now = *open.peek(); open.set(!now); },
                if open() { "▾ Registries and development installs" } else { "▸ Registries and development installs" }
            }
            if open() {
                Section { title: "Registries".to_owned(),
                    for l in listed {
                        div { key: "{l.registry.name}", style: "display: flex; gap: 8px; align-items: center;",
                            div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column;",
                                Line { text: format!("{}{}", l.registry.name, if l.builtin { " · the app's" } else { "" }) }
                                Muted { text: l.registry.git.clone().or_else(|| l.registry.folder.clone()).unwrap_or_default() }
                                if !l.registry.keys.is_empty() {
                                    Muted { text: format!("Trusts {}", l.registry.keys.iter().map(|k| k.name.as_str()).collect::<Vec<_>>().join(", ")) }
                                }
                            }
                            if !l.builtin {
                                Button {
                                    variant: ButtonVariant::Ghost,
                                    on_click: {
                                        let (registries, n) = (registries.clone(), l.registry.name.clone());
                                        move |_| {
                                            match registries.remove(&n) {
                                                Ok(_) => say(s, true, format!("Removed {n}")),
                                                Err(e) => say(s, false, e),
                                            }
                                            bump(s);
                                        }
                                    },
                                    "Remove"
                                }
                            }
                        }
                    }
                    Input { value: name, placeholder: "Name (Studio)".to_owned() }
                    Input { value: place, placeholder: "Git URL or folder".to_owned() }
                    Input { value: key_name, placeholder: "Publisher (optional)".to_owned() }
                    Input { value: key, placeholder: "Publisher's key: the base64 line of its minisign .pub".to_owned() }
                    Button { on_click: add, "Add registry" }
                }
                Section { title: "Development installs".to_owned(),
                    div { style: "display: flex; gap: 8px; align-items: center;",
                        Switch { checked: dev_on, on_change: set_dev }
                        Line { text: if forced { "On (a debug build)".to_owned() } else { "Load unsigned folders as development copies".to_owned() } }
                    }
                    Input { value: dev_folder, placeholder: "A package folder (its extension.toml in it)".to_owned() }
                    Button { on_click: load_dev, disabled: !registries.dev_installs(), "Load for development" }
                    for d in registries.dev_folders() {
                        Muted { key: "{d.display()}", text: d.display().to_string() }
                    }
                }
            }
        }
    }
}

#[component]
fn Section(title: String, children: Element) -> Element {
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; border: 1px solid var(--border); border-radius: 10px; background: var(--card);",
            SectionHeader { label: title }
            {children}
        }
    }
}

#[component]
fn Line(text: String) -> Element {
    rsx! { div { style: "font-size: 13px; color: var(--foreground);", "{text}" } }
}

#[component]
fn Muted(text: String) -> Element {
    rsx! { div { style: "font-size: 12px; color: var(--muted-foreground);", "{text}" } }
}

#[component]
fn AskLine(text: String, granted: bool) -> Element {
    rsx! {
        div { style: "display: flex; align-items: center; gap: 8px; font-size: 13px;",
            span { style: "flex: 1;", "{text}" }
            if granted {
                Badge { variant: BadgeVariant::Secondary, "Granted" }
            } else {
                Badge { variant: BadgeVariant::Outline, "Not granted" }
            }
        }
    }
}

/// What an install, an update or a grant will let it do, and where the
/// person says yes: this device, or the organization.
#[component]
pub fn ConsentPrompt(
    previews: Vec<Preview>,
    org: Option<String>,
    action: String,
    on_answer: EventHandler<Option<Scope>>,
) -> Element {
    let mut scope = use_signal(|| "device".to_owned());
    let names: Vec<String> = previews
        .iter()
        .map(|p| format!("{} {}", p.name, p.version))
        .collect();
    let mut options = vec![("device".to_owned(), "This device".to_owned())];
    if let Some(o) = &org {
        options.push((format!("org:{o}"), format!("Organization {o}")));
    }
    // In the page's flow, above what it is about: an overlay inside a
    // scrolling section is clipped by it (Blitz).
    rsx! {
        div { style: "display: flex; justify-content: center;",
            div { style: "width: 560px; max-width: 100%; border: 2px solid var(--primary); border-radius: 14px;",
                Card {
                    CardHeader {
                        CardTitle { "{action} {names.join(\", \")}?" }
                        CardDescription { "It will be able to:" }
                    }
                    CardContent {
                        div { style: "display: flex; flex-direction: column; gap: 10px;",
                            for p in previews.iter() {
                                div { key: "{p.id}", style: "display: flex; flex-direction: column; gap: 4px;",
                                    if previews.len() > 1 {
                                        div { style: "font-weight: 600; font-size: 13px;", "{p.name}" }
                                    }
                                    for ask in p.needs_consent.iter().chain(p.asks.iter().filter(|a| p.needs_consent.is_empty() && !a.says.is_empty())) {
                                        div { key: "{ask.kind}{ask.what}", style: "font-size: 13px;", "• {ask.says}" }
                                    }
                                    div { style: "font-size: 12px; color: var(--muted-foreground);",
                                        "Runs as {p.runner}. Kind {p.kind}, from {p.location}."
                                    }
                                    if !p.hooks.is_empty() {
                                        div { style: "font-size: 12px; color: var(--muted-foreground);", "Runs on: {p.hooks.join(\", \")}" }
                                    }
                                }
                            }
                            div { style: "display: flex; align-items: center; gap: 10px;",
                                span { style: "font-size: 12px; color: var(--muted-foreground);", "Allow on" }
                                SegmentedControl { value: scope(), options, on_change: move |v: String| scope.set(v) }
                            }
                        }
                    }
                    CardFooter {
                        div { style: "display: flex; justify-content: flex-end; gap: 8px; width: 100%;",
                            Button { variant: ButtonVariant::Outline, on_click: move |_| on_answer.call(None), "Cancel" }
                            Button {
                                on_click: move |_| {
                                    let chosen = scope();
                                    let s = chosen
                                        .strip_prefix("org:")
                                        .map_or(Scope::Device, |o| Scope::Org(o.to_owned()));
                                    on_answer.call(Some(s));
                                },
                                "{action}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A project's (or a vault's) missing extensions: "This project uses …",
/// each one, "Install all" through one consent prompt for all of them,
/// and what cannot be had here, each with why (kept, never dropped).
/// `what` names what is opening ("vault"; "project" when not given);
/// `on_done` hears what an install did (each one's outcome, `; `-joined),
/// to plan again.
#[component]
pub fn MissingExtensions(
    browse: Browse,
    plan: Plan,
    #[props(default)] what: Option<String>,
    #[props(default)] on_done: Option<EventHandler<String>>,
) -> Element {
    let mut asking = use_signal(|| None::<Vec<Preview>>);
    let mut note = use_signal(|| None::<String>);
    if plan.install.is_empty() && plan.offer.is_empty() && plan.missing.is_empty() {
        return rsx! {};
    }
    let what = what.unwrap_or_else(|| "project".to_owned());
    let prompt = plan.prompt_for(&what).unwrap_or_else(|| {
        if plan.install.is_empty() && plan.offer.is_empty() {
            format!("Some extensions this {what} uses cannot run here")
        } else {
            format!("Extensions this {what} works with")
        }
    });
    let can_install = !plan.install.is_empty();
    let org = browse.extensions().host().grants().and_then(|g| g.org());
    let ext = browse.extensions().clone();
    let listings = plan.install.clone();
    let mut rows: Vec<(String, String, &'static str, String)> = Vec::new();
    for l in &plan.install {
        rows.push((
            l.id.clone(),
            l.name.clone(),
            "Required",
            format!("{} {}", l.kind, l.version),
        ));
    }
    for l in &plan.offer {
        rows.push((
            l.id.clone(),
            l.name.clone(),
            "Optional",
            format!("{} {}", l.kind, l.version),
        ));
    }
    for (r, why) in &plan.missing {
        let name = if r.name.is_empty() {
            r.id.clone()
        } else {
            r.name.clone()
        };
        let tag = if r.required {
            "Can't install"
        } else {
            "Not found"
        };
        rows.push((r.id.clone(), name, tag, format!("{why}; kept as it is")));
    }
    // Inline styles over the theme's tokens, and no `style` node: one
    // here made Blitz panic (`layout_parent … does not point at its layout
    // container`) beside the workspace. The consent prompt's classes are
    // the app's to load (`document::Style` of `architect_ui::UTILITIES_CSS`).
    rsx! {
        div {
            role: "alert",
            style: "display: flex; flex-direction: column; gap: 6px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 10px; background: var(--card); color: var(--card-foreground); font-size: 13px;",
            div { style: "font-weight: 600;", "{prompt}" }
            for (id, name, tag, detail) in rows {
                div { key: "{id}", style: "display: flex; flex-wrap: wrap; align-items: flex-start; gap: 4px 8px;",
                    span { style: "font-weight: 500;", "{name}" }
                    span { style: "font-size: 11px; padding: 1px 6px; border-radius: 6px; border: 1px solid var(--border); color: var(--muted-foreground);", "{tag}" }
                    span { style: "color: var(--muted-foreground);", "{detail}" }
                }
            }
            if let Some(n) = note() {
                div { style: "font-size: 12px; color: var(--muted-foreground);", "{n}" }
            }
            if can_install {
                div { style: "display: flex; gap: 8px; padding-top: 2px;",
                    button {
                        style: "padding: 5px 12px; border-radius: 8px; border: none; background: var(--primary); color: var(--primary-foreground); font-size: 13px; font-weight: 600; cursor: pointer;",
                        onclick: move |_| {
                            let previews: Vec<Preview> = listings
                                .iter()
                                .filter_map(|l| {
                                    let exact = VersionReq::parse(&format!("={}", l.version)).ok()?;
                                    ext.preview(&l.id, &exact).ok()
                                })
                                .collect();
                            asking.set(Some(previews));
                        },
                        "Install all"
                    }
                }
            }
        }
        if let Some(previews) = asking() {
            ConsentPrompt {
                previews: previews.clone(),
                org,
                action: "Install".to_owned(),
                on_answer: move |scope: Option<Scope>| {
                    asking.set(None);
                    let Some(scope) = scope else { return };
                    let ext = browse.extensions();
                    let mut done = Vec::new();
                    for p in &previews {
                        let exact = VersionReq::parse(&format!("={}", p.version)).unwrap_or(VersionReq::STAR);
                        match ext.install_with(&p.id, &exact, Some(scope.clone()), "person", false) {
                            Ok(i) => done.push(outcome(&i, &p.name)),
                            Err(e) => done.push(format!("{}: {e}", p.name)),
                        }
                    }
                    let done = done.join("; ");
                    note.set(Some(done.clone()));
                    if let Some(on_done) = on_done {
                        on_done.call(done);
                    }
                },
            }
        }
    }
}
