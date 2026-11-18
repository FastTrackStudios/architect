//! `architect-desktop dev`: an isolated Architect for development, as many
//! at once as are wanted, none touching the person's own app or another.
//!
//! Each instance is a folder of its own (`<cache>/architect-dev/<app>/<name>`,
//! or under `$ARCHITECT_DEV_ROOT`): its app folder (settings, sessions,
//! the node's data: a separate single instance), its vault, its secrets
//! in a file (never the keychain), its log, and a control socket of its
//! own, so `architect-desktop actions --app <socket>` drives exactly it.
//! Its node stays on this machine (`ARCHITECT_IROH=local`), and it takes
//! no system-wide shortcut (`ARCHITECT_DEV_INSTANCE`). The name defaults
//! to the working folder's (its last part and a hash of the whole path),
//! so every checkout and worktree gets its own without asking.
//!
//! ```text
//! architect-desktop dev                          # this folder's instance: start it, or say where it is
//! architect-desktop dev --extension features/x   # with a development copy (reloaded on save)
//! architect-desktop dev --only navigator,x       # every other extension off (--bare: all of them; each --only adds)
//! architect-desktop dev --open "Charts/A.kf"     # opened on a file (EXT_OPEN)
//! architect-desktop dev --layout focus           # in a layout of the app's folder (HOME_LAYOUT_USE)
//! architect-desktop dev --layout layouts/x.json  # a layout file, copied into its folder as `x` first
//! architect-desktop dev --collapse left,right    # opened with those sides folded away
//! architect-desktop dev --hide all               # no rail, docks, status or Now Playing bar: the tabs and the page
//! architect-desktop dev --run ACTION[=JSON]      # any action, once it is up
//! architect-desktop dev --seed <dir> | --vault <dir> | --fresh | --name <n> | --wait
//! architect-desktop dev list                     # the instances, running or not
//! architect-desktop dev stop [<name> | --all]    # quit them
//! architect-desktop dev path [--name <n>]        # its folder and socket, as JSON
//! ```
//!
//! The vault is the built-in demo's copy unless `--vault` or `--seed`
//! says otherwise; the tour is marked done, so it opens on the app.
//!
//! **Without the launcher**: the app makes itself an instance when it
//! starts with `ARCHITECT_DEV` set (`1`: named for the working folder; or
//! the name), so `devenv up` or `dx serve` set one variable and the window
//! they run, hot reload and all, is isolated. The flags have variables:
//! `ARCHITECT_DEV_ONLY` (`id,id`, `none` for bare), `ARCHITECT_DEV_EXTENSIONS`
//! (folders, `:`-separated), `ARCHITECT_DEV_OPEN`, `ARCHITECT_DEV_LAYOUT`,
//! `ARCHITECT_DEV_COLLAPSE` (`left,right`), `ARCHITECT_DEV_VAULT`,
//! `ARCHITECT_DEV_SEED`, `ARCHITECT_DEV_FRESH=1`,
//! `ARCHITECT_DEV_RUN` (`[["ACTION", {args}], …]`). `dev` is that
//! variable set for a window it starts, which it waits for.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use architect_distribution::Distribution;
use serde_json::{Value, json};

/// Where instances live, when `$ARCHITECT_DEV_ROOT` names none.
const ROOT_ENV: &str = "ARCHITECT_DEV_ROOT";
/// The environment an instance runs with: its name (`ARCHITECT_DEV_INSTANCE`,
/// read by the app to stand aside from what is system-wide).
pub const INSTANCE_ENV: &str = "ARCHITECT_DEV_INSTANCE";
/// Set (`1`, or a name), the app is an isolated instance ([`from_env`]).
pub const DEV_ENV: &str = "ARCHITECT_DEV";
const ONLY_ENV: &str = "ARCHITECT_DEV_ONLY";
const EXTENSIONS_ENV: &str = "ARCHITECT_DEV_EXTENSIONS";
const OPEN_ENV: &str = "ARCHITECT_DEV_OPEN";

/// How long `--open` waits for a development copy's opener to claim the
/// file (its program started and said what it opens).
const OPENER_WAIT: Duration = Duration::from_secs(20);
const LAYOUT_ENV: &str = "ARCHITECT_DEV_LAYOUT";
const VAULT_ENV: &str = "ARCHITECT_DEV_VAULT";
const SEED_ENV: &str = "ARCHITECT_DEV_SEED";
const FRESH_ENV: &str = "ARCHITECT_DEV_FRESH";
/// Set by `dev` on the window it starts: its output is the log already.
const LOGGED_ENV: &str = "ARCHITECT_DEV_LOGGED";
const RUN_ENV: &str = "ARCHITECT_DEV_RUN";
const COLLAPSE_ENV: &str = "ARCHITECT_DEV_COLLAPSE";
/// The sides `--collapse` folds.
const SIDES: [&str; 3] = ["left", "right", "bottom"];
const HIDE_ENV: &str = "ARCHITECT_DEV_HIDE";
/// What `--hide` takes away: the rail (the shell's ribbon slot), the
/// status bar (its status slot), the Now Playing bar (its `now_playing`
/// slot), and the docks.
const PARTS: [&str; 6] = ["rail", "status", "now-playing", "left", "right", "bottom"];
/// How long a start waits for the window's socket to answer.
const READY: Duration = Duration::from_secs(180);

