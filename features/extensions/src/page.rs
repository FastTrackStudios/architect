//! The extensions page (#181, feature `page`): find, install, update,
//! pin, turn off, remove — and see what each one adds, asks for and runs
//! as — in the app's theme, with architect-ui's components.
//!
//! - [`ExtensionsPage`]: search every registry (by name, kind and what it
//!   adds: `browse`), a list, and the page of the one picked — what it
//!   adds, the capabilities it asks for (granted or not, each grant
//!   revocable), its versions and pin, where it runs, its recent log, and
//!   "built in, overridden by 1.4.0-dev" with the way back.
//! - [`ConsentPrompt`]: what an install (or an update that asks for more)
//!   will let it do, granted on this device or in the organization.
//! - [`MissingExtensions`]: a project's "This project uses …" with
//!   "Install all", the same consent prompt for all of them.
//! - [`use_extensions_page`]: the page as a section of the settings page
//!   (`architect_settings::use_section_page`) and ⌘O's `ext:` opener; an
//!   app with a workspace puts [`ExtensionsView`] in a view kind of its
//!   own as well.
//!
//! Every colour is the theme's (`var(--…)`, architect-ui's classes over
//! the store's CSS variables); nothing here names one.

use architect_ui::prelude::*;
use dioxus::prelude::*;
use semver::VersionReq;

use crate::browse::{Browse, Row, Where};
use crate::consent::Scope;
use crate::manage::{Installed, Preview};
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
    selected: Signal<Option<String>>,
    /// Bumped after every change: what reads the extensions reads again.
    rev: Signal<u64>,
    asking: Signal<Option<Asking>>,
    note: Signal<Option<(bool, String)>>,
}

fn state() -> State {
    use_context::<State>()
}

fn bump(s: State) {
    let mut rev = s.rev;
    rev += 1;
}

fn say(s: State, ok: bool, text: impl Into<String>) {
    let mut note = s.note;
    note.set(Some((ok, text.into())));
}

/// The page in context: a settings section and ⌘O's `ext:` (call it
/// under the settings provider and the commands provider).
pub fn use_extensions_page(browse: Browse) {
    use_context_provider(|| browse.clone());
    let s = use_context_provider(|| State {
        query: Signal::new(String::new()),
        selected: Signal::new(None),
        rev: Signal::new(0),
        asking: Signal::new(None),
        note: Signal::new(None),
    });
    architect_settings::use_section_page(
        || architect_settings::Section::new(SECTION, "Extensions").icon("puzzle"),
        ExtensionsSection,
    );
    let settings = architect_settings::try_use_settings();
    let rows = browse.clone();
    architect_commands::use_opener(move || {
        architect_commands::Opener::new(
            "ext",
            "Extensions",
            move |query| {
                rows.rows(query)
                    .into_iter()
                    .take(30)
                    .map(|r| {
                        architect_commands::OpenItem::new("ext", r.id.clone(), r.name.clone())
                            .detail(format!("{} · {} · {}", r.at.label(), r.kind, r.version))
                    })
                    .collect()
            },
            move |item, _| {
                let mut selected = s.selected;
                selected.set(Some(item.id.clone()));
                if let Some(h) = settings {
                    h.show(Some(SECTION));
                }
            },
        )
    });
}

/// The page as the settings page's section (what
/// [`use_extensions_page`] registers).
#[component]
pub fn ExtensionsSection() -> Element {
    let browse = use_context::<Browse>();
    rsx! { ExtensionsPage { browse } }
}

/// The page in a workspace pane.
#[component]
pub fn ExtensionsView() -> Element {
    let browse = use_context::<Browse>();
    rsx! {
        div { style: "height: 100%; overflow-y: scroll; padding: 20px 24px; background: var(--background); color: var(--foreground);",
            ExtensionsPage { browse }
        }
    }
}

