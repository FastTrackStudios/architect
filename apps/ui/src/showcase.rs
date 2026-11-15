//! The extension showcase in the app itself (#710): its vault, and the
//! showcase extensions it names as a registry compiled into the app.
//!
//! Opening the showcase vault (`apps/ui/showcase-vault`, from the first
//! run's "Explore the showcase", or `just showcase`) lists what it needs
//! (`.architect/extensions.toml`) with **Install all**: the Luau,
//! TypeScript and data packages the distribution carries
//! (`apps/distribution/src/showcase.rs`, the web app's too) install through
//! the one consent prompt, with no network and nothing to add, on the desk
//! and on a phone.

pub use architect_distribution::showcase::{CARRIED, SOURCE, packages, source};

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
