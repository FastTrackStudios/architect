//! The kinds the manager comes with: extensions, Luau scripts, workers,
//! themes, layouts and the keymap.

use semver::VersionReq;
use serde_json::{Value, json};

use super::{Details, Item, ItemKind, Op, Part};
use crate::browse::{Browse, Where};
use crate::extensions::Extensions;
use crate::manage::Installed;

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// Every extension, here or offered (`extension`).
pub struct ExtensionKind {
    browse: Browse,
    meters: Option<architect_workspace::Meters>,
}

impl ExtensionKind {
    #[must_use]
    pub const fn new(browse: Browse) -> Self {
        Self {
            browse,
            meters: None,
        }
    }

    /// Its views' frame times from `meters` (the app's, `EXT_VIEWS`'s):
    /// each extension's page gets a "Frame health" part.
    #[must_use]
    pub fn with_meters(mut self, meters: architect_workspace::Meters) -> Self {
        self.meters = Some(meters);
        self
    }

    const fn ext(&self) -> &Extensions {
        self.browse.extensions()
    }
}

impl ItemKind for ExtensionKind {
    fn id(&self) -> &'static str {
        "extension"
    }

    fn title(&self) -> &'static str {
        "Extensions"
    }

    fn list(&self) -> Vec<Item> {
        let problems = self.ext().problems();
        self.browse
            .rows("")
            .into_iter()
            .map(|r| {
                let mut i = Item::new("extension", &r.id, &r.name);
                i.version = Some(r.version.clone());
                i.source = match (&r.runs, r.at) {
                    (Some(runs), _) if r.overrides_builtin => runs.clone(),
                    (_, Where::Available) => {
                        format!("Available in {}", r.registry.clone().unwrap_or_default())
                    }
                    (_, at) => at.label().to_owned(),
                };
                i.enabled = (r.at != Where::Available).then_some(!r.disabled);
                i.summary = format!("{} · {}", r.kind, r.description);
                // "Update available", with what it changes, within its pin.
                if let Some(newer) = &r.newer {
                    i.summary = match &r.changelog {
                        Some(c) => format!("{} · update available: {newer} — {c}", i.summary),
                        None => format!("{} · update available: {newer}", i.summary),
                    };
                }
                i.problems = problems
                    .iter()
                    .filter(|p| p.message.starts_with(&format!("{}:", r.id)))
                    .map(|p| p.message.clone())
                    .collect();
                if r.at == Where::Available {
                    i.ops.push(Op::new("install", "Install"));
                } else {
                    i.ops.push(if r.disabled {
                        Op::new("enable", "Turn on")
                    } else {
                        Op::new("disable", "Turn off")
                    });
                    i.ops.push(Op::new("clone", "Clone to edit"));
                }
                if r.at == Where::Installed {
                    i.ops.push(Op::new("update", "Update"));
                    i.ops.push(Op::new("pin", "Pin"));
                    i.ops.push(Op::new("revoke", "Revoke its grants"));
                    i.ops.push(Op::new("remove", "Remove").destructive());
                }
                if r.overrides_builtin {
                    i.ops.push(Op::new("builtin", "Use the built-in"));
                }
                i
            })
            .collect()
    }

    fn get(&self, id: &str) -> Option<Details> {
        let item = self.list().into_iter().find(|i| i.id == id)?;
        let info = self.ext().info(id);
        let about = self.browse.about(id);
        let mut parts = Vec::new();
        if let Some(a) = &about {
            parts.push(Part::new(
                "What it adds",
                a.adds
                    .iter()
                    .cloned()
                    .chain(a.hooks.iter().map(|h| format!("Hook {h}")))
                    .collect(),
            ));
            parts.push(Part::new(
                "What it asks for",
                a.asks.iter().map(|x| x.says.clone()).collect(),
            ));
            parts.push(Part::new("Where it runs", vec![a.runner.clone()]));
        }
        let lines = self.ext().host().logs().since(id, None, 8);
        parts.push(Part::new(
            "Recent log",
            lines
                .iter()
                .map(|l| format!("{} {}: {}", l.level.name(), l.source, l.text))
                .collect(),
        ));
        let mut data = info;
        if let Some(meters) = &self.meters {
            let lines = crate::views::health_lines(meters, id);
            parts.push(Part::new(
                "Frame health",
                if lines.is_empty() {
                    vec!["No view of it measured yet".to_owned()]
                } else {
                    lines
                },
            ));
            if let Value::Object(map) = &mut data {
                map.insert(
                    "views".to_owned(),
                    Value::Array(crate::views::health_json(meters, id)),
                );
            }
        }
        Some(Details { item, parts, data })
    }

    fn run(&self, id: &str, op: &str, args: &Value) -> Result<Value, String> {
        let ext = self.ext();
        let events = |e: Vec<crate::host::Event>| {
            Value::Array(e.iter().map(crate::actions::event_json).collect())
        };
        match op {
            "install" => {
                let req = match arg(args, "version") {
                    Some(v) => VersionReq::parse(v).map_err(|e| e.to_string())?,
                    None => VersionReq::STAR,
                };
                let installed = ext
                    .install_with(id, &req, None, "person", false)
                    .map_err(|e| e.to_string())?;
                if let Installed::NeedsConsent { .. } = installed {
                    // Granting is the person's: EXT_INSTALL with `grant`,
                    // or the page's consent prompt.
                    return Ok(Extensions::installed_json(&installed));
                }
                Ok(Extensions::installed_json(&installed))
            }
            "enable" => Ok(events(ext.enable(id))),
            "disable" => Ok(events(ext.disable(id))),
            "remove" => Ok(json!({ "removed": ext.uninstall(id).map_err(|e| e.to_string())? })),
            "update" => Ok(serde_json::to_value(ext.update(Some(id))).unwrap_or(Value::Null)),
            "pin" => Ok(
                json!({ "pin": ext.pin(id, arg(args, "version"), args.get("unpin").and_then(Value::as_bool) == Some(true)).map_err(|e| e.to_string())? }),
            ),
            "clone" => {
                let c = ext.clone_to_dev(id, arg(args, "as"))?;
                Ok(json!({ "id": c.id, "folder": c.folder.display().to_string() }))
            }
            "revoke" => {
                let g = ext.host().grants().ok_or("this app keeps no consent")?;
                Ok(json!({ "revoked": g.revoke(id, None) }))
            }
            "builtin" => {
                let c = ext
                    .chosen()
                    .into_iter()
                    .find(|c| c.runs.id == id)
                    .ok_or_else(|| format!("no extension {id}"))?;
                match c.runs.origin {
                    crate::catalog::Origin::Installed => {
                        ext.uninstall(id).map_err(|e| e.to_string())?;
                    }
                    crate::catalog::Origin::Dev => {
                        if let Some(dir) = c.runs.package.dir() {
                            let _ = ext.end_dev(dir);
                        }
                    }
                    crate::catalog::Origin::Builtin => {}
                }
                Ok(
                    json!({ "runs": ext.chosen().into_iter().find(|c| c.runs.id == id).map(|c| ext.describe(&c)) }),
                )
            }
            other => Err(format!("an extension has no op {other}")),
        }
    }
}