const USAGE: &str = "usage: architect-desktop dev [--name <n>] [--fresh] [--vault <dir> | --seed <dir>] \
[--only <id,id> | --bare] [--extension <dir>]... [--open <path>] [--layout <name>] [--collapse <side,side>] [--hide <part,part>] \
[--run <ACTION[=JSON]>]... [--wait]
       architect-desktop dev list
       architect-desktop dev stop [<name> | --all]
       architect-desktop dev path [--name <n>]";

/// What `dev …` was asked.
#[derive(Debug, Default, PartialEq, Eq)]
struct Ask {
    name: Option<String>,
    fresh: bool,
    vault: Option<PathBuf>,
    seed: Option<PathBuf>,
    /// The extensions left on (`None`: all but the updater; `Some([])`:
    /// none, `--bare`).
    only: Option<Vec<String>>,
    extensions: Vec<PathBuf>,
    open: Option<String>,
    layout: Option<String>,
    /// The sides folded once it is up (`left`: the sidebar to the rail;
    /// `right`, `bottom`: their docks collapsed).
    collapse: Vec<String>,
    /// What is hidden once it is up (`rail`, `status`, `left`, `right`,
    /// `bottom`): the shell's slots and the docks, no strip left.
    hide: Vec<String>,
    run: Vec<(String, Value)>,
    wait: bool,
}

impl Ask {
    /// The variables that ask the same of an app ([`from_env`]).
    fn to_env(&self) -> Vec<(&'static str, std::ffi::OsString)> {
        let mut out: Vec<(&'static str, std::ffi::OsString)> = Vec::new();
        if let Some(only) = &self.only {
            let list = if only.is_empty() {
                "none".to_owned()
            } else {
                only.join(",")
            };
            out.push((ONLY_ENV, list.into()));
        }
        if !self.extensions.is_empty()
            && let Ok(joined) = std::env::join_paths(
                self.extensions
                    .iter()
                    .map(|d| d.canonicalize().unwrap_or_else(|_| d.clone())),
            )
        {
            out.push((EXTENSIONS_ENV, joined));
        }
        if let Some(open) = &self.open {
            out.push((OPEN_ENV, open.into()));
        }
        if let Some(layout) = &self.layout {
            // A layout file by its whole path: the window may start elsewhere.
            let file = Path::new(layout);
            let layout = if is_layout_file(layout) {
                file.canonicalize()
                    .map_or_else(|_| layout.clone(), |p| p.to_string_lossy().into_owned())
            } else {
                layout.clone()
            };
            out.push((LAYOUT_ENV, layout.into()));
        }
        if let Some(vault) = &self.vault {
            out.push((
                VAULT_ENV,
                vault
                    .canonicalize()
                    .unwrap_or_else(|_| vault.clone())
                    .into(),
            ));
        }
        if let Some(seed) = &self.seed {
            out.push((
                SEED_ENV,
                seed.canonicalize().unwrap_or_else(|_| seed.clone()).into(),
            ));
        }
        if !self.collapse.is_empty() {
            out.push((COLLAPSE_ENV, self.collapse.join(",").into()));
        }
        if !self.hide.is_empty() {
            out.push((HIDE_ENV, self.hide.join(",").into()));
        }
        if !self.run.is_empty() {
            let runs: Vec<Value> = self.run.iter().map(|(id, a)| json!([id, a])).collect();
            out.push((RUN_ENV, Value::Array(runs).to_string().into()));
        }
        out
    }

    /// What the environment asks ([`to_env`](Self::to_env)'s variables).
    fn from_env() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Self {
            name: None,
            fresh: var(FRESH_ENV).is_some_and(|v| v != "0"),
            vault: var(VAULT_ENV).map(PathBuf::from),
            seed: var(SEED_ENV).map(PathBuf::from),
            only: var(ONLY_ENV).map(|v| {
                if v == "none" {
                    Vec::new()
                } else {
                    v.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                }
            }),
            extensions: std::env::var_os(EXTENSIONS_ENV)
                .map(|v| {
                    std::env::split_paths(&v)
                        .filter(|p| !p.as_os_str().is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            open: var(OPEN_ENV),
            layout: var(LAYOUT_ENV),
            hide: var(HIDE_ENV).map_or_else(Vec::new, |v| {
                names(&v, &PARTS).unwrap_or_else(|why| {
                    let _ = writeln!(std::io::stderr(), "{HIDE_ENV}: {why}");
                    Vec::new()
                })
            }),
            collapse: var(COLLAPSE_ENV).map_or_else(Vec::new, |v| {
                names(&v, &SIDES).unwrap_or_else(|why| {
                    let _ = writeln!(std::io::stderr(), "{COLLAPSE_ENV}: {why}");
                    Vec::new()
                })
            }),
            run: var(RUN_ENV)
                .and_then(|v| serde_json::from_str::<Vec<(String, Value)>>(&v).ok())
                .unwrap_or_default(),
            wait: false,
        }
    }
}

fn parse(args: &[String]) -> Result<Ask, String> {
    let mut ask = Ask::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match a.as_str() {
            "--name" => ask.name = Some(value("--name")?),
            "--fresh" => ask.fresh = true,
            "--wait" => ask.wait = true,
            "--vault" => ask.vault = Some(value("--vault")?.into()),
            "--seed" => ask.seed = Some(value("--seed")?.into()),
            "--bare" => ask.only = Some(Vec::new()),
            "--only" => {
                // Each `--only` adds to the list: `--only a --only b,c` is
                // `--only a,b,c` (a script's own and its caller's).
                let only = ask.only.get_or_insert_with(Vec::new);
                for id in value("--only")?
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    if !only.iter().any(|o| o == id) {
                        only.push(id.to_owned());
                    }
                }
            }
            "--extension" => ask.extensions.push(value("--extension")?.into()),
            "--open" => ask.open = Some(value("--open")?),
            "--layout" => ask.layout = Some(value("--layout")?),
            "--collapse" => {
                for side in names(&value("--collapse")?, &SIDES)? {
                    if !ask.collapse.contains(&side) {
                        ask.collapse.push(side);
                    }
                }
            }
            "--hide" => {
                for part in names(&value("--hide")?, &PARTS)? {
                    if !ask.hide.contains(&part) {
                        ask.hide.push(part);
                    }
                }
            }
            "--run" => {
                let v = value("--run")?;
                let (id, args) = match v.split_once('=') {
                    Some((id, json)) => (
                        id.to_owned(),
                        serde_json::from_str(json)
                            .map_err(|e| format!("--run {id}: its arguments are not JSON: {e}"))?,
                    ),
                    None => (v, Value::Null),
                };
                ask.run.push((id, args));
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if ask.vault.is_some() && ask.seed.is_some() {
        return Err("--vault and --seed: one or the other".to_owned());
    }
    Ok(ask)
}

/// `left,right` as names out of `known` (`all`: every one).
fn names(list: &str, known: &[&str]) -> Result<Vec<String>, String> {
    if list.trim() == "all" {
        return Ok(known.iter().map(|s| (*s).to_owned()).collect());
    }
    list.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            if known.contains(&s) {
                Ok(s.to_owned())
            } else {
                Err(format!("no {s:?}: {} or all", known.join(", ")))
            }
        })
        .collect()
}

/// FNV-1a: a short name for a path that is the same on every run.
fn hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:08x}", h >> 32)
}

