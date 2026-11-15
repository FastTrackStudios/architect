//! An archive the npm CLI packs and signs (`architect-ext pack`,
//! `features/extensions/web/cli/ts.mjs`, #536) opens here as one `cargo
//! xtask ext-pack` and `ext-sign` made does: the Recipe scaler
//! (`examples/showcase/recipe-scaler`), signed with a throwaway key this
//! test makes, checked against it, and refused by any other.
//!
//! It needs node (from the PATH, as the dev shell has it). Without it the
//! test says so and passes; `ARCHITECT_WASI_WEB_REQUIRED=1` (main CI, as
//! `features/extensions/tests/wasi_web.rs` reads it) makes that a failure.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::Command;

use architect_extension_archive::sign::SecretKey;
use architect_extension_archive::{ArchiveError, Trust, open};

#[test]
fn an_archive_the_npm_cli_packs_and_signs_opens_here() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let cli = root.join("features/extensions/web/cli/cli.mjs");
    let has_deps = root
        .join("features/extensions/web/node_modules/smol-toml")
        .is_dir();
    let node = Command::new("node").arg("--version").output();
    if node.is_err() || !has_deps {
        assert!(
            std::env::var_os("ARCHITECT_WASI_WEB_REQUIRED").is_none(),
            "node, or the CLI's packages (npm ci in features/extensions/web), are missing"
        );
        eprintln!("skipped: no node, or no npm ci in features/extensions/web");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let key_file = tmp.path().join("ext.key");
    std::fs::write(&key_file, key.secret_file()).unwrap();
    let out = tmp.path().join("recipes.archext");
    let packed = Command::new("node")
        .arg(&cli)
        .arg("pack")
        .arg(root.join("examples/showcase/recipe-scaler"))
        .arg("--out")
        .arg(&out)
        .arg("--key")
        .arg(&key_file)
        .output()
        .unwrap();
    assert!(
        packed.status.success(),
        "{}",
        String::from_utf8_lossy(&packed.stderr)
    );
    let bytes = std::fs::read(&out).unwrap();
    let verified = open(&bytes, &Trust::none().with("test key", key.public())).unwrap();
    assert_eq!(verified.index.id, "showcase.recipes");
    assert_eq!(verified.index.kind, "typescript");
    assert!(verified.index.platforms.is_empty());
    assert!(
        verified
            .index
            .files
            .iter()
            .any(|f| f.path == "actions/scale.ts")
    );
    let dir = tmp.path().join("installed");
    verified.extract(&dir).unwrap();
    assert!(dir.join("pages/lib/recipe.ts").is_file());
    // Another key's trust refuses it.
    let other = SecretKey::generate().unwrap();
    assert!(matches!(
        open(&bytes, &Trust::none().with("other", other.public())),
        Err(ArchiveError::UntrustedKey { .. })
    ));
}
