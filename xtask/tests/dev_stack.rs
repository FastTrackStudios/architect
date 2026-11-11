//! The whole stack on this machine (#541), started the way `devenv up`
//! starts it: each process through `cargo xtask dev run <process>` (this
//! crate's own binary), with the binaries `DEV_STACK_BIN_*` names, on free
//! ports in a temporary state folder. Nothing here needs devenv.
//!
//! - The demo seed: the node signs in to the local issuer with its iroh
//!   endpoint on loopback alone and no relay, a seeded member sees the
//!   org's vault through the node, and each seeded share link lets in
//!   exactly its audience.
//! - A credentialed machine: a second auth server plays production on
//!   another loopback port (the guard told so by
//!   `DEV_STACK_TEST_PRODUCTION_HOSTS`, which only this feature's build
//!   honours); the stack mirrors it, every account signs in locally with
//!   the development password and not its own, the snapshot is private and
//!   names no real address, and a session minted on one server is refused
//!   by the other.
//! - The guard refuses a stack pointed at production, and a shell carrying
//!   a production secret.
//!
//! Every wait is bounded; every process is killed (its whole group) when
//! the test ends, failed or not. A failed test leaves its folder (logs in
//! `logs/`) under the temp dir; a passing one removes it.
//!
//! Run: `cargo xtask feature-tests --run --filter dev-stack`, or
//! `cargo nextest run -p xtask --features stack-test --test dev_stack`.
//! The binaries are built here unless `DEV_STACK_BIN_AUTH`,
//! `DEV_STACK_BIN_NODE` and `DEV_STACK_BIN_MOCK_OAUTH` name them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const DEV_PASSWORD: &str = "development-password";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

struct Bins {
    auth: PathBuf,
    node: PathBuf,
    mock: PathBuf,
}

/// The binaries the stack's processes run: named, or built here once.
fn bins() -> &'static Bins {
    static BINS: OnceLock<Bins> = OnceLock::new();
    BINS.get_or_init(|| {
        let named = |var: &str| std::env::var_os(var).map(PathBuf::from);
        if let (Some(auth), Some(node), Some(mock)) = (
            named("DEV_STACK_BIN_AUTH"),
            named("DEV_STACK_BIN_NODE"),
            named("DEV_STACK_BIN_MOCK_OAUTH"),
        ) {
            return Bins { auth, node, mock };
        }
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let build = |args: &[&str]| {
            let status = Command::new(&cargo)
                .arg("build")
                .arg("-q")
                .args(args)
                .current_dir(root())
                .status()
                .unwrap();
            assert!(status.success(), "cargo build {args:?}: {status}");
        };
        build(&[
            "-p",
            "auth-server",
            "--bin",
            "auth-server",
            "-p",
            "mock-oauth",
            "--bin",
            "mock-oauth",
        ]);
        build(&[
            "-p",
            "architect-desktop",
            "--no-default-features",
            "--features",
            "vox",
            "--bin",
            "architect-desktop",
        ]);
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(|| root().join("target"), PathBuf::from)
            .join("debug");
        Bins {
            auth: target.join("auth-server"),
            node: target.join("architect-desktop"),
            mock: target.join("mock-oauth"),
        }
    })
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Processes started by a test, each in its own process group: killed,
/// group and all, when this is dropped (a failed assert included).
struct Procs {
    children: Vec<(String, Child)>,
    logs: PathBuf,
}

impl Procs {
    fn new(logs: PathBuf) -> Self {
        std::fs::create_dir_all(&logs).unwrap();
        Self {
            children: Vec::new(),
            logs,
        }
    }

    fn spawn(&mut self, name: &str, mut command: Command) {
        let log = std::fs::File::create(self.log(name)).unwrap();
        let child = command
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .process_group(0)
            .spawn()
            .unwrap_or_else(|e| panic!("starting {name}: {e}"));
        self.children.push((name.to_owned(), child));
    }

    fn log(&self, name: &str) -> PathBuf {
        self.logs.join(format!("{name}.log"))
    }

    fn tail(&self, name: &str) -> String {
        let text = std::fs::read_to_string(self.log(name)).unwrap_or_default();
        let lines: Vec<_> = text.lines().collect();
        lines[lines.len().saturating_sub(30)..].join("\n")
    }

    fn whole_log(&self, name: &str) -> String {
        std::fs::read_to_string(self.log(name)).unwrap_or_default()
    }