/// The working folder's instance name: its last part and a hash of the
/// whole path, so two checkouts of the same repo differ.
fn default_name(cwd: &Path) -> String {
    let base: String = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let base = if base.is_empty() {
        "dev".to_owned()
    } else {
        base
    };
    format!("{base}-{}", hash(&cwd.to_string_lossy()))
}

fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with('.')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "{name:?}: a name is letters, digits, `-`, `_` and `.`"
        ));
    }
    Ok(())
}

/// Where the instances' folders are.
fn root() -> PathBuf {
    if let Some(dir) = std::env::var_os(ROOT_ENV).filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map_or_else(std::env::temp_dir, PathBuf::from);
    let cache = if cfg!(target_os = "macos") {
        home.join("Library/Caches")
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .filter(|d| !d.is_empty())
            .map_or_else(|| home.join(".cache"), PathBuf::from)
    };
    cache.join("architect-dev")
}

/// One instance's places.
#[derive(Clone, Debug)]
struct Instance {
    name: String,
    dir: PathBuf,
    /// The app's name: its control socket's (`<app>.sock`).
    app_name: String,
}

impl Instance {
    fn new(name: String, app_name: &str) -> Self {
        let dir = root().join(app_name).join(&name);
        Self {
            name,
            dir,
            app_name: app_name.to_owned(),
        }
    }
    fn app(&self) -> PathBuf {
        self.dir.join("app")
    }
    fn log(&self) -> PathBuf {
        self.dir.join("app.log")
    }
    fn record(&self) -> PathBuf {
        self.dir.join("instance.json")
    }
    /// Written once the window answers and applied what it was asked:
    /// its pid, or `failed: <why>`.
    fn ready(&self) -> PathBuf {
        self.dir.join("ready")
    }
    /// The control socket's folder: short (a socket's path has ~100
    /// bytes at most), so under the system's /tmp, named for the instance.
    fn socket_dir(&self) -> PathBuf {
        if cfg!(unix) {
            PathBuf::from(format!(
                "/tmp/architect-dev-{}",
                hash(&self.dir.to_string_lossy())
            ))
        } else {
            self.dir.join("run")
        }
    }
    fn socket(&self) -> PathBuf {
        if cfg!(windows) {
            // A named pipe: the folder's hash keeps it apart.
            PathBuf::from(format!(
                r"\\.\pipe\fts-{}-dev-{}",
                self.app_name,
                hash(&self.dir.to_string_lossy())
            ))
        } else {
            self.socket_dir().join(format!("{}.sock", self.app_name))
        }
    }
    fn info(&self, pid: Option<u32>, vault: &Path, running: bool) -> Value {
        json!({
            "name": self.name,
            "running": running,
            "pid": pid,
            "dir": self.dir,
            "socket": self.socket(),
            "vault": vault,
            "log": self.log(),
            "drive": format!("architect-desktop actions --app {} list", self.socket().display()),
            "stop": format!("architect-desktop dev stop {}", self.name),
        })
    }
}

