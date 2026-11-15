//! The extension showcase in the app itself (#710): its vault, and the
//! showcase extensions it names as a registry compiled into the app.
//!
//! Opening the showcase vault (`apps/ui/showcase-vault`, from the first
//! run's "Explore the showcase", or `just showcase`) lists what it needs
//! (`.architect/extensions.toml`) with **Install all**: the Luau,
//! TypeScript and data packages below install from this built-in registry
//! through the one consent prompt, with no network and nothing to add, on
//! the desk and on a phone. What needs a build of its own (the process
//! extensions, the WASI component, the web view) is not carried: `just
//! showcase` builds and loads them on a desk, and elsewhere the vault lists
//! them as not found.

use architect_extensions::bundled::{BundledPackage, BundledSource};
use architect_extensions::manifest::{MANIFEST, Manifest};
use include_dir::{Dir, DirEntry, include_dir};

/// The registry's name, as the extensions page and the prompt show it.
pub const SOURCE: &str = "Showcase (built into the app)";

static SHOWCASE_EXAMPLES: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/../../examples/showcase");
static SANDBOX_EXTENSIONS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../sandbox/extensions");

/// The sandbox's Luau extensions the showcase vault names.
const SANDBOX_NAMED: [&str; 4] = ["outline", "surface", "metronome", "tidy"];

/// The kinds carried: what runs from its files alone.
const CARRIED: [&str; 3] = ["luau", "typescript", "data"];

fn files(dir: &'static Dir<'static>, base: &Dir<'static>, out: &mut Vec<(String, &'static [u8])>) {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(d) => {
                let name = d.path().file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !matches!(name, "node_modules" | "target" | "tests") {
                    files(d, base, out);
                }
            }
            DirEntry::File(f) => {
                let rel = f
                    .path()
                    .strip_prefix(base.path())
                    .unwrap_or_else(|_| f.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, f.contents()));
            }
        }
    }
}

fn package(dir: &'static Dir<'static>) -> Option<BundledPackage> {
    let text = dir.get_file(dir.path().join(MANIFEST))?.contents_utf8()?;
    let kind = Manifest::parse(text).ok()?.kind;
    if !CARRIED.contains(&kind.name()) {
        return None;
    }
    let mut out = Vec::new();
    files(dir, dir, &mut out);
    Some(BundledPackage {
        folder: dir.path().to_string_lossy().into_owned(),
        files: out,
    })
}

/// Every showcase package the app carries.
#[must_use]
pub fn packages() -> Vec<BundledPackage> {
    let examples = SHOWCASE_EXAMPLES.dirs().filter_map(package);
    let sandbox = SANDBOX_EXTENSIONS
        .dirs()
        .filter(|d| {
            d.path()
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| SANDBOX_NAMED.contains(&n))
        })
        .filter_map(package);
    // Keyed by where each comes from: the two folders share names
    // (`surface`).
    examples
        .map(|mut p| {
            p.folder = format!("examples/{}", p.folder);
            p
        })
        .chain(sandbox.map(|mut p| {
            p.folder = format!("sandbox/{}", p.folder);
            p
        }))
        .collect()
}

/// The showcase's built-in registry.
#[must_use]
pub fn source() -> BundledSource {
    BundledSource::new(SOURCE, packages())
}

#[cfg(test)]
mod tests {
    use super::{CARRIED, packages};
    use architect_extensions::manifest::{MANIFEST, Manifest};

    /// The showcase vault, opened with nothing installed: "Install all"
    /// offers every package the app carries that the vault requires, and
    /// one grant installs them all from the built-in registry (#710).
    #[test]
    fn the_showcase_vault_installs_its_extensions_from_the_app_itself() {
        use architect::action::ActionList;
        use architect_extensions::consent::{Grants, Scope};
        use architect_extensions::extensions::Extensions;
        use architect_extensions::host::{Host, SharedSections};
        use architect_extensions::install::Installs;
        use architect_extensions::manage::Installed;

        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("Showcase");
        crate::vaults::SHOWCASE_FOR_TESTS.extract(&vault).unwrap();
        assert!(
            vault.join(".architect/extensions.toml").is_file(),
            "its needs travel with it"
        );
        let list = ActionList::new();
        // The runners the app's host has for what the showcase carries:
        // Luau (built in) and TypeScript, as `crate::extensions` adds it.
        let host = Host::new(
            list.clone(),
            SharedSections::new(architect_settings::Registry::new()),
        )
        .with_grants(Grants::open(tmp.path().join("grants.json")))
        .with_runner(architect_pages::js::action::TsRunner::new(
            architect_pages::host::AppHost::new(list),
        ));
        let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
            .with_source(super::source());
        let opened = ext.open_vault(&vault);
        let offered: Vec<&str> = opened.plan.install.iter().map(|l| l.id.as_str()).collect();
        for p in packages() {
            let (_, bytes) = p.files.iter().find(|(f, _)| f == MANIFEST).unwrap();
            let id = Manifest::parse(std::str::from_utf8(bytes).unwrap())
                .unwrap()
                .id;
            assert!(
                offered.contains(&id.as_str()),
                "{id} not offered: {offered:?}"
            );
        }
        // Nothing before the one yes; all of it after.
        let waiting = ext.accept_vault(&vault, &opened.plan, None, "person");
        assert!(
            waiting
                .iter()
                .all(|r| matches!(r, Ok(Installed::NeedsConsent { .. })))
        );
        let done = ext.accept_vault(&vault, &opened.plan, Some(&Scope::Device), "person");
        assert!(done.iter().all(Result::is_ok), "{done:?}");
        for id in offered {
            assert!(ext.installs().get(id).is_some(), "{id} not installed");
        }
    }

    #[test]
    fn it_carries_every_showcase_package_that_runs_from_its_files() {
        let carried = packages();
        let ids: Vec<String> = carried
            .iter()
            .map(|p| {
                let (_, bytes) = p.files.iter().find(|(f, _)| f == MANIFEST).unwrap();
                let m = Manifest::parse(std::str::from_utf8(bytes).unwrap()).unwrap();
                assert!(CARRIED.contains(&m.kind.name()), "{}", m.id);
                m.id
            })
            .collect();
        for id in [
            "showcase.templater",
            "showcase.recipes",
            "lab.outline",
            "fts.metronome",
        ] {
            assert!(ids.iter().any(|i| i == id), "{id} not carried: {ids:?}");
        }
        assert!(
            !ids.iter().any(|i| i == "lab.clicker"),
            "a process is not carried"
        );
        let mut folders: Vec<&str> = carried.iter().map(|p| p.folder.as_str()).collect();
        folders.sort_unstable();
        let before = folders.len();
        folders.dedup();
        assert_eq!(folders.len(), before, "two packages share a folder");
    }
}
