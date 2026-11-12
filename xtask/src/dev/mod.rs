//! `cargo xtask dev`: the whole stack on this machine (#541).
//!
//! `devenv up` runs each process through `cargo xtask dev run <process>`,
//! and so does the stack's test (`xtask/tests/dev_stack.rs`), so what a
//! process is given cannot drift between the two: the binary, its
//! environment, the fill and the guard are all decided here.
//!
//! - [`guard`]: before any process starts, every URL, issuer, bind and
//!   endpoint must be this machine or a private dev address and never
//!   production; the dev shell must carry no production credential.
//! - [`fill`]: a mirror of production on a credentialed machine, else the
//!   demo seed; decided when the auth server starts, said loudly, kept in
//!   `<state>/fill.json`.
//! - Everything under the state folder (`$DEVENV_STATE/stack`) is `0700`
//!   folders and `0600` files; `dev reset` removes it.
//!
//! Docs: `docs/content/guide/local-stack.md`.

pub mod fill;
pub mod guard;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{Subcommand, ValueEnum};
use serde::Deserialize;

use fill::{Decision, Fill, Mode};
use guard::Production;

/// The password every seeded and every mirrored account has on the local
/// auth server (`apps/auth-server/src/dev.rs` `DEV_PASSWORD`). Published on
/// purpose: it opens nothing anywhere else.
pub const DEV_PASSWORD: &str = "development-password";

/// The published secret the one-off recipes (`just auth-dev`, …) use; the
/// stack generates its own.
const PUBLISHED_SECRET: &str = "a-secret-at-least-32-bytes-long!!";

/// The node's vault id, and the folder its demo content comes from.
const NODE_VAULT: &str = "demo";
const DEMO_VAULT: &str = "apps/ui/demo-vault";
/// The demo seed's additions over the demo vault: a setlist, its charts and
/// a share link per audience kind.
const SEED_OVERLAY: &str = "scripts/dev-stack/seed-vault";

/// Where the stack's app keeps its sessions: a file in its own folder
/// (`ARCHITECT_SECRET_STORE`), never the platform keychain, which would ask
/// the person at the screen on every rebuild (an unsigned binary is a new
/// identity to macOS each time); `ARCHITECT_FORBID_KEYCHAIN` makes sure.
const APP_SECRETS: &str = "secrets.json";

/// Inherited variables no process of the stack sees: each is given exactly
/// what the stack decides, and nothing a shell happened to export (a peer,
/// an issuer, a bucket's keys, the admin token).
/// The native app's redirects at the issuer (`architect_accounts::browser`):
/// a loopback port of its choosing on a desk, its own scheme on iOS.
pub const NATIVE_LOOPBACK_REDIRECT: &str = "http://127.0.0.1/oauth/callback";
pub const NATIVE_SCHEME_REDIRECT: &str = "app.fasttrackstudio.architect:/oauth/callback";

const STRIPPED_PREFIXES: [&str; 4] = ["ARCHITECT_", "AUTH_", "AWS_", "MOCK_OAUTH_"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Process {
    MockOauth,
    Auth,
    Node,
    App,
    Web,
    Site,
}

impl Process {
    pub const ALL: [Self; 6] = [
        Self::MockOauth,
        Self::Auth,
        Self::Node,
        Self::App,
        Self::Web,
        Self::Site,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::MockOauth => "mock-oauth",
            Self::Auth => "auth",
            Self::Node => "node",
            Self::App => "app",
            Self::Web => "web",
            Self::Site => "site",
        }
    }
}

#[derive(Subcommand)]
pub enum DevAction {
    /// Start one process of the stack, as `devenv up` does: guard, fill,
    /// then exec its binary with its environment.
    Run { process: Process },
    /// Run the guard alone: what every process would be given, checked.
    Guard,
    /// Print the environment the stack gives a process (no secret is in
    /// it: secrets are files).
    Env { process: Process },
    /// What is running, the fill and why, the snapshot's age, the URLs to
    /// open, the development password.
    Status,
    /// Remove all of the stack's state: the mirror, the databases, the
    /// node's data, the app's folder and scratch vault. Refused while the
    /// stack runs.
    Reset,
    /// Mirror production again now (an administrator token required); the
    /// next start of the auth server loads it. Undoes `dev seed`.
    Refresh {
        /// Keep real addresses (redacted to `user-<id>@local.invalid` by
        /// default).
        #[arg(long)]
        real_emails: bool,
        /// A vault from production: not available yet (the docs say what
        /// is missing).
        #[arg(long)]
        vault: Option<String>,
    },
    /// Use the demo seed from the next start on, even on a credentialed
    /// machine; the auth database and the node's vault are seeded again.
    Seed,
    /// One auth server outside the stack (the `just auth-*` recipes): the
    /// same environment the stack's has, on a published secret.
    Auth {
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// A SQLite file, or `memory`.
        #[arg(long, default_value = "memory")]
        db: String,
        /// Fill an empty database with the demo cast.
        #[arg(long)]
        seed: bool,
        /// Load a snapshot (`auth-mirror`) into an empty database.
        #[arg(long)]
        import: Option<PathBuf>,
        /// Social sign-in against the mock provider on this port.
        #[arg(long)]
        mock_port: Option<u16>,
    },
}