/// `<app> dev …`, for the app `dist` is.
pub fn main(dist: &Distribution, args: &[String]) -> ExitCode {
    let app = dist.app_name();
    let result = match args.first().map(String::as_str) {
        Some("list") => {
            list(app);
            Ok(())
        }
        Some("stop") => stop(app, args.get(1..).unwrap_or_default()),
        Some("path") => path(app, args.get(1..).unwrap_or_default()),
        Some("--help" | "-h" | "help") => {
            let _ = writeln!(std::io::stdout(), "{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => start(dist, args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            let _ = writeln!(std::io::stderr(), "architect-desktop dev: {why}");
            ExitCode::FAILURE
        }
    }
}

fn instance_for(app: &str, name: Option<String>) -> Result<Instance, String> {
    let name = match name {
        Some(n) => n,
        None => default_name(&std::env::current_dir().map_err(|e| e.to_string())?),
    };
    check_name(&name)?;
    Ok(Instance::new(name, app))
}

fn print(v: &Value) {
    let _ = writeln!(
        std::io::stdout(),
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_default()
    );
}

fn path(app: &str, args: &[String]) -> Result<(), String> {
    let ask = parse(args)?;
    let inst = instance_for(app, ask.name)?;
    // What it was started with (its record: the vault it was given), else
    // where it would be.
    let vault = std::fs::read_to_string(inst.record())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("vault").and_then(Value::as_str).map(PathBuf::from))
        .unwrap_or_else(|| inst.dir.join("vault"));
    print(&inst.info(None, &vault, answers(&inst.socket())));
    Ok(())
}

fn list(app: &str) {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root().join(app)) {
        for e in entries.flatten() {
            let inst = Instance::new(e.file_name().to_string_lossy().into_owned(), app);
            if !inst.record().is_file() {
                continue;
            }
            let kept: Value = std::fs::read_to_string(inst.record())
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(Value::Null);
            let mut info = kept;
            if let Some(o) = info.as_object_mut() {
                o.insert("running".to_owned(), Value::Bool(answers(&inst.socket())));
            }
            out.push(info);
        }
    }
    print(&Value::Array(out));
}

fn stop(app: &str, args: &[String]) -> Result<(), String> {
    let all = args.iter().any(|a| a == "--all");
    let names: Vec<String> = if all {
        std::fs::read_dir(root().join(app))
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![instance_for(app, args.first().filter(|a| !a.starts_with("--")).cloned())?.name]
    };
    for name in names {
        let inst = Instance::new(name, app);
        if quit(&inst) {
            let _ = writeln!(std::io::stdout(), "stopped {}", inst.name);
        }
    }
    Ok(())
}

/// Quits `inst` if it runs: `PRESENCE_QUIT`, else its process is ended.
fn quit(inst: &Instance) -> bool {
    if !answers(&inst.socket()) {
        return false;
    }
    if act(&inst.socket(), "PRESENCE_QUIT", &Value::Null).is_ok() {
        // Gone only once it no longer answers: a start right after must
        // not find the one quitting.
        let began = Instant::now();
        while answers(&inst.socket()) && began.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(250));
        }
        return true;
    }
    let pid = std::fs::read_to_string(inst.record())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("pid").and_then(Value::as_u64));
    #[cfg(unix)]
    if let Some(pid) = pid {
        return Command::new("kill")
            .arg(pid.to_string())
            .status()
            .is_ok_and(|s| s.success());
    }
    let _ = pid;
    false
}

/// Runs `action` with `args` in the app at `socket` (this binary's own
/// actions CLI), answering its JSON.
fn act(socket: &Path, action: &str, args: &Value) -> Result<Value, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .arg("actions")
        .arg("--app")
        .arg(socket)
        .args(["run", action, "--json", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(args.to_string().as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(serde_json::from_str(text.trim()).unwrap_or(Value::Null))
}

/// Whether an app answers on `socket`.
fn answers(socket: &Path) -> bool {
    (cfg!(windows) || socket.exists()) && act(socket, "APP_STATUS", &Value::Null).is_ok()
}

fn start(dist: &Distribution, args: &[String]) -> Result<(), String> {
    let ask = parse(args).map_err(|why| format!("{why}\n{USAGE}"))?;
    let inst = instance_for(dist.app_name(), ask.name.clone())?;
    let socket = inst.socket();
    if answers(&socket) {
        // Running already: what is asked now, applied to it.
        if ask.fresh {
            quit(&inst);
        } else {
            apply(&inst, &ask)?;
            print(&inst.info(None, &vault_of(&inst, &ask), true));
            if ask.wait {
                while answers(&socket) {
                    std::thread::sleep(Duration::from_secs(2));
                }
                let tail = log_tail(&inst);
                if tail.contains("panicked") {
                    return Err(format!(
                        "the app panicked; its log ({}) ends:\n{tail}",
                        inst.log().display()
                    ));
                }
            }
            return Ok(());
        }
    }
    let mut child = launch(&inst, &ask)?;
    let pid = child.id();
    wait_ready(&inst, &mut child)?;
    print(&inst.info(Some(pid), &vault_of(&inst, &ask), true));
    if ask.wait {
        // Until it ends: a quit is a success, a panic or a signal is not.
        let status = child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!(
                "the app (pid {pid}) ended: {status}; its log ({}) ends:\n{}",
                inst.log().display(),
                log_tail(&inst)
            ));
        }
    }
    Ok(())
}

fn vault_of(inst: &Instance, ask: &Ask) -> PathBuf {
    ask.vault.clone().unwrap_or_else(|| inst.dir.join("vault"))
}