/// Each Luau script a loaded extension runs: its actions' and its
/// hooks' (`script`, `<extension>/<path>`).
pub struct ScriptKind {
    extensions: Extensions,
}

impl ScriptKind {
    #[must_use]
    pub const fn new(extensions: Extensions) -> Self {
        Self { extensions }
    }
}

impl ItemKind for ScriptKind {
    fn id(&self) -> &'static str {
        "script"
    }

    fn title(&self) -> &'static str {
        "Luau scripts"
    }

    fn list(&self) -> Vec<Item> {
        let mut out = Vec::new();
        for l in self.extensions.host().loaded() {
            let m = &l.manifest;
            let mut add = |path: &str, what: String| {
                let mut i = Item::new("script", format!("{}/{path}", m.id), path);
                i.source.clone_from(&m.id);
                i.version = Some(m.version.to_string());
                i.summary = what;
                i.ops.push(Op::new("clone", "Clone its extension to edit"));
                out.push(i);
            };
            for a in &m.actions {
                if let Some(p) = &a.script {
                    add(p, format!("runs {} ({})", a.id, a.name));
                }
            }
            for h in &m.contributes.hooks {
                if let Some(p) = &h.script {
                    add(p, format!("hook on {}", h.on));
                }
            }
        }
        out
    }

    fn get(&self, id: &str) -> Option<Details> {
        let item = self.list().into_iter().find(|i| i.id == id)?;
        let (ext, path) = id.split_once('/')?;
        let text = self
            .extensions
            .chosen()
            .into_iter()
            .find(|c| c.runs.id == ext)
            .and_then(|c| c.runs.package.text(path).ok())
            .unwrap_or_default();
        let lines: Vec<String> = text.lines().take(60).map(str::to_owned).collect();
        let log = self
            .extensions
            .host()
            .logs()
            .since(ext, None, 8)
            .into_iter()
            .filter(|l| matches!(l.source, "print" | "script" | "hook"))
            .map(|l| format!("{} {}: {}", l.level.name(), l.source, l.text))
            .collect();
        Some(Details {
            item,
            parts: vec![
                Part::new("Source", lines),
                Part::new("Recent prints and errors", log),
            ],
            data: json!({ "extension": ext, "path": path, "text": text }),
        })
    }

    fn run(&self, id: &str, op: &str, args: &Value) -> Result<Value, String> {
        let (ext, _) = id.split_once('/').ok_or("not a script")?;
        match op {
            "clone" => {
                let c = self.extensions.clone_to_dev(ext, arg(args, "as"))?;
                Ok(json!({ "id": c.id, "folder": c.folder.display().to_string() }))
            }
            other => Err(format!("a script has no op {other}")),
        }
    }
}