/// Run `action`.
///
/// # Errors
///
/// What went wrong, for a person.
pub fn run_cli(action: DevAction) -> Result<(), String> {
    let env = |k: &str| std::env::var(k).ok();
    let stack = Stack::load(&env)?;
    match action {
        DevAction::Run { process } => stack.run(process, &env),
        DevAction::Guard => {
            stack.guard()?;
            println!("dev guard: every process is local, nothing names production");
            Ok(())
        }
        DevAction::Env { process } => {
            let fill = stack.read_fill().ok();
            for (k, v) in stack.env_for(process, fill.as_ref()) {
                println!("{k}={v}");
            }
            Ok(())
        }
        DevAction::Status => {
            stack.status();
            Ok(())
        }
        DevAction::Reset => stack.reset(),
        DevAction::Refresh { real_emails, vault } => stack.refresh(real_emails, vault, &env),
        DevAction::Seed => stack.seed(),
        DevAction::Auth {
            port,
            db,
            seed,
            import,
            mock_port,
        } => {
            let database_url = if db == "memory" {
                "sqlite::memory:".to_owned()
            } else {
                format!("sqlite://{db}?mode=rwc")
            };
            let vars = auth_env(&AuthRun {
                port,
                host: "localhost",
                database_url,
                secret: Secret::Published,
                mock_port,
                seed,
                import,
                web_port: None,
            });
            let refused = guard::check_settings(&vars, &stack.production);
            if !refused.is_empty() {
                return Err(refusal_message(&refused));
            }
            exec(stack.command(Process::Auth), &vars)
        }
    }
}

#[derive(Debug, Deserialize)]
struct StackFile {
    ports: Ports,
    mirror: MirrorConfig,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Ports {
    pub mock_oauth: u16,
    pub auth: u16,
    pub node: u16,
    pub web: u16,
    pub site: u16,
}

#[derive(Debug, Deserialize)]
struct MirrorConfig {
    from: String,
    #[serde(default)]
    node_org: String,
}

/// The stack's settings and where its state lives.
pub struct Stack {
    pub root: PathBuf,
    /// `$DEVENV_STATE`: the app's scratch vault is here, beside `stack/`.
    pub devenv_state: PathBuf,
    /// `$DEVENV_STATE/stack` (or `$DEV_STACK_STATE`).
    pub state: PathBuf,
    pub ports: Ports,
    pub mirror_from: String,
    pub node_org: String,
    pub production: Production,
}

impl Stack {
    /// `dev-stack.toml` at the repo root, then `DEV_STACK_*` over it.
    ///
    /// # Errors
    ///
    /// The file is missing or malformed, or an override is not a port.
    pub fn load(env: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let root = env("DEV_STACK_ROOT")
            .or_else(|| env("DEVENV_ROOT"))
            .map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                PathBuf::from,
            );
        let file = root.join("dev-stack.toml");
        let text = std::fs::read_to_string(&file)
            .map_err(|e| format!("reading {}: {e} (run from the repo root)", file.display()))?;
        let parsed: StackFile =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut ports = parsed.ports;
        for (name, port) in [
            ("MOCK_OAUTH", &mut ports.mock_oauth),
            ("AUTH", &mut ports.auth),
            ("NODE", &mut ports.node),
            ("WEB", &mut ports.web),
            ("SITE", &mut ports.site),
        ] {
            let var = format!("DEV_STACK_PORT_{name}");
            if let Some(value) = env(&var) {
                *port = value
                    .trim()
                    .parse()
                    .map_err(|_| format!("{var}=`{value}` is not a port"))?;
            }
        }
        let devenv_state =
            env("DEVENV_STATE").map_or_else(|| root.join(".devenv/state"), PathBuf::from);
        let state =
            env("DEV_STACK_STATE").map_or_else(|| devenv_state.join("stack"), PathBuf::from);
        Ok(Self {
            mirror_from: env("DEV_STACK_MIRROR_FROM").unwrap_or(parsed.mirror.from),
            node_org: parsed.mirror.node_org,
            production: Production::with_extra(test_production_hosts(env)),
            root,
            devenv_state,
            state,
            ports,
        })
    }

    fn url(port: u16) -> String {
        format!("http://127.0.0.1:{port}")
    }

    fn auth_dir(&self) -> PathBuf {
        self.state.join("auth")
    }
    fn snapshot(&self) -> PathBuf {
        self.auth_dir().join("snapshot.json")
    }
    fn fill_file(&self) -> PathBuf {
        self.state.join("fill.json")
    }
    fn node_dir(&self) -> PathBuf {
        self.state.join("node")
    }
    fn node_data(&self) -> PathBuf {
        self.node_dir().join("data")
    }
    fn node_vault(&self) -> PathBuf {
        self.node_data().join("vaults").join(NODE_VAULT)
    }
    fn session(&self) -> PathBuf {
        self.node_dir().join("session")
    }
    fn app_vault(&self) -> PathBuf {
        self.devenv_state.join("demo-vault")
    }
    fn forced_demo(&self) -> PathBuf {
        self.state.join("use-demo-seed")
    }
    fn real_emails(&self) -> PathBuf {
        self.state.join("mirror-real-emails")
    }