/// The extensions page.
#[component]
pub fn ExtensionsPage(browse: Browse) -> Element {
    let s = state();
    let _ = (s.rev)();
    let query = (s.query)();
    let rows = browse.rows(&query);
    let selected = (s.selected)().or_else(|| rows.first().map(|r| r.id.clone()));
    let picked = selected
        .as_ref()
        .and_then(|id| rows.iter().find(|r| &r.id == id).cloned());
    let org = browse.extensions().host().grants().and_then(|g| g.org());
    rsx! {
        style { {architect_ui::UTILITIES_CSS} }
        div { style: "display: flex; flex-direction: column; gap: 14px; font-family: inherit; color: var(--foreground);",
            if let Some((ok, text)) = (s.note)() {
                Alert { variant: if ok { AlertVariant::Default } else { AlertVariant::Destructive },
                    AlertDescription { "{text}" }
                }
            }
            div { style: "display: flex; gap: 16px; align-items: flex-start;",
                // The list.
                div { style: "flex: 0 0 300px; display: flex; flex-direction: column; gap: 8px;",
                    Input { value: s.query, placeholder: "Search: name, kind:luau, adds:tab".to_owned() }
                    div { style: "font-size: 12px; color: var(--muted-foreground);",
                        "{rows.len()} extension(s)"
                    }
                    div { style: "display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: 10px; overflow: hidden;",
                        if rows.is_empty() {
                            EmptyState { message: "Nothing matches".to_owned() }
                        }
                        for r in rows.iter().cloned() {
                            RowItem { key: "{r.id}", on: selected.as_deref() == Some(r.id.as_str()), row: r }
                        }
                    }
                }
                // The one picked.
                div { style: "flex: 1; min-width: 0;",
                    if let Some(row) = picked {
                        Detail { key: "{row.id}", browse: browse.clone(), row }
                    } else {
                        EmptyState { message: "Pick an extension".to_owned() }
                    }
                }
            }
            if let Some(asking) = (s.asking)() {
                ConsentPrompt {
                    previews: asking.previews.clone(),
                    org,
                    action: match asking.purpose { For::Install { .. } => "Install", For::Update => "Update", For::Grant => "Grant" }.to_owned(),
                    on_answer: move |scope: Option<Scope>| answer(&browse, s, &asking, scope),
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

#[component]
fn RowItem(on: bool, row: Row) -> Element {
    let s = state();
    let ground = if on { "var(--accent)" } else { "transparent" };
    let id = row.id.clone();
    rsx! {
        button {
            style: "display: flex; flex-direction: column; align-items: stretch; gap: 4px; padding: 10px 12px; border: none; \
                    border-bottom: 1px solid var(--border); background: {ground}; color: var(--foreground); text-align: left; cursor: pointer;",
            onclick: move |_| {
                let mut selected = s.selected;
                selected.set(Some(id.clone()));
            },
            div { style: "display: flex; align-items: center; gap: 6px;",
                span { style: "font-size: 13px; font-weight: 600; flex: 1;", "{row.name}" }
                Badge { variant: if row.at == Where::Available { BadgeVariant::Outline } else { BadgeVariant::Secondary }, "{row.at.label()}" }
            }
            div { style: "display: flex; gap: 6px; font-size: 11px; color: var(--muted-foreground);",
                span { "{row.kind}" }
                span { "{row.version}" }
                if row.overrides_builtin { span { "· overrides built-in" } }
                if row.disabled { span { "· off" } }
                if let Some(n) = &row.newer { span { "· {n} available" } }
            }
        }
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
    let about: Option<Preview> = preview.read().clone().or_else(|| {
        let chosen = ext.chosen().into_iter().find(|c| c.runs.id == row.id)?;
        let m = chosen.runs.package.manifest.clone();
        let listing = crate::index::Listing {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            kind: m.kind.name().to_owned(),
            description: m.description.clone(),
            category: String::new(),
            location: crate::index::Location::Path(String::new()),
        };
        Some(ext.preview_of(&m, &listing, row.registry.as_deref().unwrap_or("")))
    });
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
    rsx! {
        div {
            style: "position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 1000; display: flex; align-items: center; \
                    justify-content: center; background: color-mix(in srgb, var(--background) 70%, transparent);",
            div { style: "width: 520px; max-width: 92%;",
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