    /// Wait until `url` answers `want` (any status when `None`), `name`
    /// alive meanwhile; at most `within`.
    fn wait(&mut self, name: &str, url: &str, want: Option<u16>, within: Duration) {
        let deadline = Instant::now() + within;
        loop {
            let (status, _) = http("GET", url, None, None);
            if status != 0 && want.is_none_or(|w| w == status) {
                return;
            }
            if let Some((_, child)) = self.children.iter_mut().find(|(n, _)| n == name)
                && let Ok(Some(exit)) = child.try_wait()
            {
                panic!("{name} exited ({exit}):\n{}", self.tail(name));
            }
            assert!(
                Instant::now() < deadline,
                "{name} did not answer {url} within {within:?}:\n{}",
                self.tail(name)
            );
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

impl Drop for Procs {
    fn drop(&mut self) {
        for (_, child) in &mut self.children {
            let _ = Command::new("kill")
                .args(["-KILL", &format!("-{}", child.id())])
                .stderr(Stdio::null())
                .status();
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// `method url` with a bearer and a JSON body; the status (0: no answer)
/// and the body. curl, bounded; the bearer on its stdin, as the stack
/// itself passes one.
fn http(method: &str, url: &str, bearer: Option<&str>, body: Option<&str>) -> (u16, String) {
    let mut curl = Command::new("curl");
    curl.args(["-s", "--max-time", "20", "-X", method])
        .args(["-o", "-", "-w", "\n%{http_code}"]);
    if bearer.is_some() {
        curl.args(["--header", "@-"]);
    }
    if let Some(body) = body {
        curl.args(["--header", "content-type: application/json", "--data", body]);
    }
    let mut child = curl
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        if let Some(token) = bearer {
            stdin
                .write_all(format!("authorization: Bearer {token}\n").as_bytes())
                .unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", "0"));
    (code.trim().parse().unwrap_or(0), body.to_owned())
}

fn sign_in(auth: &str, email: &str, password: &str) -> Result<String, (u16, String)> {
    let body = serde_json::json!({"input": {"email": email, "password": password}}).to_string();
    let (status, text) = http(
        "POST",
        &format!("{auth}/auth/sign-in/email"),
        None,
        Some(&body),
    );
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v["token"].as_str().map(str::to_owned))
        .filter(|_| status == 200)
        .ok_or((status, text))
}

fn session_status(auth: &str, token: &str) -> u16 {
    let body = serde_json::json!({ "token": token }).to_string();
    http("POST", &format!("{auth}/auth/session"), None, Some(&body)).0
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The stack's processes for one test: its state folder, its ports.
struct Stack {
    state: PathBuf,
    ports: [(&'static str, u16); 5],
    extra: Vec<(String, String)>,
}

impl Stack {
    fn new(tmp: &Path) -> Self {
        Self {
            state: tmp.join("state/stack"),
            ports: [
                ("MOCK_OAUTH", free_port()),
                ("AUTH", free_port()),
                ("NODE", free_port()),
                ("WEB", free_port()),
                ("SITE", free_port()),
            ],
            extra: Vec::new(),
        }
    }

    fn port(&self, name: &str) -> u16 {
        self.ports.iter().find(|(n, _)| *n == name).unwrap().1
    }

    fn url(&self, name: &str) -> String {
        format!("http://127.0.0.1:{}", self.port(name))
    }

    /// `xtask dev <args>` with the stack's settings and nothing of the
    /// shell's that the stack owns.
    fn xtask(&self, args: &[&str]) -> Command {
        let bins = bins();
        let mut c = Command::new(env!("CARGO_BIN_EXE_xtask"));
        c.arg("dev").args(args).current_dir(root());
        for (key, _) in std::env::vars() {
            if key.starts_with("AUTH_")
                || key.starts_with("ARCHITECT_")
                || key.starts_with("DEV_STACK_")
            {
                c.env_remove(key);
            }
        }
        c.env("DEV_STACK_ROOT", root())
            .env("DEV_STACK_STATE", &self.state)
            .env("DEVENV_STATE", self.state.parent().unwrap())
            .env("DEV_STACK_BIN_AUTH", &bins.auth)
            .env("DEV_STACK_BIN_NODE", &bins.node)
            .env("DEV_STACK_BIN_MOCK_OAUTH", &bins.mock);
        for (name, port) in self.ports {
            c.env(format!("DEV_STACK_PORT_{name}"), port.to_string());
        }
        for (k, v) in &self.extra {
            c.env(k, v);
        }
        c
    }

    fn start(&self, procs: &mut Procs, process: &str) {
        procs.spawn(process, self.xtask(&["run", process]));
    }

    fn fill(&self) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(self.state.join("fill.json")).unwrap())
            .unwrap()
    }
}

/// Wait until `url` answers 200 with `want` in it (as `bearer`); its body.
fn page_with(url: &str, bearer: Option<&str>, want: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (status, body) = http("GET", url, bearer, None);
        if status == 200 && body.contains(want) {
            return body;
        }
        assert!(
            Instant::now() < deadline,
            "{url} never had `{want}`; last {status}: {}",
            body.chars().take(400).collect::<String>()
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[test]
fn the_demo_stack_signs_in_serves_the_orgs_vault_and_each_share_audience() {
    let tmp = fresh_dir("demo");
    let stack = Stack::new(&tmp);
    let mut procs = Procs::new(tmp.join("logs"));

    stack.start(&mut procs, "mock-oauth");
    procs.wait(
        "mock-oauth",
        &stack.url("MOCK_OAUTH"),
        None,
        Duration::from_secs(30),
    );
    stack.start(&mut procs, "auth");
    procs.wait(
        "auth",
        &format!("{}/health", stack.url("AUTH")),
        Some(200),
        Duration::from_secs(60),
    );

    // Decided once, said, kept: the demo seed on a machine with no token.
    let fill = stack.fill();
    assert_eq!(fill["mode"], "demo", "{fill}");
    assert!(
        fill["why"]
            .as_str()
            .unwrap()
            .contains("no administrator token")
    );
    assert!(procs.whole_log("auth").contains("THE DEMO SEED"));
    assert_eq!(mode(&stack.state), 0o700);
    assert_eq!(mode(&stack.state.join("auth")), 0o700);
    assert_eq!(mode(&stack.state.join("auth/secret")), 0o600);
    assert_eq!(mode(&stack.state.join("fill.json")), 0o600);

    stack.start(&mut procs, "node");
    let node = stack.url("NODE");
    procs.wait(
        "node",
        &format!("{node}/health"),
        Some(200),
        Duration::from_secs(120),
    );
    assert_eq!(mode(&stack.state.join("node/session")), 0o600);

    // The node is on the local issuer's account, iroh on loopback alone.
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        let (code, body) = http("POST", &format!("{node}/node/status"), None, Some("{}"));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        if code == 200 && v["signed_in"] == true {
            break v;
        }
        assert!(
            Instant::now() < deadline,
            "the node never signed in: {code} {body}\n{}",
            procs.tail("node")
        );
        std::thread::sleep(Duration::from_millis(500));
    };
    assert_eq!(status["relays"], serde_json::json!([]), "{status}");
    let addrs = status["iroh_addrs"].as_array().unwrap();
    assert!(!addrs.is_empty());
    for addr in addrs {
        let addr: std::net::SocketAddr = addr.as_str().unwrap().parse().unwrap();
        assert!(addr.ip().is_loopback(), "{status}");
    }

    let auth = stack.url("AUTH");
    let grace = sign_in(&auth, "grace@local.test", DEV_PASSWORD).unwrap();
    let alan = sign_in(&auth, "alan@local.test", DEV_PASSWORD).unwrap();
    assert!(sign_in(&auth, "grace@local.test", "not-the-password").is_err());

    // Org only (`acme`, the vault's org): its member sees the vault through
    // the node; another org's member and nobody are refused.
    let org = format!("{node}/s/ksqx75iaaaaaaaad/");
    page_with(&org, Some(&grace), "Tracklist");
    assert_eq!(http("GET", &org, Some(&alan), None).0, 403);
    assert_eq!(http("GET", &org, None, None).0, 401);

    // Public and anyone-with-the-link: everybody.
    page_with(&format!("{node}/s/ksqx75iaaaaaaaab/"), None, "Modes");
    page_with(
        &format!("{node}/s/ksqx75iaaaaaaaac/"),
        None,
        "Blue Moon Walk",
    );

    // Named audiences: each link holds its own parts only.
    let violin = page_with(
        &format!("{node}/s/ksqx75iaaaaaaaaf/"),
        None,
        "Slow River (violin)",
    );
    assert!(!violin.contains("(cello)"), "{violin}");
    let cello = page_with(
        &format!("{node}/s/ksqx75iaaaaaaaag/"),
        None,
        "Amazing Grace (cello)",
    );
    assert!(!cello.contains("(violin)"), "{cello}");
    let cello_page = cello
        .split("href=\"")
        .filter_map(|s| s.split('"').next())
        .find(|h| h.contains("cello") && h.starts_with("/s/ksqx75iaaaaaaaag/"))
        .unwrap_or_else(|| panic!("no cello page linked: {cello}"))
        .to_owned();
    assert_eq!(
        http("GET", &format!("{node}{cello_page}"), None, None).0,
        200
    );
    let on_violin = cello_page.replace("ksqx75iaaaaaaaag", "ksqx75iaaaaaaaaf");
    assert_eq!(
        http("GET", &format!("{node}{on_violin}"), None, None).0,
        404
    );

    // The stack's own status reads what it decided.
    let out = stack.xtask(&["status"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("the demo seed") && text.contains(DEV_PASSWORD),
        "{text}"
    );

    drop(procs);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn a_credentialed_machine_mirrors_production_and_sessions_never_cross() {
    let tmp = fresh_dir("mirror");
    let mut procs = Procs::new(tmp.join("logs"));

    // "Production": a seeded auth server on its own port and secret.
    let prod_port = free_port();
    let prod = format!("http://127.0.0.1:{prod_port}");
    std::fs::create_dir_all(tmp.join("prod")).unwrap();
    let mut server = Command::new(&bins().auth);
    server
        .env("AUTH_BIND_ADDR", format!("127.0.0.1:{prod_port}"))
        .env("AUTH_BASE_URL", &prod)
        .env(
            "AUTH_DATABASE_URL",
            format!("sqlite://{}?mode=rwc", tmp.join("prod/auth.db").display()),
        )
        .env(
            "AUTH_SECRET",
            "production-stand-in-secret-of-48-bytes-or-more!!",
        )
        .env("AUTH_DEV_SEED", "1");
    procs.spawn("production", server);
    procs.wait(
        "production",
        &format!("{prod}/health"),
        Some(200),
        Duration::from_secs(60),
    );
    let body = serde_json::json!({"input": {
        "email": "only-in-prod@local.test",
        "password": "a password only production knows",
        "name": "Only In Production",
    }})
    .to_string();
    let (status, text) = http(
        "POST",
        &format!("{prod}/auth/sign-up/email"),
        None,
        Some(&body),
    );
    assert_eq!(status, 200, "{text}");
    let admin = sign_in(&prod, "ada@local.test", DEV_PASSWORD).unwrap();
    let token_file = tmp.join("admin-token");
    std::fs::write(&token_file, &admin).unwrap();

    let mut stack = Stack::new(&tmp);
    stack.extra = vec![
        (
            "AUTH_ADMIN_TOKEN_FILE".into(),
            token_file.display().to_string(),
        ),
        ("DEV_STACK_MIRROR_FROM".into(), prod.clone()),
        (
            "DEV_STACK_TEST_PRODUCTION_HOSTS".into(),
            format!("127.0.0.1:{prod_port}"),
        ),
    ];

    // The guard: a stack whose issuer is production's is refused, and so
    // is a shell carrying a production secret.
    let mut pointed = stack.xtask(&["guard"]);
    pointed.env("DEV_STACK_PORT_AUTH", prod_port.to_string());
    let out = pointed.output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("is production"));
    let mut carrying = stack.xtask(&["guard"]);
    carrying.env("AUTH_SECRET", "production's");
    let out = carrying.output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("AUTH_SECRET"));
    let out = stack.xtask(&["guard"]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    stack.start(&mut procs, "mock-oauth");
    stack.start(&mut procs, "auth");
    let auth = stack.url("AUTH");
    procs.wait(
        "auth",
        &format!("{auth}/health"),
        Some(200),
        Duration::from_secs(60),
    );

    let fill = stack.fill();
    assert_eq!(fill["mode"], "mirror", "{fill}");
    assert_eq!(fill["emails_redacted"], true);
    assert!(
        fill["why"]
            .as_str()
            .unwrap()
            .contains("AUTH_ADMIN_TOKEN_FILE")
    );
    assert_eq!(fill["node_org"], "acme", "{fill}");

    // The snapshot: private, no real address, no credential.
    let snapshot_path = stack.state.join("auth/snapshot.json");
    assert_eq!(mode(&snapshot_path), 0o600);
    let snapshot = std::fs::read_to_string(&snapshot_path).unwrap();
    assert!(!snapshot.contains("@local.test"));
    assert!(!snapshot.contains(&admin));
    for name in ["auth", "mock-oauth"] {
        assert!(
            !procs.whole_log(name).contains(&admin),
            "{name}'s log has the token"
        );
    }

    // Every mirrored account signs in locally with the development
    // password, never with its own.
    let snap: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    let email = snap["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["name"] == "Only In Production")
        .and_then(|u| u["email"].as_str())
        .unwrap()
        .to_owned();
    assert!(email.ends_with("@local.invalid"), "{email}");
    let local = sign_in(&auth, &email, DEV_PASSWORD).unwrap();
    assert!(sign_in(&auth, &email, "a password only production knows").is_err());

    // Sessions are worthless across the line, both ways.
    assert_eq!(session_status(&auth, &local), 200);
    assert_ne!(session_status(&prod, &local), 200);
    assert_eq!(session_status(&prod, &admin), 200);
    assert_ne!(session_status(&auth, &admin), 200);

    drop(procs);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A fresh folder under the temp dir.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dev-stack-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