/// Each process extension's process (`worker`).
#[cfg(all(feature = "process", unix))]
pub struct WorkerKind {
    processes: crate::process::Processes,
}

#[cfg(all(feature = "process", unix))]
impl WorkerKind {
    #[must_use]
    pub const fn new(processes: crate::process::Processes) -> Self {
        Self { processes }
    }
}

#[cfg(all(feature = "process", unix))]
impl ItemKind for WorkerKind {
    fn id(&self) -> &'static str {
        "worker"
    }

    fn title(&self) -> &'static str {
        "Workers"
    }

    fn list(&self) -> Vec<Item> {
        use crate::process::State;
        self.processes
            .ids()
            .into_iter()
            .filter_map(|id| self.processes.get(&id))
            // A WASI component is the `component` kind's.
            .filter(|p| !p.is_local())
            .map(|p| {
                let mut i = Item::new("worker", p.id(), p.name());
                i.source = format!("process of {}", p.id());
                let (on, summary, problem) = match p.state() {
                    State::Starting => (true, "starting".to_owned(), None),
                    State::Running { generation, pid } => (
                        true,
                        format!("running, pid {pid}, start {generation}"),
                        None,
                    ),
                    State::Exited { why, restarting } => (
                        false,
                        format!("exited{}", if restarting { ", restarting" } else { "" }),
                        Some(why),
                    ),
                    State::Stopped => (false, "stopped".to_owned(), None),
                };
                i.enabled = Some(on);
                i.summary = summary;
                i.problems.extend(problem);
                i.ops.push(Op::new("restart", "Restart"));
                i.ops.push(Op::new("stop", "Stop").destructive());
                i
            })
            .collect()
    }

    fn run(&self, id: &str, op: &str, _: &Value) -> Result<Value, String> {
        let p = self
            .processes
            .get(id)
            .ok_or_else(|| format!("no worker {id}"))?;
        match op {
            "restart" => p.restart(),
            "stop" => p.stop(),
            other => return Err(format!("a worker has no op {other}")),
        }
        Ok(json!({ "worker": id, "op": op }))
    }
}

/// Every theme in the app's store (`theme`).
pub struct ThemeKind {
    store: architect_theme::ThemeStore,
    list: architect::action::ActionList,
}

impl ThemeKind {
    /// The themes of `store`, chosen with `list`'s `THEME_SET`.
    #[must_use]
    pub const fn new(
        store: architect_theme::ThemeStore,
        list: architect::action::ActionList,
    ) -> Self {
        Self { store, list }
    }

    fn file(&self, id: &str) -> Option<std::path::PathBuf> {
        self.store
            .themes()
            .into_iter()
            .find(|t| t.id == id)
            .and_then(|t| match t.source {
                architect_theme::Source::File(f) => Some(f.into()),
                _ => None,
            })
    }
}