/// Starts the instance's window: this binary with `ARCHITECT_DEV` naming
/// it and the asks as variables (the window makes itself the instance,
/// [`from_env`]), in a process group of its own (a terminal's Ctrl-C or
/// its closing leaves it), its output in the instance's log.
fn launch(inst: &Instance, ask: &Ask) -> Result<std::process::Child, String> {
    // Started over here, not in the window: there it would take the log
    // this opens with it.
    if ask.fresh && inst.dir.exists() {
        std::fs::remove_dir_all(&inst.dir).map_err(|e| format!("{}: {e}", inst.dir.display()))?;
    }
    std::fs::create_dir_all(&inst.dir).map_err(|e| format!("{}: {e}", inst.dir.display()))?;
    let _ = std::fs::remove_file(inst.ready());
    let log =
        std::fs::File::create(inst.log()).map_err(|e| format!("{}: {e}", inst.log().display()))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.env(DEV_ENV, &inst.name)
        .envs(ask.to_env())
        .env_remove(FRESH_ENV)
        .env(LOGGED_ENV, "1")
        .env_remove(INSTANCE_ENV)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }
    cmd.spawn()
        .map_err(|e| format!("could not start the app: {e}"))
}

/// Waits for the window `pid` to answer and to have applied what it was
/// asked (its `ready` file names it).
fn wait_ready(inst: &Instance, child: &mut std::process::Child) -> Result<(), String> {
    let pid = child.id();
    let began = Instant::now();
    while began.elapsed() < READY {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "the app (pid {pid}) ended before it was ready: {status}; its log ({}) ends:\n{}",
                inst.log().display(),
                log_tail(inst)
            ));
        }
        let ready = std::fs::read_to_string(inst.ready()).unwrap_or_default();
        if ready.trim() == pid.to_string() {
            return Ok(());
        }
        if let Some(why) = ready.trim().strip_prefix("failed: ") {
            return Err(why.to_owned());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!(
        "the app (pid {pid}) was not ready on {} within {}s; its log ({}) ends:\n{}",
        inst.socket().display(),
        READY.as_secs(),
        inst.log().display(),
        log_tail(inst)
    ))
}

/// The last lines of the instance's log (a panic's backtrace fits).
fn log_tail(inst: &Instance) -> String {
    let text = std::fs::read_to_string(inst.log()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines
        .get(lines.len().saturating_sub(40)..)
        .unwrap_or_default()
        .join("\n")
}

/// A window made an instance by its environment alone (not started by
/// `dev`, so its output is not the log): a panic is written to the log
/// too, with its backtrace, before the usual report.
fn log_panics(log: PathBuf) {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
        {
            let trace = std::backtrace::Backtrace::force_capture();
            let _ = writeln!(f, "{info}\n{trace}");
        }
        before(info);
    }));
}

/// The app as an isolated instance when `ARCHITECT_DEV` asks.
///
/// The first thing `run` does, before any thread starts. Its places are
/// set as the app's environment (its folder, socket, secrets, vault; its
/// node kept on this machine), its tour marked done, and once its socket
/// answers what the other variables ask is applied, on a thread of its own.
pub fn from_env(dist: &Distribution) {
    let Some(asked) = std::env::var(DEV_ENV)
        .ok()
        .filter(|v| !v.is_empty() && v != "0")
    else {
        return;
    };
    let name = if matches!(asked.as_str(), "1" | "true" | "yes") {
        std::env::current_dir().map_or_else(|_| "dev".to_owned(), |d| default_name(&d))
    } else {
        asked
    };
    if let Err(why) = check_name(&name) {
        let _ = writeln!(std::io::stderr(), "{DEV_ENV}: {why}; not isolated");
        return;
    }
    let ask = Ask::from_env();
    let inst = Instance::new(name, dist.app_name());
    if ask.fresh && inst.dir.exists() {
        let _ = std::fs::remove_dir_all(&inst.dir);
    }
    let app = inst.app();
    let _ = std::fs::create_dir_all(&app);
    if std::env::var_os(LOGGED_ENV).is_none() {
        let _ = std::fs::write(
            inst.log(),
            format!(
                "pid {} (its output is where it was started)\n",
                std::process::id()
            ),
        );
        log_panics(inst.log());
    }
    let _ = std::fs::create_dir_all(inst.socket_dir());
    // The tour is for a person's first run, not a developer's.
    let _ = std::fs::write(app.join("tour-place"), "done");
    let vault = vault_of(&inst, &ask);
    let mut vars: Vec<(String, std::ffi::OsString)> = vec![
        (dist.folder_env(), app.into_os_string()),
        ("FTS_CONTROL".to_owned(), "1".into()),
        (
            "FTS_CONTROL_DIR".to_owned(),
            inst.socket_dir().into_os_string(),
        ),
        ("ARCHITECT_FORBID_KEYCHAIN".to_owned(), "1".into()),
        (
            "ARCHITECT_SECRET_STORE".to_owned(),
            format!("file:{}", inst.dir.join("secrets.json").display()).into(),
        ),
        ("ARCHITECT_IROH".to_owned(), "local".into()),
        ("ARCHITECT_VAULT".to_owned(), vault.clone().into_os_string()),
        (INSTANCE_ENV.to_owned(), inst.name.clone().into()),
    ];
    if ask.vault.is_none() {
        let seed = ask
            .seed
            .clone()
            .map_or_else(|| "builtin:demo".into(), PathBuf::into_os_string);
        vars.push(("ARCHITECT_VAULT_SEED".to_owned(), seed));
    }
    for (k, v) in vars {
        // SAFETY: `run` calls this first, on the main thread, before any
        // thread exists.
        unsafe { std::env::set_var(k, v) };
    }
    let info = inst.info(Some(std::process::id()), &vault, true);
    let _ = std::fs::write(
        inst.record(),
        serde_json::to_string_pretty(&info).unwrap_or_default(),
    );
    let _ = std::fs::remove_file(inst.ready());
    let _ = std::thread::Builder::new()
        .name("dev-instance".to_owned())
        .spawn(move || {
            let began = Instant::now();
            while !answers(&inst.socket()) {
                if began.elapsed() > READY {
                    return;
                }
                std::thread::sleep(Duration::from_millis(400));
            }
            let mark = match apply(&inst, &ask) {
                Ok(()) => std::process::id().to_string(),
                Err(why) => {
                    tracing::warn!(dev.instance = %inst.name, %why, "dev instance: not all it was asked");
                    format!("failed: {why}")
                }
            };
            let _ = std::fs::write(inst.ready(), mark);
        });
}

