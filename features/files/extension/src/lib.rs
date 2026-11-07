//! Files: the org's projects and asset libraries in the app (#347).
//!
//! Everything comes from the device's files agent through `FILES_*`
//! (architect-files' `agent`; the app registers it over the agent's HTTP
//! face). The navigator provider (`nav`) lists each org's projects and
//! libraries; a project's page pins and unpins it, browses its folders,
//! shows a file's versions and the project's checkpoints and puts either
//! back; an asset's page shows its properties, its links and a waveform
//! per stem; every file's copies, where they are and how sure we are of
//! each (`copies`); an org's storage health, and search over the registry,
//! offline. A linked view is given no pane state, so `FILES_OPEN` sets
//! what the page shows before it opens it.

pub mod copies;
pub mod nav;

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use architect_extensions::hostcall::HostRequest;
use architect_extensions::linked::{Builder, Extension, Handle};
use dioxus::prelude::*;
use serde_json::{Value, json};

pub const ID: &str = "files";

static HANDLE: Mutex<Option<Handle>> = Mutex::new(None);
static CURRENT: Mutex<Option<Target>> = Mutex::new(None);
static REVISION: AtomicU64 = AtomicU64::new(0);

/// What a page shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Target {
    /// The root's place.
    pub root: String,
    /// An asset's name (the asset page), else the root's page.
    pub asset: Option<String>,
    /// The folder shown, root-relative.
    pub folder: String,
    /// A file to show the versions of.
    pub file: Option<String>,
}

