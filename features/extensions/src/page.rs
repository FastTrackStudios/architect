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
//! - [`use_manager_page`]: the manager as a section of the settings page
//!   (`architect_settings::use_section_page`) and ⌘O's `ext:` opener; an
//!   app with a workspace puts [`ManagerView`] in a view kind of its own
//!   as well (it is the `manager` extension's view too).
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

/// How many rows the list shows at once.
const SHOWN: usize = 12;

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

/// The manager in context: a settings section ("Extensions") and ⌘O's
/// `ext:` over every kind (call it under the settings provider and the
/// commands provider). `browse` is the extensions' catalog, for the
/// extension kind's own page.
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
    architect_settings::use_section_page(
        || architect_settings::Section::new(SECTION, "Extensions").icon("puzzle"),
        ManagerSection,
    );
    let settings = architect_settings::try_use_settings();
    let items = manager.clone();
    architect_commands::use_opener(move || {
        architect_commands::Opener::new(
            "ext",
            "Extensions",
            move |query| {
                items
                    .list(None, query)
                    .into_iter()
                    .take(40)
                    .map(|i| {
                        architect_commands::OpenItem::new(
                            "ext",
                            format!("{}\u{1f}{}", i.kind, i.id),
                            i.name.clone(),
                        )
                        .detail(format!("{} · {}", i.kind, i.source))
                    })
                    .collect()
            },
            move |item, _| {
                if let Some((kind, id)) = item.id.split_once('\u{1f}') {
                    let mut selected = s.selected;
                    selected.set(Some((kind.to_owned(), id.to_owned())));
                }
                if let Some(h) = settings {
                    h.show(Some(SECTION));
                }
            },
        )
    });
}

/// The manager as the settings page's section.
#[component]
pub fn ManagerSection() -> Element {
    let manager = use_context::<Manager>();
    rsx! { ManagerPage { manager } }
}

/// The manager in a workspace pane (and the `manager` extension's view).
#[component]
pub fn ManagerView() -> Element {
    let manager = use_context::<Manager>();
    rsx! {
        div { style: "height: 100%; overflow-y: scroll; padding: 20px 24px; background: var(--background); color: var(--foreground);",
            ManagerPage { manager }
        }
    }
}

/// The manager's page: kinds, search across them, the list, and the
/// page of the item picked.
#[component]
pub fn ManagerPage(manager: Manager) -> Element {
    let s = state();
    let _ = (s.rev)();
    let query = (s.query)();
    let kind = (s.kind)();
    let items = manager.list(kind.as_deref(), &query);
    let selected = (s.selected)()
        .filter(|(k, i)| items.iter().any(|x| &x.kind == k && &x.id == i))
        .or_else(|| items.first().map(|i| (i.kind.clone(), i.id.clone())));
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
    rsx! {
        style { {architect_ui::UTILITIES_CSS} }
        div { style: "display: flex; flex-direction: column; gap: 14px; font-family: inherit; color: var(--foreground);",
            if let (Some(asking), Some(browse)) = ((s.asking)(), browse.clone()) {
                ConsentPrompt {
                    previews: asking.previews.clone(),
                    org,
                    action: match asking.purpose { For::Install { .. } => "Install", For::Update => "Update", For::Grant => "Grant" }.to_owned(),
                    on_answer: move |scope: Option<Scope>| answer(&browse, s, &asking, scope),
                }
            }
            if let Some((ok, text)) = (s.note)() {
                Alert { variant: if ok { AlertVariant::Default } else { AlertVariant::Destructive },
                    AlertDescription { "{text}" }
                }
            }
            SegmentedControl {
                value: kind.clone().unwrap_or_default(),
                options: kinds,
                on_change: move |v: String| {
                    let mut k = s.kind;
                    k.set((!v.is_empty()).then_some(v));
                },
            }
            // Side by side when there is room; the page under the list in
            // a narrow pane.
            div { style: "display: flex; flex-wrap: wrap; gap: 16px; align-items: flex-start;",
                div { style: "flex: 1 1 260px; max-width: 360px; display: flex; flex-direction: column; gap: 8px;",
                    Input { value: s.query, placeholder: "Search every kind".to_owned() }
                    div { style: "font-size: 12px; color: var(--muted-foreground);",
                        "{items.len()} item(s) · by name, kind, source"
                    }
                    // No overflow of its own: a scroller inside the pane's
                    // scroller loses its clicks (Blitz). The first rows, and
                    // the search to narrow them.
                    div { style: "display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: 10px;",
                        if items.is_empty() {
                            EmptyState { message: "Nothing matches".to_owned() }
                        }
                        for i in items.iter().take(SHOWN).cloned() {
                            ItemRow {
                                key: "{i.kind}/{i.id}",
                                on: selected.as_ref().is_some_and(|(k, id)| k == &i.kind && id == &i.id),
                                item: i,
                            }
                        }
                    }
                    if items.len() > SHOWN {
                        Muted { text: format!("{} more: search or pick a kind to narrow them", items.len() - SHOWN) }
                    }
                }
                div { style: "flex: 999 1 380px; min-width: 0;",
                    match (selected, browse) {
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

#[component]
fn ItemRow(on: bool, item: Item) -> Element {
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
        detail.push_str(&format!(" · {} problem(s)", item.problems.len()));
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
                        if let Some(p) = &pin { Badge { variant: BadgeVariant::Outline, "Pinned {p}" } }
                    }
                    KeyValueRow { label: "Id".to_owned(), value: row.id.clone() }
                    if let Some(runs) = &row.runs {
                        KeyValueRow { label: "Runs".to_owned(), value: runs.clone(), mono: false }
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
        if row.at == Where::Available {
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

/// A project's missing extensions: "This project uses …", each one, and
/// "Install all" through the same consent prompt.
#[component]
pub fn MissingExtensions(browse: Browse, plan: Plan) -> Element {
    let mut asking = use_signal(|| None::<Vec<Preview>>);
    let mut note = use_signal(|| None::<String>);
    let Some(prompt) = plan.prompt() else {
        return rsx! {};
    };
    let org = browse.extensions().host().grants().and_then(|g| g.org());
    let ext = browse.extensions().clone();
    let listings = plan.install.clone();
    rsx! {
        style { {architect_ui::UTILITIES_CSS} }
        Alert {
            AlertTitle { "{prompt}" }
            AlertDescription {
                div { style: "display: flex; flex-direction: column; gap: 8px;",
                    for l in plan.install.iter() {
                        ListRow { key: "{l.id}", label: l.name.clone(), detail: format!("{} · {}", l.kind, l.version), tag: "Required".to_owned() }
                    }
                    for l in plan.offer.iter() {
                        ListRow { key: "{l.id}", label: l.name.clone(), detail: format!("{} · {}", l.kind, l.version), tag: "Optional".to_owned() }
                    }
                    if let Some(n) = note() { div { style: "font-size: 12px;", "{n}" } }
                    div {
                        Button {
                            on_click: move |_| {
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
                    note.set(Some(done.join("; ")));
                },
            }
        }
    }
}