/// What the instance is asked for once it answers: its development
/// copies, the extensions left on, its layout, what it opens, what runs.
fn apply(inst: &Instance, ask: &Ask) -> Result<(), String> {
    let socket = inst.socket();
    // The window answers before its extensions are loaded: wait for them.
    let began = Instant::now();
    let listed = loop {
        match act(&socket, "EXT_LIST", &Value::Null) {
            Ok(listed) => break listed,
            Err(_) if began.elapsed() < READY => std::thread::sleep(Duration::from_millis(400)),
            Err(why) => return Err(format!("its extensions never loaded: {why}")),
        }
    };
    // Each step's failure is kept and the rest still done: a copy that
    // does not load leaves the window hidden, folded and on its document
    // all the same. All of them are what `ready` says.
    let mut failed: Vec<String> = Vec::new();
    let mut keep: Vec<String> = Vec::new();
    for dir in &ask.extensions {
        let folder = match dir.canonicalize() {
            Ok(folder) => folder,
            Err(e) => {
                failed.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        let out = act(&socket, "EXT_DEV", &json!({ "folder": folder }));
        if let Err(e) = &out {
            failed.push(format!("EXT_DEV {}: {e}", folder.display()));
        }
        // Kept on whatever happened: `--only` never turns it off.
        if let Some(id) = out
            .ok()
            .and_then(|o| o.get("id").and_then(Value::as_str).map(str::to_owned))
            .or_else(|| manifest_id(&folder))
        {
            keep.push(id);
        }
    }
    // The updater never runs in an instance; with `--only`/`--bare`,
    // nothing but what is named and the development copies.
    let all = listed
        .get("extensions")
        .and_then(Value::as_array)
        .or_else(|| listed.as_array())
        .cloned()
        .unwrap_or_default();
    for e in all {
        let Some(id) = e.get("id").and_then(Value::as_str) else {
            continue;
        };
        let on = ask.only.as_ref().map_or_else(
            || id != "updates",
            |only| only.iter().any(|o| o == id) || keep.iter().any(|k| k == id),
        );
        let loaded = e.get("loaded").and_then(Value::as_bool).unwrap_or(true);
        if !on && loaded {
            if let Err(e) = act(&socket, "EXT_DISABLE", &json!({ "id": id })) {
                failed.push(format!("EXT_DISABLE {id}: {e}"));
            }
        } else if on && !loaded {
            // Turned off by an earlier ask (it is kept): on again. One that
            // cannot run here stays off, and says why in EXT_LOGS.
            let _ = act(&socket, "EXT_ENABLE", &json!({ "id": id }));
        }
    }
    if let Some(layout) = &ask.layout
        && let Err(e) = use_layout(inst, &socket, layout)
    {
        failed.push(e);
    }
    if let Some(path) = &ask.open
        && let Err(e) = open(&socket, path, !ask.extensions.is_empty())
    {
        failed.push(e);
    }
    if let Err(e) = hide(inst, &socket, &ask.hide) {
        failed.push(e);
    }
    if let Err(e) = collapse(&socket, &ask.collapse) {
        failed.push(e);
    }
    for (id, args) in &ask.run {
        if let Err(e) = act(&socket, id, args) {
            failed.push(format!("{id}: {e}"));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

/// Whether `--layout`'s value names a layout file (`….json`, any case)
/// rather than a layout of the app's folder.
fn is_layout_file(layout: &str) -> bool {
    Path::new(layout)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
}

/// A layout file (`--layout <file>.json`): the layout's name (its file's
/// stem) and its text. `None` for a name of a layout the app has already.
fn layout_file(layout: &str) -> Result<Option<(String, String)>, String> {
    let path = Path::new(layout);
    if !is_layout_file(layout) {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("--layout {layout}: {e}"))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| check_name(s).is_ok())
        .ok_or_else(|| format!("--layout {layout}: its file's name is no layout name"))?;
    Ok(Some((name, text)))
}

/// Shows `layout`: a layout of the app's folder by name, or a layout file
/// copied into the instance's `layouts/` first (the app reads it on its
/// next poll, so its use waits for that).
fn use_layout(inst: &Instance, socket: &Path, layout: &str) -> Result<(), String> {
    let Some((name, text)) = layout_file(layout)? else {
        return act(socket, "HOME_LAYOUT_USE", &json!({ "name": layout }))
            .map(|_| ())
            .map_err(|e| format!("HOME_LAYOUT_USE {layout}: {e}"));
    };
    let dir = inst.app().join("layouts");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let into = dir.join(format!("{name}.json"));
    std::fs::write(&into, text).map_err(|e| format!("{}: {e}", into.display()))?;
    let began = Instant::now();
    loop {
        match act(socket, "HOME_LAYOUT_USE", &json!({ "name": name })) {
            Ok(_) => return Ok(()),
            Err(_) if began.elapsed() < Duration::from_secs(15) => {
                std::thread::sleep(Duration::from_millis(300));
            }
            Err(e) => {
                let why = std::fs::read_to_string(dir.join(format!("{name}.error")))
                    .map(|w| format!(" ({w})"))
                    .unwrap_or_default();
                return Err(format!("HOME_LAYOUT_USE {name} ({layout}): {e}{why}"));
            }
        }
    }
}

/// Folds `sides` away: the left as the sidebar folded to the rail (a
/// desk's left dock is its sidebar) and its dock collapsed; the right and
/// the bottom as their docks collapsed. The person can open them again.
fn collapse(socket: &Path, sides: &[String]) -> Result<(), String> {
    for side in sides {
        if side == "left" {
            act(
                socket,
                "COMMANDS_RUN",
                &json!({ "id": "SHELL_SIDEBAR_RAIL" }),
            )
            .map_err(|e| format!("SHELL_SIDEBAR_RAIL: {e}"))?;
        }
        act(
            socket,
            "WORKSPACE_DOCK_SET",
            &json!({ "side": side, "shown": "collapsed" }),
        )
        .map_err(|e| format!("WORKSPACE_DOCK_SET {side}: {e}"))?;
    }
    Ok(())
}

/// Takes away the parts in `parts`: the rail and the status bar as the
/// shell's slots hidden (`HOME_SLOT_SET`), a dock as hidden, strip and
/// all (`WORKSPACE_DOCK_SET`; the left also folds the sidebar to the
/// rail). The slots an earlier ask hid that this one does not come back.
fn hide(inst: &Instance, socket: &Path, parts: &[String]) -> Result<(), String> {
    let slot = |part: &str| match part {
        "rail" => Some("ribbon"),
        "status" => Some("status"),
        "now-playing" => Some("now_playing"),
        _ => None,
    };
    let record = inst.dir.join("hidden-slots");
    let before = std::fs::read_to_string(&record).unwrap_or_default();
    let mut changed = false;
    for gone in before.lines().filter(|s| !s.is_empty()) {
        if !parts.iter().any(|p| slot(p) == Some(gone)) {
            act(
                socket,
                "HOME_SLOT_SET",
                &json!({ "slot": gone, "clear": true }),
            )
            .map_err(|e| format!("HOME_SLOT_SET {gone}: {e}"))?;
            changed = true;
        }
    }
    // The slots first: a slot hidden or shown draws the shell again, and a
    // fold asked of it before that has settled is lost.
    let mut hidden = Vec::new();
    for name in parts.iter().filter_map(|p| slot(p)) {
        act(
            socket,
            "HOME_SLOT_SET",
            &json!({ "slot": name, "hidden": true }),
        )
        .map_err(|e| format!("HOME_SLOT_SET {name}: {e}"))?;
        hidden.push(name);
        changed = true;
    }
    std::fs::write(&record, hidden.join("\n")).map_err(|e| format!("{}: {e}", record.display()))?;
    let docks: Vec<&String> = parts.iter().filter(|p| slot(p).is_none()).collect();
    if changed && !docks.is_empty() {
        std::thread::sleep(Duration::from_millis(600));
    }
    for side in docks {
        if side == "left" {
            act(
                socket,
                "COMMANDS_RUN",
                &json!({ "id": "SHELL_SIDEBAR_RAIL" }),
            )
            .map_err(|e| format!("SHELL_SIDEBAR_RAIL: {e}"))?;
        }
        act(
            socket,
            "WORKSPACE_DOCK_SET",
            &json!({ "side": side, "shown": "hidden" }),
        )
        .map_err(|e| format!("WORKSPACE_DOCK_SET {side}: {e}"))?;
    }
    Ok(())
}

/// Opens `path` (a vault path): with the extension that claims it
/// (`EXT_OPEN`: a `.kf` chart), else as a note of the person's vault
/// (`@me`), beside the document on show (`WORKSPACE_OPEN`).
///
/// With development copies (`dev_copies`) the opener may be one of
/// theirs that is not there yet: a process copy's program says what it
/// opens once it runs. It is asked again until one claims it, for up to
/// [`OPENER_WAIT`], before the path opens as a note.
fn open(socket: &Path, path: &str, dev_copies: bool) -> Result<(), String> {
    let began = Instant::now();
    // A note is the vault's: nothing to wait for.
    let note = path.starts_with('@') || path.to_ascii_lowercase().ends_with(".md");
    let wait = dev_copies && !note;
    let why = loop {
        match act(socket, "EXT_OPEN", &json!({ "path": path })) {
            Ok(_) => return Ok(()),
            Err(_) if wait && began.elapsed() < OPENER_WAIT => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(why) => break why,
        }
    };
    let layout = act(socket, "WORKSPACE_LAYOUT", &Value::Null)?;
    let beside = layout
        .get("current_document")
        .and_then(Value::as_str)
        .or_else(|| layout.get("active").and_then(Value::as_str))
        .ok_or_else(|| format!("nothing opens {path} ({why}), and no pane to open it beside"))?
        .to_owned();
    let state = if path.starts_with('@') {
        path.to_owned()
    } else {
        format!("@me/{}", path.trim_start_matches('/'))
    };
    let title = Path::new(path)
        .file_stem()
        .map_or_else(|| path.to_owned(), |s| s.to_string_lossy().into_owned());
    act(
        socket,
        "WORKSPACE_OPEN",
        &json!({ "view": "vault.note", "state": state, "title": title, "at": { "beside": { "pane": beside } }, "focus": true }),
    )
    .map(|_| ())
    .map_err(|e| format!("opening {path}: {e}"))
}

/// An extension folder's id, from its `extension.toml`'s `id = "…"`.
fn manifest_id(folder: &Path) -> Option<String> {
    let text = std::fs::read_to_string(folder.join("extension.toml")).ok()?;
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "id").then(|| v.trim().trim_matches('"').to_owned())
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn a_working_folder_names_its_instance_and_two_checkouts_differ() {
        let a = default_name(Path::new("/work/architect"));
        let b = default_name(Path::new("/other/architect"));
        assert!(a.starts_with("architect-") && b.starts_with("architect-"));
        assert_ne!(a, b);
        assert_eq!(a, default_name(Path::new("/work/architect")));
        assert!(check_name(&a).is_ok());
        assert!(check_name("../x").is_err() && check_name("").is_err());
    }

    #[test]
    fn its_arguments_read_as_asked() {
        let ask = parse(&args(
            "--name kf --only navigator,keyflow --extension features/extension --open Charts/A.kf --layout focus --run EXT_LIST --run HOME_LAYOUT_USE={\"name\":\"x\"} --wait",
        ))
        .unwrap();
        assert_eq!(ask.name.as_deref(), Some("kf"));
        assert_eq!(
            ask.only,
            Some(vec!["navigator".to_owned(), "keyflow".to_owned()])
        );
        assert_eq!(ask.extensions, [PathBuf::from("features/extension")]);
        assert_eq!(ask.open.as_deref(), Some("Charts/A.kf"));
        assert_eq!(ask.run.len(), 2);
        assert_eq!(ask.run[1].1["name"], "x");
        assert!(ask.wait);
        assert_eq!(parse(&args("--bare")).unwrap().only, Some(Vec::new()));
        assert!(parse(&args("--vault a --seed b")).is_err());
        assert!(parse(&args("--run X={nope")).is_err());
        assert!(parse(&args("--what")).is_err());
        let collapse = parse(&args("--collapse left,right --collapse left"))
            .unwrap()
            .collapse;
        assert_eq!(collapse, ["left", "right"], "each side once");
        assert_eq!(parse(&args("--collapse all")).unwrap().collapse.len(), 3);
        assert!(parse(&args("--collapse top")).is_err());
        assert_eq!(
            parse(&args("--hide rail,status")).unwrap().hide,
            ["rail", "status"]
        );
        assert_eq!(parse(&args("--hide all")).unwrap().hide.len(), 6);
        assert!(parse(&args("--hide ribbon")).is_err());
        let env = parse(&args("--collapse right")).unwrap().to_env();
        assert!(env.iter().any(|(k, v)| *k == COLLAPSE_ENV && v == "right"));
    }

    #[test]
    fn each_only_adds_and_extensions_all_load() {
        let ask = parse(&args(
            "--only playhead --only daw,studio.mixer --only daw --extension a --extension b --extension c --extension d --extension e",
        ))
        .unwrap();
        assert_eq!(
            ask.only.as_deref().unwrap(),
            ["playhead", "daw", "studio.mixer"]
        );
        assert_eq!(ask.extensions.len(), 5);
        assert_eq!(
            parse(&args("--bare --only x")).unwrap().only.unwrap(),
            ["x"]
        );
    }

    #[test]
    fn a_layout_file_is_named_for_its_file_and_a_name_is_a_name() {
        let dir = std::env::temp_dir().join(format!("dev-layout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("setlist-window.json");
        std::fs::write(&file, "{\"name\":\"Setlist\"}").unwrap();
        let (name, text) = layout_file(&file.to_string_lossy()).unwrap().unwrap();
        assert_eq!(name, "setlist-window");
        assert!(text.contains("Setlist"));
        assert_eq!(layout_file("focus").unwrap(), None);
        assert!(layout_file(&dir.join("gone.json").to_string_lossy()).is_err());
        let env = parse(&args(&format!("--layout {}", file.display())))
            .unwrap()
            .to_env();
        assert!(
            env.iter()
                .any(|(k, v)| *k == LAYOUT_ENV && Path::new(v).is_absolute())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_instances_socket_is_short_and_its_own() {
        let a = Instance::new("one".to_owned(), "architect-desktop");
        let b = Instance::new("two".to_owned(), "architect-desktop");
        assert_ne!(a.socket(), b.socket());
        assert!(
            a.socket().as_os_str().len() < 100,
            "{}",
            a.socket().display()
        );
    }
}