impl ItemKind for ThemeKind {
    fn id(&self) -> &'static str {
        "theme"
    }

    fn title(&self) -> &'static str {
        "Themes"
    }

    fn list(&self) -> Vec<Item> {
        let current = self.store.current();
        self.store
            .themes()
            .into_iter()
            .map(|t| {
                let mut i = Item::new("theme", &t.id, &t.label);
                i.source = match &t.source {
                    architect_theme::Source::Builtin => "built in".to_owned(),
                    architect_theme::Source::File(f) => format!("file {f}"),
                    architect_theme::Source::Extension { id, .. } => format!("extension {id}"),
                    architect_theme::Source::Memory => "made by the app".to_owned(),
                };
                i.enabled = Some(current.id() == t.id);
                i.summary = t.scheme.map(|s| s.as_str().to_owned()).unwrap_or_default();
                i.problems.clone_from(&t.problems);
                i.ops.push(Op::new("use", "Use"));
                if matches!(t.source, architect_theme::Source::File(_)) {
                    i.ops.push(Op::new("clone", "Clone to edit"));
                    i.ops
                        .push(Op::new("remove", "Remove its file").destructive());
                }
                i
            })
            .collect()
    }

    fn run(&self, id: &str, op: &str, args: &Value) -> Result<Value, String> {
        match op {
            "use" => {
                let out = self
                    .list
                    .run_json("THEME_SET", json!({ "one": id }).to_string().as_bytes())
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::from_slice(&out).unwrap_or(Value::Null))
            }
            "clone" => {
                let file = self
                    .file(id)
                    .ok_or("only a theme file can be cloned here")?;
                let new = arg(args, "as").map_or_else(|| format!("{id}-copy"), str::to_owned);
                let to = file.with_file_name(format!("{new}.json"));
                if to.exists() {
                    return Err(format!("{} exists: give `as`", to.display()));
                }
                std::fs::copy(&file, &to).map_err(|e| e.to_string())?;
                Ok(json!({ "file": to.display().to_string() }))
            }
            "remove" => {
                let file = self.file(id).ok_or("only a theme file can be removed")?;
                std::fs::remove_file(&file).map_err(|e| e.to_string())?;
                Ok(json!({ "removed": self.store.remove_theme(id) }))
            }
            other => Err(format!("a theme has no op {other}")),
        }
    }
}

/// The interface as files: each `layouts/<name>.json` and `keys.json`
/// (`layout`, ids `layout:<name>` and `keys`).
pub struct LayoutKind {
    home: architect_workspace::home::Home,
}

impl LayoutKind {
    #[must_use]
    pub const fn new(home: architect_workspace::home::Home) -> Self {
        Self { home }
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.home.dir().join("layouts").join(format!("{name}.json"))
    }
}