    /// The environment the stack gives `process`. `fill` names the node's
    /// account and org (a placeholder before the auth server decided).
    #[must_use]
    pub fn env_for(&self, process: Process, fill: Option<&Fill>) -> Vec<(String, String)> {
        let p = &self.ports;
        let auth = Self::url(p.auth);
        let pair = |k: &str, v: String| (k.to_owned(), v);
        match process {
            Process::MockOauth => vec![pair(
                "MOCK_OAUTH_BIND_ADDR",
                format!("127.0.0.1:{}", p.mock_oauth),
            )],
            Process::Auth => {
                let dir = self.auth_dir();
                auth_env(&AuthRun {
                    port: p.auth,
                    host: "127.0.0.1",
                    database_url: format!("sqlite://{}?mode=rwc", dir.join("auth.db").display()),
                    secret: Secret::File(dir.join("secret")),
                    mock_port: Some(p.mock_oauth),
                    seed: false,
                    import: None,
                    web_port: Some(p.web),
                })
            }
            Process::Node => vec![
                pair("ARCHITECT_NAME", "dev-node".to_owned()),
                pair("ARCHITECT_BIND", format!("127.0.0.1:{}", p.node)),
                pair("ARCHITECT_DATA_DIR", self.node_data().display().to_string()),
                pair(
                    "ARCHITECT_VAULTS",
                    format!("{NODE_VAULT}=vaults/{NODE_VAULT}"),
                ),
                pair("ARCHITECT_ISSUER", auth),
                pair(
                    "ARCHITECT_SESSION_FILE",
                    self.session().display().to_string(),
                ),
                pair(
                    "ARCHITECT_HOST_ORG",
                    fill.map_or_else(|| fill::DEMO_NODE_ORG.to_owned(), |f| f.node_org.clone()),
                ),
                // No relays and no lookup: it cannot find or be found by a
                // peer anywhere but this machine.
                pair("ARCHITECT_IROH", "local".to_owned()),
                pair(
                    "ARCHITECT_FILES_DIR",
                    self.node_dir().join("files").display().to_string(),
                ),
                pair("ARCHITECT_FILES_HOLD", "true".to_owned()),
            ],
            Process::App => vec![
                pair("ARCHITECT_AUTH_ISSUER", auth),
                pair("ARCHITECT_ORG_HOST", Self::url(p.node)),
                pair(
                    "ARCHITECT_SECRET_STORE",
                    format!(
                        "file:{}",
                        self.state.join("app").join(APP_SECRETS).display()
                    ),
                ),
                pair("ARCHITECT_FORBID_KEYCHAIN", "1".to_owned()),
                pair(
                    "ARCHITECT_CONFIG_DIR",
                    self.state.join("app").display().to_string(),
                ),
                pair("ARCHITECT_VAULT", self.app_vault().display().to_string()),
                pair(
                    "ARCHITECT_VAULT_SEED",
                    self.root.join(DEMO_VAULT).display().to_string(),
                ),
            ],
            Process::Web => vec![pair("ARCHITECT_WEB_URL", Self::url(p.web))],
            Process::Site => Vec::new(),
        }
    }

