//! The extension showcase's packages the app carries (#710).
//!
//! The Luau, TypeScript and data packages the showcase vault
//! (`apps/ui/demo-vault`) names, as a registry compiled into the build
//! (`architect_extensions::bundled::BundledSource`). The desk and phone app
//! (`apps/ui/src/showcase.rs`) and the web app (`apps/web/src/app/extensions.rs`)
//! offer them with the vault's **Install all**, through the one consent
//! prompt, with no network. What needs a build of its own (the process
//! extensions, the WASI component, the web view) is not carried: `just
//! showcase` builds and loads them on a desk, and elsewhere the vault lists
//! them as not found. What a platform has no runner for (Luau in a browser,
//! #812) is carried and listed as "Can't run here", with why.

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
pub const CARRIED: [&str; 3] = ["luau", "typescript", "data"];

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