impl ItemKind for LayoutKind {
    fn id(&self) -> &'static str {
        "layout"
    }

    fn title(&self) -> &'static str {
        "Layouts and keys"
    }

    fn list(&self) -> Vec<Item> {
        let problems = self.home.problems();
        let shown = self.home.active();
        let mut out: Vec<Item> = self
            .home
            .names()
            .into_iter()
            .map(|n| {
                let title = self
                    .home
                    .layout(&n)
                    .and_then(|l| l.name)
                    .unwrap_or_else(|| n.clone());
                let mut i = Item::new("layout", format!("layout:{n}"), title);
                let builtin = self.home.is_builtin(&n);
                i.source = if builtin {
                    "built in (read-only)".to_owned()
                } else {
                    format!("layouts/{n}.json")
                };
                i.enabled = Some(n == shown);
                "a layout".clone_into(&mut i.summary);
                i.problems = problems
                    .iter()
                    .filter(|p| p.file == format!("layouts/{n}.json"))
                    .map(|p| p.message.clone())
                    .collect();
                i.ops.push(Op::new("use", "Show"));
                i.ops.push(Op::new("clone", "Clone to edit"));
                if !builtin {
                    i.ops
                        .push(Op::new("remove", "Remove its file").destructive());
                }
                i
            })
            .collect();
        // A layout file that never read has no layout: its problem alone.
        for p in problems.iter().filter(|p| p.file.starts_with("layouts/")) {
            let n = p
                .file
                .trim_start_matches("layouts/")
                .trim_end_matches(".json");
            if !out.iter().any(|i| i.id == format!("layout:{n}")) {
                let mut i = Item::new("layout", format!("layout:{n}"), n);
                i.source.clone_from(&p.file);
                "does not read".clone_into(&mut i.summary);
                i.problems.push(p.message.clone());
                i.ops
                    .push(Op::new("remove", "Remove its file").destructive());
                out.push(i);
            }
        }
        let mut keys = Item::new("layout", "keys", "Keymap (keys.json)");
        "keys.json".clone_into(&mut keys.source);
        keys.enabled = Some(self.home.keys().is_some());
        "the app's keys, rebound".clone_into(&mut keys.summary);
        keys.problems = problems
            .iter()
            .filter(|p| p.file == "keys.json")
            .map(|p| p.message.clone())
            .collect();
        out.push(keys);
        out
    }

    fn run(&self, id: &str, op: &str, args: &Value) -> Result<Value, String> {
        let name = id
            .strip_prefix("layout:")
            .ok_or("keys.json is edited as a file")?;
        match op {
            "use" => {
                self.home.use_layout(name)?;
                Ok(json!({ "shown": name }))
            }
            "clone" => {
                let new = arg(args, "as").map_or_else(|| format!("{name}-copy"), str::to_owned);
                let to = self.path(&new);
                if to.exists() {
                    return Err(format!("{} exists: give `as`", to.display()));
                }
                let text = std::fs::read_to_string(self.path(name)).map_err(|e| e.to_string())?;
                std::fs::write(&to, text).map_err(|e| e.to_string())?;
                let _ = self.home.poll();
                Ok(json!({ "file": to.display().to_string() }))
            }
            "remove" => {
                if self.home.is_builtin(name) {
                    return Err(format!("{name} is built in: clone it to change it"));
                }
                std::fs::remove_file(self.path(name)).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_file(
                    self.home
                        .dir()
                        .join("layouts")
                        .join(format!("{name}.error")),
                );
                let _ = self.home.poll();
                Ok(json!({ "removed": name }))
            }
            other => Err(format!("a layout has no op {other}")),
        }
    }
}

/// Each WASI component running (`component`): its memory against its
/// limit, its calls, the times it trapped and why.
#[cfg(feature = "wasi")]
pub struct ComponentKind {
    components: crate::wasi::Components,
}

#[cfg(feature = "wasi")]
impl ComponentKind {
    #[must_use]
    pub const fn new(components: crate::wasi::Components) -> Self {
        Self { components }
    }
}

#[cfg(feature = "wasi")]
impl ItemKind for ComponentKind {
    fn id(&self) -> &'static str {
        "component"
    }

    fn title(&self) -> &'static str {
        "Components"
    }

    fn list(&self) -> Vec<Item> {
        self.components
            .all()
            .into_iter()
            .map(|c| {
                let mut i = Item::new("component", &c.id, &c.name);
                i.source = format!("WASI component of {}", c.id);
                i.enabled = Some(c.alive());
                i.summary = format!(
                    "{} of {} MiB, {} calls, {} traps, {} ms a call at most",
                    format_mib(c.memory()),
                    c.limits.memory >> 20,
                    c.calls(),
                    c.traps(),
                    c.limits.call.as_millis()
                );
                // Its workers: more instances of it, on the runner's pool.
                let running = c.workers_alive();
                if running > 0 {
                    i.summary = format!("{}, {running} worker(s) running", i.summary);
                }
                i.problems.extend(c.last_trap());
                i.ops.push(Op::new("restart", "Restart"));
                i
            })
            .collect()
    }

    fn run(&self, id: &str, op: &str, _: &Value) -> Result<Value, String> {
        let c = self
            .components
            .get(id)
            .ok_or_else(|| format!("no component {id}"))?;
        match op {
            "restart" => c.restart(),
            other => return Err(format!("a component has no op {other}")),
        }
        Ok(json!({ "component": id, "op": op }))
    }
}

#[cfg(feature = "wasi")]
fn format_mib(bytes: usize) -> String {
    let tenths = bytes.saturating_mul(10) >> 20;
    format!("{}.{} MiB", tenths / 10, tenths % 10)
}