    /// Refuse unless every process's settings are local and the shell
    /// carries no production credential.
    ///
    /// # Errors
    ///
    /// Every refusal, with what to do.
    pub fn guard(&self) -> Result<(), String> {
        let inherited: Vec<(String, String)> = std::env::vars().collect();
        let mut refused =
            guard::check_inherited(inherited.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let fill = self.read_fill().ok();
        for process in Process::ALL {
            refused.extend(guard::check_settings(
                &self.env_for(process, fill.as_ref()),
                &self.production,
            ));
        }
        if refused.is_empty() {
            Ok(())
        } else {
            Err(refusal_message(&refused))
        }
    }

    fn command(&self, process: Process) -> Command {
        let bin = |var: &str| std::env::var(var).ok().filter(|v| !v.is_empty());
        let cargo = |args: &[&str]| {
            let mut c = Command::new("cargo");
            c.args(["run", "-q"]).args(args).current_dir(&self.root);
            c
        };
        match process {
            Process::MockOauth => bin("DEV_STACK_BIN_MOCK_OAUTH")
                .map_or_else(|| cargo(&["-p", "mock-oauth"]), Command::new),
            Process::Auth => bin("DEV_STACK_BIN_AUTH")
                .map_or_else(|| cargo(&["-p", "auth-server"]), Command::new),
            Process::Node => {
                let mut c = bin("DEV_STACK_BIN_NODE").map_or_else(
                    || {
                        cargo(&[
                            "-p",
                            "architect-desktop",
                            "--no-default-features",
                            "--features",
                            "vox",
                            "--bin",
                            "architect-desktop",
                            "--",
                        ])
                    },
                    Command::new,
                );
                c.arg("serve");
                c
            }
            Process::App => {
                let mut c = Command::new("dx");
                c.args([
                    "serve",
                    "--platform",
                    "desktop",
                    "--renderer",
                    "native",
                    "--hot-patch",
                    "false",
                ])
                .current_dir(self.root.join("apps/desktop"));
                c
            }
            Process::Web => {
                let mut c = Command::new("nix");
                c.args(["run", "nixpkgs#static-web-server", "--", "--config-file"])
                    .arg(self.root.join("apps/web/sws.toml"))
                    .arg("--root")
                    .arg(self.state.join("web"))
                    .args(["--host", "127.0.0.1", "--port"])
                    .arg(self.ports.web.to_string());
                c
            }
            Process::Site => {
                let mut c = Command::new("nix");
                c.args([
                    "shell",
                    "nixpkgs#bun",
                    "nixpkgs#nodejs_22",
                    "-c",
                    "bash",
                    "-c",
                ])
                .arg(format!(
                    "bun install --frozen-lockfile && exec bun x vite dev --host 127.0.0.1 --port {}",
                    self.ports.site
                ))
                .current_dir(self.root.join("apps/site"));
                c
            }
        }
    }

    /// Start `process`: guard, prepare its state, exec it.
    ///
    /// # Errors
    ///
    /// The guard refused, or a step before the exec failed.
    pub fn run(
        &self,
        process: Process,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(), String> {
        self.guard()?;
        private_dir(&self.state)?;
        let fill = match process {
            Process::Auth => Some(self.prepare_auth(env)?),
            Process::Node => Some(self.prepare_node()?),
            _ => self.read_fill().ok(),
        };
        match process {
            Process::App => private_dir(&self.state.join("app"))?,
            Process::Web => self.prepare_web()?,
            _ => {}
        }
        let mut vars = self.env_for(process, fill.as_ref());
        if process == Process::Auth {
            let fill = fill.as_ref().ok_or("no fill")?;
            vars.extend(self.auth_fill_env(fill));
        }
        exec(self.command(process), &vars)
    }

    /// The auth database's seed or import, when it is new.
    fn auth_fill_env(&self, fill: &Fill) -> Vec<(String, String)> {
        let db = self.auth_dir().join("auth.db");
        let fresh = !db.exists();
        let mut vars = Vec::new();
        if fill.mode == Mode::Demo {
            // A no-op on a database already seeded.
            vars.push(("AUTH_DEV_SEED".to_owned(), "1".to_owned()));
        } else if fresh {
            vars.push((
                "AUTH_IMPORT_SNAPSHOT".to_owned(),
                self.snapshot().display().to_string(),
            ));
        }
        vars
    }

    /// Decide the fill, write what it needs, say it.
    fn prepare_auth(&self, env: &dyn Fn(&str) -> Option<String>) -> Result<Fill, String> {
        let dir = self.auth_dir();
        private_dir(&dir)?;
        let secret = dir.join("secret");
        if !secret.exists() {
            write_private(&secret, random_hex(32)?.as_bytes())?;
        }
        let redact = !self.real_emails().exists();
        let decision = fill::decide(
            self.forced_demo().exists(),
            fill::admin_token(env),
            self.snapshot().exists(),
            |token| {
                crate::mirror::fetch(
                    &self.mirror_from,
                    &token.value,
                    redact,
                    Some(fill::MIRROR_TIMEOUT),
                )
            },
        );
        let (mode, why) = match decision {
            Decision::Mirror(bytes, why) => {
                write_private(&self.snapshot(), &bytes)?;
                (Mode::Mirror, why)
            }
            Decision::LastSnapshot(why) => (Mode::LastSnapshot, why),
            Decision::Demo(why) => (Mode::Demo, why),
        };
        let mut fill = Fill {
            mode,
            why,
            at: now(),
            snapshot_taken_at: None,
            emails_redacted: None,
            node_email: fill::DEMO_NODE_EMAIL.to_owned(),
            node_org: fill::DEMO_NODE_ORG.to_owned(),
            db_source: "demo".to_owned(),
        };
        if mode != Mode::Demo {
            match fill::SnapshotView::read(&self.snapshot())
                .and_then(|s| s.node_account(&self.node_org).map(|a| (s, a)))
            {
                Ok((snap, (email, org))) => {
                    fill.db_source = format!("snapshot:{}", snap.taken_at);
                    fill.snapshot_taken_at = Some(snap.taken_at);
                    fill.emails_redacted = Some(snap.emails_redacted);
                    fill.node_email = email;
                    fill.node_org = org;
                }
                Err(problem) => {
                    fill.why = format!("{}; then {problem}, so the demo seed", fill.why);
                    fill.mode = Mode::Demo;
                }
            }
        }
        // A database made from another source starts again; the node's
        // vault and session belong to the org it was made for.
        let previous = self.read_fill().ok();
        if previous
            .as_ref()
            .is_none_or(|p| p.db_source != fill.db_source)
        {
            remove_auth_db(&dir)?;
            remove_if_exists(&self.session())?;
        }
        if previous
            .as_ref()
            .is_some_and(|p| p.node_org != fill.node_org)
        {
            remove_if_exists(&self.node_data())?;
        }
        write_private(
            &self.fill_file(),
            serde_json::to_string_pretty(&fill)
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )?;
        print_banner(&fill, self);
        Ok(fill)
    }

    /// The node's vault seeded, its session signed in.
    fn prepare_node(&self) -> Result<Fill, String> {
        let fill = self
            .read_fill()
            .map_err(|e| format!("{e}: start the auth server first (it decides the fill)"))?;
        private_dir(&self.node_dir())?;
        private_dir(&self.node_data())?;
        private_dir(&self.node_dir().join("files"))?;
        if !self.node_vault().exists() {
            seed_vault(&self.root, &self.node_vault())?;
        }
        let token = sign_in(
            &Self::url(self.ports.auth),
            &fill.node_email,
            Duration::from_secs(90),
        )?;
        write_private(&self.session(), token.as_bytes())?;
        println!(
            "dev node: signed in as {} at {}, hosting org `{}` (iroh: local only)",
            fill.node_email,
            Self::url(self.ports.auth),
            fill.node_org
        );
        Ok(fill)
    }

    fn prepare_web(&self) -> Result<(), String> {
        let out = self.state.join("web");
        let status = Command::new("bash")
            .arg(self.root.join("scripts/web-bundle.sh"))
            .arg(&out)
            .env("ARCHITECT_WEB_URL", Self::url(self.ports.web))
            .current_dir(&self.root)
            .status()
            .map_err(|e| format!("running scripts/web-bundle.sh: {e}"))?;
        if !status.success() {
            return Err(format!("scripts/web-bundle.sh: {status}"));
        }
        // The bundle's config names production's issuer: the local one here.
        let config = serde_json::json!({
            "issuer": Self::url(self.ports.auth),
            "client_id": "architect",
        });
        std::fs::write(out.join("app/config.json"), config.to_string())
            .map_err(|e| format!("writing the web app's config.json: {e}"))
    }

    fn read_fill(&self) -> Result<Fill, String> {
        let text = std::fs::read_to_string(self.fill_file())
            .map_err(|_| format!("no {}", self.fill_file().display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", self.fill_file().display()))
    }

    fn running(&self) -> Vec<(Process, u16)> {
        let p = self.ports;
        [
            (Process::MockOauth, p.mock_oauth),
            (Process::Auth, p.auth),
            (Process::Node, p.node),
            (Process::Web, p.web),
            (Process::Site, p.site),
        ]
        .into_iter()
        .filter(|(_, port)| answers(*port))
        .collect()
    }

    fn status(&self) {
        let running = self.running();
        println!("The local stack ({})", self.state.display());
        for (process, port) in [
            (Process::MockOauth, self.ports.mock_oauth),
            (Process::Auth, self.ports.auth),
            (Process::Node, self.ports.node),
            (Process::Web, self.ports.web),
            (Process::Site, self.ports.site),
        ] {
            let up = running.iter().any(|(p, _)| *p == process);
            println!(
                "  {:<11} {:<26} {}",
                process.name(),
                Self::url(port),
                if up { "running" } else { "not running" }
            );
        }
        match self.read_fill() {
            Ok(fill) => {
                println!("\nFilled with {}: {}", fill.mode.label(), fill.why);
                if let Some(taken) = &fill.snapshot_taken_at {
                    println!(
                        "  snapshot taken {taken}{} ({} old at the decision)",
                        if fill.emails_redacted == Some(true) {
                            ", addresses redacted"
                        } else {
                            ", real addresses"
                        },
                        age(fill.at)
                    );
                }
                println!(
                    "  the node signs in as {} and hosts `{}`",
                    fill.node_email, fill.node_org
                );
            }
            Err(_) => println!("\nNot filled yet: the auth server decides when it starts."),
        }
        println!(
            "\nSign in anywhere on the stack with the development password: {DEV_PASSWORD}\n\
             Demo accounts: ada@local.test (owner of acme, admin), grace@local.test (acme admin, \
             owner of indie), alan@local.test (indie member)."
        );
        println!(
            "\nOpen:\n  the auth server's pages   {auth}/login\n  the node's status         \
             {node}/node/status (POST)\n  share links               {node}/s/<code>/ \
             (docs/content/guide/local-stack.md lists the seeded ones)\n  the app's Action List     \
             ARCHITECT_CONFIG_DIR={app} just app actions list",
            auth = Self::url(self.ports.auth),
            node = Self::url(self.ports.node),
            app = self.state.join("app").display(),
        );
    }

    fn refuse_while_running(&self, what: &str) -> Result<(), String> {
        let running = self.running();
        if running.is_empty() {
            return Ok(());
        }
        let names: Vec<_> = running
            .iter()
            .map(|(p, port)| format!("{} (:{port})", p.name()))
            .collect();
        Err(format!(
            "the stack is running ({}): stop `devenv up` before {what}",
            names.join(", ")
        ))
    }

    fn reset(&self) -> Result<(), String> {
        self.refuse_while_running("a reset")?;
        remove_if_exists(&self.state)?;
        remove_if_exists(&self.app_vault())?;
        println!(
            "dev reset: removed {} and {}",
            self.state.display(),
            self.app_vault().display()
        );
        Ok(())
    }

    fn refresh(
        &self,
        real_emails: bool,
        vault: Option<String>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(), String> {
        if let Some(vault) = vault {
            return Err(format!(
                "a vault from production (`{vault}`) is not available yet: production has no \
                 read-only export of a vault to import from (docs/content/guide/local-stack.md, \
                 \"Vaults from production\"). The node holds the demo vault."
            ));
        }
        let token = fill::admin_token(env)?.ok_or_else(|| {
            format!(
                "this machine is not credentialed: set {} or {} (docs/content/guide/local-stack.md)",
                fill::TOKEN_VAR,
                fill::TOKEN_FILE_VAR
            )
        })?;
        private_dir(&self.state)?;
        private_dir(&self.auth_dir())?;
        let snapshot = crate::mirror::fetch(
            &self.mirror_from,
            &token.value,
            !real_emails,
            Some(fill::MIRROR_TIMEOUT),
        )?;
        write_private(&self.snapshot(), &snapshot)?;
        if real_emails {
            write_private(&self.real_emails(), b"")?;
        } else {
            remove_if_exists(&self.real_emails())?;
        }
        remove_if_exists(&self.forced_demo())?;
        println!(
            "dev refresh: mirrored {} ({} bytes{}); the auth server loads a fresh mirror when it \
             next starts (restart `devenv up`)",
            self.mirror_from,
            snapshot.len(),
            if real_emails {
                ", real addresses"
            } else {
                ", addresses redacted"
            }
        );
        Ok(())
    }

    fn seed(&self) -> Result<(), String> {
        self.refuse_while_running("re-seeding")?;
        private_dir(&self.state)?;
        write_private(&self.forced_demo(), b"")?;
        remove_auth_db(&self.auth_dir())?;
        remove_if_exists(&self.node_data())?;
        remove_if_exists(&self.session())?;
        remove_if_exists(&self.fill_file())?;
        println!(
            "dev seed: the next `devenv up` starts from the demo seed (`just dev-refresh` mirrors again)"
        );
        Ok(())
    }
}

/// One auth server's settings (the stack's, or a recipe's).
pub struct AuthRun {
    pub port: u16,
    /// `127.0.0.1` for the stack, `localhost` for the recipes (the
    /// callbacks the hosted pages were written against).
    pub host: &'static str,
    pub database_url: String,
    pub secret: Secret,
    pub mock_port: Option<u16>,
    pub seed: bool,
    pub import: Option<PathBuf>,
    /// The web app's origin, registered as the `architect` client's redirect.
    pub web_port: Option<u16>,
}

pub enum Secret {
    /// The recipes' published development secret.
    Published,
    /// A file the stack generated (never production's).
    File(PathBuf),
}

/// The environment for one auth server: the one place it is written.
#[must_use]
pub fn auth_env(run: &AuthRun) -> Vec<(String, String)> {
    let base = format!("http://{}:{}", run.host, run.port);
    let mut vars = vec![
        (
            "AUTH_BIND_ADDR".to_owned(),
            format!("127.0.0.1:{}", run.port),
        ),
        ("AUTH_DATABASE_URL".to_owned(), run.database_url.clone()),
        ("AUTH_BASE_URL".to_owned(), base),
    ];
    match &run.secret {
        Secret::Published => vars.push(("AUTH_SECRET".to_owned(), PUBLISHED_SECRET.to_owned())),
        Secret::File(path) => {
            vars.push(("AUTH_SECRET_FILE".to_owned(), path.display().to_string()));
        }
    }
    if let Some(mock) = run.mock_port {
        for (k, v) in [
            (
                "AUTH_SOCIAL_MOCK_URL",
                format!("http://{}:{mock}", run.host),
            ),
            ("AUTH_GITHUB_CLIENT_ID", "mock-github".to_owned()),
            ("AUTH_GITHUB_CLIENT_SECRET", "mock-github-secret".to_owned()),
            ("AUTH_GOOGLE_CLIENT_ID", "mock-google".to_owned()),
            ("AUTH_GOOGLE_CLIENT_SECRET", "mock-google-secret".to_owned()),
            ("AUTH_TONE3000_CLIENT_ID", "mock-tone3000".to_owned()),
        ] {
            vars.push((k.to_owned(), v));
        }
    }
    // The Architect apps sign in as the public client `architect`, as they
    // do at production's issuer: the native app in the system browser,
    // back on a loopback port (any, RFC 8252 §7.3) or its own scheme, its
    // fresh token traded for a session (`issue_sessions`); the web app on
    // its callback, when it runs.
    let mut redirects = vec![
        NATIVE_LOOPBACK_REDIRECT.to_owned(),
        NATIVE_SCHEME_REDIRECT.to_owned(),
    ];
    if let Some(web) = run.web_port {
        redirects.push(format!("http://127.0.0.1:{web}/auth/callback"));
    }
    let clients = serde_json::json!([{
        "client_id": "architect",
        "name": "Architect (local)",
        "redirect_uris": redirects,
        "scopes": ["openid", "profile", "email", "offline_access"],
        "public_client": true,
        "skip_consent": true,
        "issue_sessions": true,
    }]);
    vars.push(("AUTH_OIDC_CLIENTS".to_owned(), clients.to_string()));
    if run.seed {
        vars.push(("AUTH_DEV_SEED".to_owned(), "1".to_owned()));
    }
    if let Some(import) = &run.import {
        vars.push((
            "AUTH_IMPORT_SNAPSHOT".to_owned(),
            import.display().to_string(),
        ));
    }
    vars
}

/// The production hosts a test adds (`DEV_STACK_TEST_PRODUCTION_HOSTS`, a
/// loopback port standing in for production): honoured only in a build
/// with the `stack-test` feature, which nothing but the stack's test turns on.
fn test_production_hosts(env: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    if cfg!(feature = "stack-test") {
        env("DEV_STACK_TEST_PRODUCTION_HOSTS")
            .map(|v| {
                v.split(',')
                    .map(|h| h.trim().to_owned())
                    .filter(|h| !h.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}

fn refusal_message(refused: &[guard::Refusal]) -> String {
    let mut out = String::from("the dev stack's guard refused to start anything:\n");
    for r in refused {
        let _ = writeln!(out, "  - {r}");
    }
    out.push_str(
        "The stack talks to this machine only (docs/content/guide/local-stack.md, \"Isolation\").",
    );
    out
}

/// Replace this process with `command`, given `vars` and none of the
/// inherited variables the stack owns.
fn exec(mut command: Command, vars: &[(String, String)]) -> Result<(), String> {
    use std::os::unix::process::CommandExt as _;
    for (key, _) in std::env::vars() {
        if STRIPPED_PREFIXES.iter().any(|p| key.starts_with(p)) {
            command.env_remove(&key);
        }
    }
    command.envs(vars.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let err = command.exec();
    Err(format!(
        "starting {}: {err}",
        command.get_program().to_string_lossy()
    ))
}

fn print_banner(fill: &Fill, stack: &Stack) {
    let rule = "=".repeat(72);
    println!("{rule}");
    println!(
        "  THE LOCAL STACK IS FILLED WITH {}",
        fill.mode.label().to_uppercase()
    );
    println!("  why: {}", fill.why);
    if let Some(taken) = &fill.snapshot_taken_at {
        println!(
            "  snapshot taken {taken}, {}; every account's password is `{DEV_PASSWORD}`",
            if fill.emails_redacted == Some(true) {
                "addresses redacted"
            } else {
                "REAL addresses"
            }
        );
    }
    println!(
        "  node: {} hosting `{}`; state: {}",
        fill.node_email,
        fill.node_org,
        stack.state.display()
    );
    println!("  nothing here can reach production: docs/content/guide/local-stack.md");
    println!("{rule}");
}

/// Sign in at `auth` as `email` with the development password, retrying
/// while the server comes up; the session token.
fn sign_in(auth: &str, email: &str, within: Duration) -> Result<String, String> {
    use std::io::Write as _;
    use std::process::Stdio;
    let body = serde_json::json!({"input": {"email": email, "password": DEV_PASSWORD}}).to_string();
    let deadline = Instant::now()
        .checked_add(within)
        .unwrap_or_else(Instant::now);
    loop {
        let mut child = Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--fail-with-body",
                "--max-time",
                "10",
            ])
            .args([
                "--header",
                "content-type: application/json",
                "--data-binary",
                "@-",
            ])
            .arg(format!("{auth}/auth/sign-in/email"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("running curl: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(body.as_bytes());
        }
        let out = child
            .wait_with_output()
            .map_err(|e| format!("running curl: {e}"))?;
        let token = serde_json::from_slice::<serde_json::Value>(&out.stdout)
            .ok()
            .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(str::to_owned));
        match token {
            Some(token) if out.status.success() => return Ok(token),
            _ if Instant::now() >= deadline => {
                return Err(format!(
                    "signing in to {auth} as {email} failed: {}{}",
                    String::from_utf8_lossy(&out.stderr).trim(),
                    String::from_utf8_lossy(&out.stdout)
                        .chars()
                        .take(200)
                        .collect::<String>()
                ));
            }
            _ => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

/// The demo vault, then the seed's additions, into `to`.
fn seed_vault(root: &Path, to: &Path) -> Result<(), String> {
    copy_tree(&root.join(DEMO_VAULT), to)?;
    let empty = root.join(DEMO_VAULT).join(".empty-folders");
    if let Ok(list) = std::fs::read_to_string(&empty) {
        for folder in list.lines().filter(|l| !l.trim().is_empty()) {
            std::fs::create_dir_all(to.join(folder.trim()))
                .map_err(|e| format!("creating {folder}: {e}"))?;
        }
    }
    let _ = std::fs::remove_file(to.join(".empty-folders"));
    copy_tree(&root.join(SEED_OVERLAY), to)
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("creating {}: {e}", to.display()))?;
    let entries =
        std::fs::read_dir(from).map_err(|e| format!("reading {}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)
                .map_err(|e| format!("copying {}: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// A folder only its owner can enter (made, or tightened if it was there).
fn private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("creating {}: {e}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("tightening {}: {e}", dir.display()))
}

/// A file only its owner can read.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    if let Some(parent) = path.parent() {
        private_dir(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("writing {}: {e}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("tightening {}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("removing {}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

fn remove_auth_db(dir: &Path) -> Result<(), String> {
    for name in ["auth.db", "auth.db-wal", "auth.db-shm", "auth.db-journal"] {
        remove_if_exists(&dir.join(name))?;
    }
    Ok(())
}

fn random_hex(bytes: usize) -> Result<String, String> {
    use std::io::Read as _;
    let mut buf = vec![0_u8; bytes];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| format!("reading /dev/urandom: {e}"))?;
    Ok(buf.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    }))
}

fn answers(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(300),
    )
    .is_ok()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn age(at: u64) -> String {
    let secs = now().saturating_sub(at);
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn stack() -> Stack {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned();
        let root_s = root.display().to_string();
        let env = move |k: &str| match k {
            "DEV_STACK_ROOT" => Some(root_s.clone()),
            "DEVENV_STATE" => Some("/tmp/devenv-state".to_owned()),
            _ => None,
        };
        Stack::load(&env).unwrap()
    }

    #[test]
    fn the_stack_as_configured_passes_its_own_guard() {
        let stack = stack();
        for process in Process::ALL {
            let refused = guard::check_settings(&stack.env_for(process, None), &stack.production);
            assert!(refused.is_empty(), "{process:?}: {refused:?}");
        }
    }

    #[test]
    fn the_stacks_app_never_opens_the_platform_keychain() {
        let stack = stack();
        let env = stack.env_for(Process::App, None);
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("ARCHITECT_FORBID_KEYCHAIN"), Some("1"));
        assert!(get("ARCHITECT_KEYCHAIN_SERVICE").is_none());
        let store = get("ARCHITECT_SECRET_STORE").unwrap();
        let path = store.strip_prefix("file:").unwrap();
        assert!(
            std::path::Path::new(path).starts_with(&stack.state),
            "the sessions live in the stack's own state: {path}"
        );
    }

    #[test]
    fn the_node_runs_local_iroh_and_signs_in_locally() {
        let env = stack().env_for(Process::Node, None);
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("ARCHITECT_IROH"), Some("local"));
        assert!(
            get("ARCHITECT_ISSUER")
                .unwrap()
                .starts_with("http://127.0.0.1:")
        );
        assert!(get("ARCHITECT_PEERS").is_none() && get("ARCHITECT_ENROL_AT").is_none());
        assert!(
            env.iter()
                .all(|(k, _)| !k.starts_with("ARCHITECT_BLOB_S3_"))
        );
    }

    #[test]
    fn the_stacks_auth_server_has_its_own_secret_file() {
        let env = stack().env_for(Process::Auth, None);
        assert!(
            env.iter()
                .any(|(k, v)| k == "AUTH_SECRET_FILE" && v.ends_with("stack/auth/secret"))
        );
        assert!(env.iter().all(|(k, _)| k != "AUTH_SECRET"));
    }

    #[test]
    fn a_port_override_that_points_at_production_is_refused() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .display()
            .to_string();
        let env = move |k: &str| match k {
            "DEV_STACK_ROOT" => Some(root.clone()),
            "DEV_STACK_PORT_AUTH" => Some("7999".to_owned()),
            _ => None,
        };
        let mut stack = Stack::load(&env).unwrap();
        stack.production = Production::with_extra(vec!["127.0.0.1:7999".to_owned()]);
        let refused: Vec<_> = Process::ALL
            .into_iter()
            .flat_map(|p| guard::check_settings(&stack.env_for(p, None), &stack.production))
            .collect();
        assert!(
            refused.iter().any(|r| r.key == "ARCHITECT_ISSUER"),
            "{refused:?}"
        );
        assert!(
            refused.iter().any(|r| r.key == "AUTH_BIND_ADDR"),
            "{refused:?}"
        );
    }

    #[test]
    fn state_files_are_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("xtask-private-{}", std::process::id()));
        let file = dir.join("a/b/secret");
        write_private(&file, b"x").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(&dir.join("a/b")), 0o700);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
