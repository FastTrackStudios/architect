//! A share link's extensions (#710 L2, G1): the owner chooses which a link
//! runs (`EXT_SHARE_NEEDS_SET`): a built-in from the linked core, an
//! installed one as its signed archive copied into the link. A guest reads
//! the link's file, plans, checks each package it is handed (the pinned
//! sha256, the signature, the id and version) and runs it within the guest
//! profile.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use architect::action::ActionList;
use architect_extensions::consent::Grants;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::{FolderRegistry, Installs, Trust};
use architect_extensions::linked::{Builder, Extension};
use architect_extensions::share_needs::{
    LinkNeeds, PackageError, file_path, packages_dir, plan_link, verify_package,
};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::json;

fn showcase() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/examples/showcase-registry")
}

fn key() -> String {
    std::fs::read_to_string(showcase().join("keys/showcase.pub"))
        .unwrap()
        .lines()
        .nth(1)
        .unwrap()
        .to_owned()
}

/// A built-in, linked into the distribution: the guest's core has it too.
struct Recipes;

impl Extension for Recipes {
    fn manifest(&self) -> &'static str {
        "id = \"core.recipes\"\nname = \"Recipes\"\nversion = \"1.4.0\"\nkind = \"data\"\n"
    }

    fn build(&self, _: &mut Builder) {}
}

const CODE: &str = "k3vq8x2mz7wn4pta";

#[test]
fn the_owner_chooses_a_links_extensions_and_a_guest_checks_each_package() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_grants(Grants::open(tmp.path().join("grants.json")));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_source(FolderRegistry::new(showcase()).trusting("Showcase", key()))
        .with_linked(&Recipes)
        .unwrap();
    ext.install_actions().unwrap();
    let _ = ext.sync();
    let run = |id: &str, args: serde_json::Value| {
        block_on(list.run_json_async(id, args.to_string().as_bytes()))
            .map(|o| serde_json::from_slice::<serde_json::Value>(&o).unwrap())
    };
    run(
        "EXT_INSTALL",
        json!({ "id": "showcase.greeter", "version": "=0.1.0", "grant": "device" }),
    )
    .unwrap();

    // A host its manifest does not ask for is refused; so is a code that
    // is not one.
    let folder = vault.display().to_string();
    let e = run(
        "EXT_SHARE_NEEDS_SET",
        json!({ "folder": folder, "code": CODE, "extensions": [{ "id": "showcase.greeter", "hosts": ["evil.example"] }] }),
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("does not ask for host evil.example"), "{e}");
    let e = run(
        "EXT_SHARE_NEEDS_SET",
        json!({ "folder": folder, "code": "../x", "extensions": [] }),
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("not a share code"), "{e}");

    // The owner's choice: the greeter from the link, recipes from the core.
    let set = run(
        "EXT_SHARE_NEEDS_SET",
        json!({ "folder": folder, "code": CODE, "extensions": [
            { "id": "showcase.greeter" },
            { "id": "core.recipes", "required": false },
        ] }),
    )
    .unwrap();
    // The greeter as the link's package; recipes from the core (none).
    assert_eq!(set["packages"][0]["id"], "showcase.greeter", "{set}");
    assert_eq!(set["packages"].as_array().unwrap().len(), 1, "{set}");
    assert_eq!(set["extensions"][1]["required"], false, "{set}");
    let read = run("EXT_SHARE_NEEDS", json!({ "folder": folder, "code": CODE })).unwrap();
    assert_eq!(read, set);

    // The guest: the link's file, its plan, the package it is handed.
    let text = std::fs::read_to_string(vault.join(file_path(CODE))).unwrap();
    let needs = LinkNeeds::from_toml(&text).unwrap();
    let core = vec![("core.recipes".to_owned(), semver::Version::new(1, 4, 2))];
    let plan = plan_link(&needs, &core);
    assert_eq!(plan.ready, ["core.recipes"]);
    assert_eq!(plan.prompt().as_deref(), Some("This link uses Greeter"));
    let (_, package) = &plan.fetch[0];
    let bytes = std::fs::read(vault.join(packages_dir(CODE)).join(&package.archive)).unwrap();
    let trust = Trust::org().with("Showcase", key());
    let pkg = verify_package(&bytes, &needs, package, &trust, "web").unwrap();
    assert_eq!(pkg.manifest.id, "showcase.greeter");
    // Its asks (the selection, notifications) within the guest profile.
    assert_eq!(pkg.capabilities.reads, ["selection"]);
    assert_eq!(pkg.capabilities.ui, ["notify"]);
    assert!(pkg.capabilities.writes.is_empty());

    // Changed bytes, or a key the guest does not trust: refused.
    let mut tampered = bytes.clone();
    let final_byte = tampered.len() - 1;
    tampered[final_byte] ^= 1;
    assert!(matches!(
        verify_package(&tampered, &needs, package, &trust, "web"),
        Err(PackageError::Sha256 { .. })
    ));
    assert!(matches!(
        verify_package(&bytes, &needs, package, &Trust::org(), "web"),
        Err(PackageError::Archive(_))
    ));

    // Cleared: the file and the packages go.
    run(
        "EXT_SHARE_NEEDS_SET",
        json!({ "folder": folder, "code": CODE, "extensions": [] }),
    )
    .unwrap();
    assert!(!vault.join(file_path(CODE)).exists());
    assert!(!vault.join(packages_dir(CODE)).exists());
}

#[cfg(unix)]
#[test]
fn a_links_files_are_never_read_or_removed_through_a_link() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(vault.join("_architect")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.archext"), b"not yours").unwrap();
    std::fs::write(outside.join("big.toml"), vec![b'#'; 200 * 1024]).unwrap();
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));

    // The shares folder a link elsewhere: refused, nothing read.
    std::os::unix::fs::symlink(&outside, vault.join("_architect/shares")).unwrap();
    let e = ext.share_needs(&vault, CODE).unwrap_err();
    assert!(e.contains("link"), "{e}");
    // Clearing a link's extensions removes nothing through it.
    let e = ext.set_share_needs(&vault, CODE, &[]).unwrap_err();
    assert!(e.contains("link"), "{e}");
    assert!(outside.join("keep.archext").is_file());

    // A real folder, its packages folder a link elsewhere: the same.
    std::fs::remove_file(vault.join("_architect/shares")).unwrap();
    std::fs::create_dir_all(vault.join("_architect/shares")).unwrap();
    std::os::unix::fs::symlink(&outside, vault.join(packages_dir(CODE))).unwrap();
    std::fs::write(vault.join(file_path(CODE)), "").unwrap();
    assert!(
        ext.set_share_needs(&vault, CODE, &[])
            .unwrap_err()
            .contains("link")
    );
    assert!(
        outside.join("keep.archext").is_file(),
        "deleted through a link"
    );

    // An oversized file: refused by its size, before it is read.
    std::fs::remove_file(vault.join(packages_dir(CODE))).unwrap();
    std::fs::copy(outside.join("big.toml"), vault.join(file_path(CODE))).unwrap();
    assert!(
        ext.share_needs(&vault, CODE)
            .unwrap_err()
            .contains("more than")
    );
}