fn current() -> Option<Target> {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn set_current(target: Target) {
    *CURRENT.lock().unwrap_or_else(PoisonError::into_inner) = Some(target);
    REVISION.fetch_add(1, Ordering::Relaxed);
}

/// Run a host action.
///
/// # Errors
///
/// The extension is not running, or the action failed.
pub async fn run(action: &str, args: Value) -> Result<Value, String> {
    let handle = HANDLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or("the files extension is not running")?;
    handle
        .host(HostRequest::Run {
            action: action.to_owned(),
            args,
            ticket: None,
        })
        .await
        .map_err(|e| e.to_string())
}

/// An id field: a string, or the `["…"]` an id newtype encodes as.
#[must_use]
pub fn id(v: &Value) -> String {
    match v.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .first()
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// `1.6 GB`.
#[must_use]
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut unit = 0_usize;
    let mut scale = 1_u64;
    while unit < 4 && bytes.checked_div(scale).unwrap_or(0) >= 1000 {
        scale = scale.saturating_mul(1000);
        unit = unit.saturating_add(1);
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    let tenths = bytes
        .checked_div(scale.checked_div(10).unwrap_or(1))
        .unwrap_or(0);
    format!(
        "{}.{} {}",
        tenths.checked_div(10).unwrap_or(0),
        tenths.checked_rem(10).unwrap_or(0),
        UNITS.get(unit).unwrap_or(&"TB")
    )
}

/// A root's one-line state: `pinned · 21 files · 1.6 GB`.
#[must_use]
pub fn summary(root: &Value) -> String {
    let pinned = root.get("pinned").and_then(Value::as_bool).unwrap_or(false);
    let held = !pinned && root.get("path").is_some_and(|p| !p.is_null());
    let state = if pinned {
        if root.get("synced").and_then(Value::as_bool).unwrap_or(false) {
            "pinned, synced"
        } else {
            "pinned, syncing"
        }
    } else if held {
        "held here"
    } else {
        "online"
    };
    let files = root.get("files").and_then(Value::as_u64).unwrap_or(0);
    let bytes = root.get("bytes").and_then(Value::as_u64).unwrap_or(0);
    format!("{state} · {files} files · {}", size(bytes))
}

/// `[min, max]` pairs reduced to `bars` heights, 0 to 1.
#[must_use]
pub fn bars(peaks: &Value, bars: usize) -> Vec<f32> {
    let data: Vec<i64> = peaks
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .collect();
    let pairs = data.len().checked_div(2).unwrap_or(0);
    if pairs == 0 || bars == 0 {
        return Vec::new();
    }
    let per = pairs.div_ceil(bars).max(1);
    let mut out = Vec::with_capacity(bars);
    for chunk in data.chunks(per.saturating_mul(2)) {
        let peak = chunk.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
        let peak = u16::try_from(peak.min(32767)).unwrap_or(u16::MAX);
        out.push((f32::from(peak) / 32767.0).min(1.0));
    }
    out
}

async fn open(args: Value) -> Result<Value, String> {
    let mut root = text(&args, "root").to_owned();
    let tool = matches!(
        args.get("view").and_then(Value::as_str),
        Some("health" | "find")
    );
    if root.is_empty() && !tool {
        let roots = run("FILES_ROOTS", json!({})).await?;
        root = roots
            .as_array()
            .and_then(|r| r.first())
            .map(|r| text(r, "place").to_owned())
            .ok_or("this device knows no roots yet")?;
    }
    let asset = args.get("asset").and_then(Value::as_str).map(str::to_owned);
    let path = text(&args, "path").to_owned();
    let (folder, file) = match path.rsplit_once('/') {
        _ if path.is_empty() => (String::new(), None),
        Some((dir, _)) if !args.get("dir").and_then(Value::as_bool).unwrap_or(false) => {
            (dir.to_owned(), Some(path.clone()))
        }
        _ if args.get("dir").and_then(Value::as_bool).unwrap_or(false) => (path.clone(), None),
        _ => (String::new(), Some(path.clone())),
    };
    let view = match args.get("view").and_then(Value::as_str) {
        Some("health") => "health",
        Some("find") => "find",
        _ if asset.is_some() => "asset",
        _ => "root",
    };
    let file = if view == "find" {
        args.get("query").and_then(Value::as_str).map(str::to_owned)
    } else {
        file
    };
    set_current(Target {
        root,
        asset,
        folder,
        file,
    });
    run("EXT_VIEW_OPEN", json!({ "extension": ID, "view": view })).await
}

/// The extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilesExtension;

impl Extension for FilesExtension {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        *HANDLE.lock().unwrap_or_else(PoisonError::into_inner) = Some(ext.handle());
        ext.navigator(nav::ACTIONS, nav::FilesNav);
        ext.view("root", RootView)
            .view("asset", AssetView)
            .view("drive", DriveView)
            .view("health", copies::HealthView)
            .view("find", copies::FindView)
            .action("FILES_OPEN", open)
            .action("FILES_DRIVE_OPEN", |_| async {
                run("EXT_VIEW_OPEN", json!({ "extension": ID, "view": "drive" })).await
            });
    }
}

/// Re-renders on a change it made, and every two seconds while it shows.
fn use_revision() -> u64 {
    let mut seen = use_signal(|| REVISION.load(Ordering::Relaxed));
    use_future(move || async move {
        let mut ticks = 0_u32;
        loop {
            futures_timer::Delay::new(Duration::from_millis(250)).await;
            ticks = ticks.wrapping_add(1);
            if ticks.is_multiple_of(8) {
                REVISION.fetch_add(1, Ordering::Relaxed);
            }
            let now = REVISION.load(Ordering::Relaxed);
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

fn bump() {
    REVISION.fetch_add(1, Ordering::Relaxed);
}

const PAGE: &str = "padding: 20px 24px; display: flex; flex-direction: column; gap: 14px; color: var(--foreground); font-size: 13px;";
const MUTED: &str = "color: var(--muted-foreground);";
const HEADING: &str = "font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted-foreground);";
const BUTTON: &str = "height: 28px; padding: 0 12px; border-radius: 7px; border: 1px solid var(--border); background: var(--secondary); color: var(--foreground); font-size: 12px; cursor: pointer;";
const ROW: &str = "display: flex; align-items: center; gap: 10px; padding: 5px 8px; border-radius: 6px; cursor: pointer;";
const CARD: &str = "border: 1px solid var(--border); border-radius: 10px; padding: 12px 14px; display: flex; flex-direction: column; gap: 6px;";

/// Read `action` again whenever the revision moves.
fn use_read(action: &'static str, args: &Value) -> Signal<Option<Result<Value, String>>> {
    let args = args.clone();
    let revision = use_revision();
    let mut got = use_signal(|| None::<Result<Value, String>>);
    use_effect(use_reactive!(|revision, args| {
        let _ = revision;
        spawn(async move {
            let now = run(action, args).await;
            if got.peek().as_ref() != Some(&now) {
                got.set(Some(now));
            }
        });
    }));
    got
}

#[allow(non_snake_case)]
fn RootView() -> Element {
    let Some(target) = current() else {
        return rsx! { div { style: "{PAGE} {MUTED}", "Open a project from the Files navigator (or FILES_OPEN {{root}})." } };
    };
    rsx! { RootPage { key: "{target.root}|{target.folder}|{target.file:?}", target } }
}

#[component]
fn RootPage(target: Target) -> Element {
    let root = target.root.clone();
    let roots = use_read("FILES_ROOTS", &json!({}));
    let entries = use_read(
        "FILES_BROWSE",
        &json!({ "root": root, "path": target.folder }),
    );
    let checkpoints = use_read("FILES_CHECKPOINTS", &json!({ "root": root }));
    let mut problem = use_signal(|| None::<String>);
    if let Some(Err(e)) = roots() {
        return rsx! { div { style: "{PAGE} color: var(--destructive);", "The files agent: {e}" } };
    }
    let status = roots().and_then(Result::ok).and_then(|all| {
        all.as_array()?
            .iter()
            .find(|r| text(r, "place") == root)
            .cloned()
    });
    let Some(status) = status else {
        return rsx! { div { style: "{PAGE} {MUTED}", "Reading {root}…" } };
    };
    let pinned = status
        .get("pinned")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let path = status
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let name = text(&status, "name").to_owned();
    let org = text(&status, "org").to_owned();
    let line = summary(&status);
    let crumbs: Vec<(String, String)> = {
        let mut acc = String::new();
        let mut out = vec![(String::new(), name.clone())];
        for seg in target.folder.split('/').filter(|s| !s.is_empty()) {
            acc = if acc.is_empty() {
                seg.to_owned()
            } else {
                format!("{acc}/{seg}")
            };
            out.push((acc.clone(), seg.to_owned()));
        }
        out
    };
    let toggle = {
        let root = root.clone();
        move |_| {
            let root = root.clone();
            spawn(async move {
                let done = if pinned {
                    run("FILES_UNPIN", json!({ "root": root })).await
                } else {
                    run("FILES_PIN", json!({ "root": root, "path": null })).await
                };
                problem.set(done.err());
                bump();
            });
        }
    };
    let free = {
        let root = root.clone();
        move |_| {
            let root = root.clone();
            spawn(async move {
                let done = run("FILES_EVICT", json!({ "root": root, "path": "" })).await;
                problem.set(match done {
                    Ok(n) => Some(format!("Freed {n} cached files on this device.")),
                    Err(e) => Some(e),
                });
                bump();
            });
        }
    };
    let checkpoint = {
        let root = root.clone();
        move |_| {
            let root = root.clone();
            spawn(async move {
                let done = run(
                    "FILES_CHECKPOINT",
                    json!({ "root": root, "name": "Checkpoint" }),
                )
                .await;
                problem.set(done.err());
                bump();
            });
        }
    };
    let entries = entries()
        .and_then(Result::ok)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let mut checkpoints = checkpoints()
        .and_then(Result::ok)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    checkpoints.reverse();
    rsx! {
        div { style: "{PAGE}", "data-files-page": "root",
            div { style: "display: flex; flex-wrap: wrap; align-items: center; gap: 12px;",
                div { style: "display: flex; flex-direction: column; gap: 2px; flex: 1 1 240px;",
                    span { style: "{MUTED} font-size: 12px;", "{org}" }
                    span { style: "font-size: 20px; font-weight: 700;", "{name}" }
                    span { style: "{MUTED}", "{line}" }
                    if let Some(p) = &path { span { style: "{MUTED} font-size: 11px;", "{p}" } }
                }
                button { style: "{BUTTON}", "data-files": "pin", onclick: toggle, if pinned { "Unpin" } else { "Pin to this device" } }
                if !pinned { button { style: "{BUTTON}", "data-files": "free", onclick: free, "Free up space" } }
                button { style: "{BUTTON}", onclick: checkpoint, "Checkpoint" }
            }
            if let Some(p) = problem() { div { style: "color: var(--destructive);", "{p}" } }
            DriveLine {}
            div { style: "display: flex; flex-wrap: wrap; gap: 6px; align-items: center;",
                for (i, (at, label)) in crumbs.into_iter().enumerate() {
                    if i > 0 { span { style: "{MUTED}", "/" } }
                    span {
                        style: "cursor: pointer; font-weight: 600;",
                        onclick: {
                            let mut t = target.clone();
                            move |_| { t.folder = at.clone(); t.file = None; set_current(t.clone()); }
                        },
                        "{label}"
                    }
                }
            }
            div { style: "display: flex; flex-wrap: wrap; gap: 16px; align-items: flex-start;",
                div { style: "flex: 3 1 300px; display: flex; flex-direction: column;",
                    for e in entries {
                        EntryRow { key: "{text(&e, \"path\")}", entry: e, target: target.clone() }
                    }
                }
                div { style: "flex: 2 1 260px; display: flex; flex-direction: column; gap: 12px;",
                    if let Some(file) = target.file {
                        Versions { root: root.clone(), file }
                    }
                    div { style: "{CARD}",
                        span { style: "{HEADING}", "Checkpoints" }
                        if checkpoints.is_empty() { span { style: "{MUTED}", "None yet." } }
                        for c in checkpoints.into_iter().take(12) {
                            CheckpointRow { key: "{id(&c)}", root: root.clone(), checkpoint: c }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn EntryRow(entry: Value, target: Target) -> Element {
    let dir = entry.get("dir").and_then(Value::as_bool).unwrap_or(false);
    let here = entry.get("here").and_then(Value::as_bool).unwrap_or(false);
    let name = text(&entry, "name").to_owned();
    let path = text(&entry, "path").to_owned();
    let bytes = size(entry.get("size").and_then(Value::as_u64).unwrap_or(0));
    let selected = target.file.as_deref() == Some(path.as_str());
    let background = if selected {
        "background: var(--accent);"
    } else {
        ""
    };
    let icon = if dir {
        "📁"
    } else if here {
        "●"
    } else {
        "○"
    };
    rsx! {
        div {
            style: "{ROW} {background}",
            "data-files-entry": "{path}",
            onclick: move |_| {
                let mut t = target.clone();
                if dir { t.folder.clone_from(&path); t.file = None; } else { t.file = Some(path.clone()); }
                set_current(t);
            },
            span { style: "width: 16px; {MUTED}", "{icon}" }
            span { style: "flex: 1;", "{name}" }
            span { style: "{MUTED} font-size: 12px;", "{bytes}" }
        }
    }
}

#[component]
fn Versions(root: String, file: String) -> Element {
    let history = use_read("FILES_HISTORY", &json!({ "root": root, "path": file }));
    let mut problem = use_signal(|| None::<String>);
    let sound = ["wav", "ogg", "mp3", "flac", "m4a", "aif", "aiff"]
        .iter()
        .any(|e| file.to_ascii_lowercase().ends_with(&format!(".{e}")));
    let mut versions = history()
        .and_then(Result::ok)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    versions.reverse();
    let name = file.rsplit('/').next().unwrap_or(&file).to_owned();
    rsx! {
        div { style: "{CARD}", "data-files": "versions",
            span { style: "{HEADING}", "{name}" }
            if sound { Waveform { root: root.clone(), path: file.clone() } }
            copies::Copies { root: root.clone(), path: file.clone() }
            if let Some(p) = problem() { span { style: "color: var(--destructive);", "{p}" } }
            for (i, v) in versions.into_iter().enumerate() {
                div { key: "{id(&v)}", style: "display: flex; gap: 8px; align-items: center;",
                    span { style: "flex: 1;",
                        if v.get("content").is_some_and(Value::is_null) { "deleted" } else { "{size(v.get(\"size\").and_then(Value::as_u64).unwrap_or(0))}" }
                        span { style: "{MUTED}", " · by {text(&v, \"by\")}" }
                    }
                    if i == 0 {
                        span { style: "{MUTED} font-size: 11px;", "current" }
                    } else {
                        button {
                            style: "{BUTTON}",
                            onclick: {
                                let (root, file, id) = (root.clone(), file.clone(), id(&v));
                                move |_| {
                                    let (root, file, id) = (root.clone(), file.clone(), id.clone());
                                    spawn(async move {
                                        let done = run("FILES_RESTORE", json!({ "root": root, "path": file, "version": id })).await;
                                        problem.set(done.err());
                                        bump();
                                    });
                                }
                            },
                            "Restore"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CheckpointRow(root: String, checkpoint: Value) -> Element {
    let name = text(&checkpoint, "name").to_owned();
    let by = text(&checkpoint, "by").to_owned();
    let files = checkpoint
        .get("heads")
        .and_then(Value::as_object)
        .map_or(0, serde_json::Map::len);
    let id = id(&checkpoint);
    rsx! {
        div { style: "display: flex; gap: 8px; align-items: center;",
            span { style: "flex: 1;", "{name}" span { style: "{MUTED}", " · {files} files · {by}" } }
            button {
                style: "{BUTTON}",
                onclick: move |_| {
                    let (root, id) = (root.clone(), id.clone());
                    spawn(async move {
                        let _ = run("FILES_RESTORE_CHECKPOINT", json!({ "root": root, "checkpoint": id })).await;
                        bump();
                    });
                },
                "Restore"
            }
        }
    }
}

/// A sound file's waveform, as bars.
#[component]
fn Waveform(root: String, path: String) -> Element {
    let mut peaks = use_signal(|| None::<Result<Value, String>>);
    use_effect(use_reactive!(|root, path| {
        spawn(async move {
            peaks.set(Some(
                run("FILES_PEAKS", json!({ "root": root, "path": path })).await,
            ));
        });
    }));
    match peaks() {
        None => rsx! { div { style: "height: 48px; {MUTED} font-size: 11px;", "Drawing…" } },
        Some(Err(e)) => rsx! { div { style: "{MUTED} font-size: 11px;", "No waveform: {e}" } },
        Some(Ok(p)) => {
            let heights = bars(&p, 120);
            rsx! {
                div { style: "height: 48px; display: flex; align-items: center; gap: 1px;", "data-files": "waveform",
                    for (i, h) in heights.into_iter().enumerate() {
                        div { key: "{i}", style: "flex: 1; min-width: 1px; height: {(h * 46.0).max(1.0)}px; background: var(--primary); border-radius: 1px;" }
                    }
                }
            }
        }
    }
}

#[allow(non_snake_case)]
fn AssetView() -> Element {
    let Some(target) = current() else {
        return rsx! { div { style: "{PAGE} {MUTED}", "Open an asset from the Files navigator." } };
    };
    let Some(asset) = target.asset.clone() else {
        return rsx! { RootView {} };
    };
    rsx! { AssetPage { key: "{target.root}|{asset}", root: target.root, asset } }
}

#[component]
fn AssetPage(root: String, asset: String) -> Element {
    let assets = use_read("FILES_ASSETS", &json!({ "root": root }));
    if let Some(Err(e)) = assets() {
        return rsx! { div { style: "{PAGE} color: var(--destructive);", "The files agent: {e}" } };
    }
    let found = assets().and_then(Result::ok).and_then(|all| {
        all.as_array()?
            .iter()
            .find(|a| text(a, "name") == asset)
            .cloned()
    });
    let Some(a) = found else {
        return rsx! { div { style: "{PAGE} {MUTED}", "Reading {asset}…" } };
    };
    let kind = text(&a, "kind").to_owned();
    let properties: Vec<(String, String)> = a
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
        .collect();
    let media = a
        .get("media")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let links: Vec<String> = a
        .get("links")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str().map(str::to_owned))
        .collect();
    let org = root.split('/').next().unwrap_or_default().to_owned();
    rsx! {
        div { style: "{PAGE}", "data-files-page": "asset",
            div { style: "display: flex; flex-direction: column; gap: 2px;",
                span { style: "{MUTED} font-size: 12px;", "{root}" }
                span { style: "font-size: 20px; font-weight: 700;", "{asset}" }
                span { style: "{MUTED}", "{kind} · {media.len()} media" }
            }
            if !properties.is_empty() {
                div { style: "{CARD}",
                    span { style: "{HEADING}", "Properties" }
                    for (k, v) in properties {
                        div { key: "{k}", style: "display: flex; gap: 12px;",
                            span { style: "width: 110px; {MUTED}", "{k}" }
                            span { "{v}" }
                        }
                    }
                }
            }
            if !links.is_empty() {
                div { style: "{CARD}",
                    span { style: "{HEADING}", "Links" }
                    for l in links {
                        LinkRow { key: "{l}", org: org.clone(), name: l }
                    }
                }
            }
            if !media.is_empty() {
                div { style: "{CARD}",
                    span { style: "{HEADING}", "Media" }
                    for m in media {
                        div { key: "{text(&m, \"path\")}", style: "display: flex; flex-direction: column; gap: 4px; padding: 4px 0;",
                            div { style: "display: flex; gap: 8px;",
                                span { style: "flex: 1; font-weight: 600;", "{text(&m, \"name\")}" }
                                span { style: "{MUTED} font-size: 12px;", "{size(m.get(\"size\").and_then(Value::as_u64).unwrap_or(0))}" }
                            }
                            Waveform { root: root.clone(), path: text(&m, "path").to_owned() }
                            copies::Copies { root: root.clone(), path: text(&m, "path").to_owned() }
                        }
                    }
                }
            }
        }
    }
}

/// A link to another asset of the org: opens it, wherever its library is.
#[component]
fn LinkRow(org: String, name: String) -> Element {
    let label = name.clone();
    rsx! {
        div {
            style: "{ROW} color: var(--primary);",
            onclick: move |_| {
                let (org, name) = (org.clone(), name.clone());
                spawn(async move {
                    let Ok(roots) = run("FILES_ROOTS", json!({})).await else { return };
                    for r in roots.as_array().into_iter().flatten() {
                        if text(r, "org") != org || text(r, "section") != "Assets" {
                            continue;
                        }
                        let place = text(r, "place").to_owned();
                        let Ok(assets) = run("FILES_ASSETS", json!({ "root": place })).await else { continue };
                        if assets.as_array().into_iter().flatten().any(|a| text(a, "name") == name) {
                            let _ = open(json!({ "root": place, "asset": name })).await;
                            return;
                        }
                    }
                });
            },
            "{label}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_summaries_read_short() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1_600_000_000), "1.6 GB");
        let root = json!({"pinned": true, "synced": true, "files": 21, "bytes": 1_600_000_000_u64, "path": "/x"});
        assert_eq!(summary(&root), "pinned, synced · 21 files · 1.6 GB");
        assert_eq!(
            summary(&json!({"files": 2, "bytes": 10, "path": null})),
            "online · 2 files · 10 B"
        );
    }

    #[test]
    fn the_drive_reads_in_one_line() {
        let d = json!({"mount": "/home/a/Architect", "mounted": true, "syncing": 1, "offline": false, "cache_bytes": 1_200_000_000_u64});
        assert_eq!(
            drive_line(&d),
            "Drive at /home/a/Architect · 1 syncing · cache 1.2 GB"
        );
        assert_eq!(
            drive_line(&json!({"mount": null})),
            "No drive: this device's agent has none"
        );
    }

    #[test]
    fn peaks_reduce_to_bars() {
        let peaks = json!({"data": [-100, 100, -32768, 32767, 0, 0, -16384, 16384]});
        let b = bars(&peaks, 2);
        assert_eq!(b.len(), 2);
        assert!(
            (b[0] - 1.0).abs() < 0.01 && (b[1] - 0.5).abs() < 0.01,
            "{b:?}"
        );
    }
}

/// The drive in one line: mounted where, syncing, offline, the cache.
#[must_use]
pub fn drive_line(d: &Value) -> String {
    let mount = d.get("mount").and_then(Value::as_str).unwrap_or_default();
    if mount.is_empty() {
        return "No drive: this device's agent has none".to_owned();
    }
    let mounted = d.get("mounted").and_then(Value::as_bool).unwrap_or(false);
    let syncing = d.get("syncing").and_then(Value::as_u64).unwrap_or(0);
    let offline = d.get("offline").and_then(Value::as_bool).unwrap_or(false);
    let cache = size(d.get("cache_bytes").and_then(Value::as_u64).unwrap_or(0));
    let mut out = format!(
        "{} {mount}",
        if mounted {
            "Drive at"
        } else {
            "Drive not mounted at"
        }
    );
    if syncing > 0 {
        let _ = write!(out, " · {syncing} syncing");
    }
    if offline {
        out.push_str(" · offline");
    }
    let _ = write!(out, " · cache {cache}");
    out
}

#[component]
fn DriveLine() -> Element {
    let drive = use_read("FILES_DRIVE", &json!({}));
    let Some(Ok(d)) = drive() else {
        return rsx! {};
    };
    let line = drive_line(&d);
    rsx! {
        div {
            style: "{MUTED} font-size: 12px; cursor: pointer;",
            "data-files": "drive-line",
            onclick: move |_| {
                spawn(async move {
                    let _ = run("EXT_VIEW_OPEN", json!({ "extension": ID, "view": "drive" })).await;
                });
            },
            "{line}"
        }
    }
}

#[allow(non_snake_case)]
fn DriveView() -> Element {
    let drive = use_read("FILES_DRIVE", &json!({}));
    let config = use_read("DRIVE_CONFIG", &json!({}));
    let mut mount = use_signal(String::new);
    let mut store = use_signal(String::new);
    let mut note = use_signal(|| None::<String>);
    let current = config().and_then(Result::ok);
    use_effect(use_reactive!(|current| {
        if let Some(c) = current {
            if mount.peek().is_empty() {
                mount.set(text(&c, "mount").to_owned());
            }
            if store.peek().is_empty() {
                store.set(text(&c, "store").to_owned());
            }
        }
    }));
    let status = match drive() {
        Some(Ok(d)) => drive_line(&d),
        Some(Err(e)) => format!("The files agent: {e}"),
        None => "Reading…".to_owned(),
    };
    let file = config()
        .and_then(Result::ok)
        .map(|c| text(&c, "config").to_owned())
        .unwrap_or_default();
    let save = move |_| {
        spawn(async move {
            let done = run(
                "DRIVE_CONFIGURE",
                json!({ "mount": mount(), "store": store() }),
            )
            .await;
            note.set(Some(match done {
                Ok(v) if v.get("restarted").and_then(Value::as_bool) == Some(true) => {
                    "Saved; the agent restarted.".to_owned()
                }
                Ok(_) => "Saved. Restart the agent to apply it.".to_owned(),
                Err(e) => e,
            }));
            bump();
        });
    };
    let field = "height: 30px; box-sizing: border-box; padding: 0 10px; border-radius: 7px; border: 1px solid var(--input); background: var(--background); color: var(--foreground); font-size: 13px; flex: 1;";
    rsx! {
        div { style: "{PAGE}", "data-files-page": "drive",
            span { style: "font-size: 20px; font-weight: 700;", "Drive" }
            span { style: "{MUTED}", "{status}" }
            div { style: "{CARD}",
                span { style: "{HEADING}", "Mount point" }
                input { style: "{field}", value: "{mount}", oninput: move |e| mount.set(e.value()) }
                span { style: "{HEADING}", "Store (working copies of pinned projects)" }
                input { style: "{field}", value: "{store}", oninput: move |e| store.set(e.value()) }
                div { style: "display: flex; gap: 8px; align-items: center;",
                    button { style: "{BUTTON}", onclick: save, "Save" }
                    if let Some(n) = note() { span { style: "{MUTED}", "{n}" } }
                }
                if !file.is_empty() { span { style: "{MUTED} font-size: 11px;", "{file}" } }
            }
        }
    }
}
