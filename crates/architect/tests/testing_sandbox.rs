//! `architect::testing::sandbox_server`: a boot never sees the
//! developer's home. HOME (and every per-user dir variable) is set to a
//! poisoned directory first; the seed, the served app and a request
//! handler all see the sandbox instead, and the poisoned directory is
//! still empty afterwards.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use architect::LayerRouter;
use architect::http::axum::Router;
use architect::http::axum::routing::get;
use architect::testing::{HOME_VARS, Sandbox, env_lock, sandbox, sandbox_server};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// Plain HTTP/1.0 GET over a loopback socket; returns the body.
async fn http_get(addr: std::net::SocketAddr, path: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(format!("GET {path} HTTP/1.0\r\nHost: {addr}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    assert!(
        head.starts_with("HTTP/1.0 200") || head.starts_with("HTTP/1.1 200"),
        "{head}"
    );
    body.to_owned()
}

/// Point HOME and every per-user dir variable at `poison`, under the lock.
async fn poison_home(poison: &Path) {
    let mut env = env_lock().await;
    for (name, _) in HOME_VARS {
        env.set(name, poison);
    }
}

fn assert_home_is_sandboxed(sb: &Sandbox, poison: &Path) {
    for (name, _) in HOME_VARS {
        let value = PathBuf::from(std::env::var_os(name).unwrap());
        assert!(
            sb.contains(&value),
            "{name}={} is outside the sandbox",
            value.display()
        );
        assert!(
            !value.starts_with(poison),
            "{name} still points at the poisoned home"
        );
    }
    assert_eq!(std::env::var_os("HOME").map(PathBuf::from), Some(sb.home()));
    assert!(sb.contains(&std::env::home_dir().unwrap()));
}

/// What an app does at boot: resolve its config dir from the environment
/// and write there.
fn write_app_config() -> PathBuf {
    let config = PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap()).join("app");
    std::fs::create_dir_all(&config).unwrap();
    let file = config.join("settings.json");
    std::fs::write(&file, "{}").unwrap();
    file
}

#[tokio::test]
async fn sandbox_server_never_reads_the_developers_home() {
    let poison = tempfile::tempdir().unwrap();
    let poison_path = poison.path().to_owned();
    poison_home(&poison_path).await;

    let seed_poison = poison_path.clone();
    let server = sandbox_server(|sb| async move {
        assert_home_is_sandboxed(&sb, &seed_poison);
        let written = write_app_config();
        assert!(sb.contains(&written));
        assert!(sb.home().join(".config/app/settings.json").is_file());

        // A handler that reads HOME per request, after the boot.
        let app = Router::new().route(
            "/home",
            get(|| async { std::env::var("HOME").unwrap_or_default() }),
        );
        Ok(app)
    })
    .await
    .unwrap();

    // Per request, HOME is still a sandbox's: this one's under nextest (a
    // process per test), or a sibling test's under plain `cargo test`.
    let served_home = PathBuf::from(http_get(server.addr(), "/home").await);
    assert!(!served_home.starts_with(&poison_path));
    assert!(
        served_home.to_string_lossy().contains("architect-sandbox-"),
        "{}",
        served_home.display()
    );
    assert!(server.url().starts_with("ws://127.0.0.1:"));
    assert!(server.url().ends_with("/vox"));

    assert_eq!(
        std::fs::read_dir(&poison_path).unwrap().count(),
        0,
        "the poisoned home was written to"
    );

    let root = server.sandbox().root().to_owned();
    assert!(root.is_dir());
    drop(server);
    assert!(!root.exists(), "dropping the server deletes the sandbox");
}

#[tokio::test]
async fn a_vox_router_is_served_on_an_engine_host() {
    let server = sandbox_server(|_| async { Ok(LayerRouter::new()) })
        .await
        .unwrap();
    assert_eq!(http_get(server.addr(), "/health").await, "ok");
}

#[tokio::test]
async fn app_variables_are_cleared_set_and_pointed_at_the_data_root() {
    {
        let mut env = env_lock().await;
        env.set("ARCHITECT_SANDBOX_TEST_STALE", "/from/a/developer/shell");
    }
    let booted = sandbox()
        .clear("ARCHITECT_SANDBOX_TEST_STALE")
        .data_env("ARCHITECT_SANDBOX_TEST_DATA_ROOT")
        .env("ARCHITECT_SANDBOX_TEST_FLAG", "1")
        .boot(|sb| async move {
            assert_eq!(std::env::var_os("ARCHITECT_SANDBOX_TEST_STALE"), None);
            assert_eq!(
                std::env::var_os("ARCHITECT_SANDBOX_TEST_DATA_ROOT").map(PathBuf::from),
                Some(sb.data())
            );
            assert_eq!(
                std::env::var("ARCHITECT_SANDBOX_TEST_FLAG").as_deref(),
                Ok("1")
            );
            std::fs::write(sb.data().join("seeded"), "x")?;
            Ok::<_, eyre::Report>(sb.data().join("seeded"))
        })
        .await
        .unwrap();
    assert!(booted.value.is_file());

    // The restart shape: a second boot over the first's sandbox sees what
    // the first left, and does not own (or delete) the directory.
    let again = sandbox()
        .over(&booted.sandbox)
        .boot(|sb| async move { Ok(sb.data().join("seeded").is_file()) })
        .await
        .unwrap();
    assert!(again.value);
    let root = again.sandbox.root().to_owned();
    drop(again);
    assert!(
        root.is_dir(),
        "a boot over another's sandbox must not delete it"
    );
    drop(booted);
    assert!(!root.exists());
}
