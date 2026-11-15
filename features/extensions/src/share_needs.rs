//! What a share link's guests run (#710 L2, G1).
//!
//! The owner's choice, per link, of the extensions a guest gets, where
//! each comes from, and the guest profile it runs in
//! (`docs/content/architecture/extensions.md`, "A share link's
//! extensions").
//!
//! A share is a note of its vault (`_architect/shares/<code>.md`, kept by
//! `architect-spaces`); what its link runs is kept beside it, as its
//! comments are, so it syncs with it and the node serves it with it:
//!
//! ```toml
//! # _architect/shares/<code>.extensions.toml
//! # The link's Requirements: the same [[extensions]] a vault's
//! # .architect/extensions.toml and a project's meta hold.
//! [[extensions]]
//! id = "health.meal-plan"
//! name = "Meal plan"
//! version = "^1.2"
//! required = true
//!
//! [[extensions]]
//! id = "setlist"            # no package: the distribution's linked core
//! required = false
//!
//! # The signed archives the link carries, in _architect/shares/<code>.extensions/.
//! [[packages]]
//! id = "health.meal-plan"
//! version = "1.2.0"
//! archive = "health.meal-plan-1.2.0.archext"
//! sha256 = "…"
//!
//! # The hosts the owner lets each fetch (of those its manifest asks).
//! [hosts]
//! "health.meal-plan" = ["api.example.com"]
//! ```
//!
//! An audience's link (its own code) has a file of its own; one without
//! takes its share's ([`file_for`]).
//!
//! The guest host (the web app's, iOS's, the desktop's) reads it with
//! [`LinkNeeds::from_toml`], plans with [`plan_link`], checks each package
//! it is handed with [`verify_package`] (feature `archive`), and runs each
//! extension within [`guest_capabilities`]: read-only documents of the
//! share, no writes, no secrets, `fetch` only to the hosts the owner
//! approved. This module is the model alone: pure, wasm-clean.

use std::collections::BTreeMap;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::manifest::{Capabilities, Reach};
use crate::project::{Requirement, Requirements};

/// The folder a vault's shares are kept in (`architect_spaces::SHARES_FOLDER`).
pub const SHARES_FOLDER: &str = "_architect/shares";

/// The most a link's needs file may be.
pub const MAX_FILE: usize = 64 * 1024;

/// Whether `code` can be a link's code: lower-case letters and digits,
/// 1 to 64 of them (so its files stay in [`SHARES_FOLDER`]).
#[must_use]
pub fn is_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// The file share link `code`'s extensions are named in, in its vault.
#[must_use]
pub fn file_path(code: &str) -> String {
    format!("{SHARES_FOLDER}/{code}.extensions.toml")
}

/// The folder share link `code`'s packages are kept in, in its vault.
#[must_use]
pub fn packages_dir(code: &str) -> String {
    format!("{SHARES_FOLDER}/{code}.extensions")
}

/// The file that names what link `code` runs: its own when it has one
/// (`exists`), else its share's (`share`, for an audience's link).
#[must_use]
pub fn file_for(code: &str, share: &str, exists: impl Fn(&str) -> bool) -> String {
    let own = file_path(code);
    if code == share || exists(&own) {
        own
    } else {
        file_path(share)
    }
}

/// A signed archive a link carries for one of its extensions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPackage {
    pub id: String,
    /// Its version: one its requirement takes.
    pub version: String,
    /// Its file name in [`packages_dir`]: a plain `.archext` name.
    pub archive: String,
    /// Its SHA-256, lower-case hex.
    pub sha256: String,
}

/// What a share link's guests run: the owner's choice, per link.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkNeeds {
    /// Its requirements (`[[extensions]]`): an id with a package comes
    /// from the link, one without from the distribution's linked core.
    #[serde(flatten)]
    pub requirements: Requirements,
    /// The signed archives it carries (`[[packages]]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<LinkPackage>,
    /// The hosts the owner lets each fetch, by id (`[hosts]`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hosts: BTreeMap<String, Vec<String>>,
}

impl LinkNeeds {
    /// From the file's text, checked ([`Self::check`]).
    ///
    /// # Errors
    ///
    /// It is larger than [`MAX_FILE`], not TOML, not link needs, or breaks
    /// a rule.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        if text.len() > MAX_FILE {
            return Err(format!("{} bytes, more than {MAX_FILE}", text.len()));
        }
        let needs: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        needs.check()?;
        Ok(needs)
    }

    /// As the file's text.
    ///
    /// # Errors
    ///
    /// The encoder's message.
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|e| e.to_string())
    }

    /// The package the link carries for `id`, when it carries one.
    #[must_use]
    pub fn package(&self, id: &str) -> Option<&LinkPackage> {
        self.packages.iter().find(|p| p.id == id)
    }

    /// The hosts the owner lets `id` fetch.
    #[must_use]
    pub fn hosts_of(&self, id: &str) -> &[String] {
        self.hosts.get(id).map_or(&[], Vec::as_slice)
    }

    /// The rules: each id required once; each package for a required id,
    /// once, at a version its range takes, a plain `.archext` file name
    /// (never a path) and a SHA-256; hosts only for required ids, each
    /// named (never `*`).
    ///
    /// # Errors
    ///
    /// The first rule broken, naming the extension.
    pub fn check(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        for r in &self.requirements.extensions {
            if !seen.insert(r.id.as_str()) {
                return Err(format!("{} is named twice", r.id));
            }
            if VersionReq::parse(&r.version).is_err() {
                return Err(format!("{}: version {:?} is not a range", r.id, r.version));
            }
        }
        let mut packaged = std::collections::BTreeSet::new();
        for p in &self.packages {
            let Some(r) = self.requirements.extensions.iter().find(|r| r.id == p.id) else {
                return Err(format!("package {}: the link does not name it", p.id));
            };
            if !packaged.insert(p.id.as_str()) {
                return Err(format!("{} has two packages", p.id));
            }
            let v = Version::parse(&p.version)
                .map_err(|e| format!("package {}: version {:?}: {e}", p.id, p.version))?;
            if !r.req().matches(&v) {
                return Err(format!(
                    "package {} {v}: not a version the link takes ({})",
                    p.id, r.version
                ));
            }
            if !safe_file_name(&p.archive) {
                return Err(format!(
                    "package {}: {:?} is not a .archext file name",
                    p.id, p.archive
                ));
            }
            if p.sha256.len() != 64 || !p.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("package {}: no sha256", p.id));
            }
        }
        for (id, hosts) in &self.hosts {
            if !seen.contains(id.as_str()) {
                return Err(format!("hosts of {id}: the link does not name it"));
            }
            if let Some(h) = hosts.iter().find(|h| h.as_str() == "*" || h.is_empty()) {
                return Err(format!("hosts of {id}: {h:?}: name each host"));
            }
        }
        Ok(())
    }
}

/// A file name alone, ending `.archext`: no separator, no `..`, not hidden.
fn safe_file_name(name: &str) -> bool {
    name.ends_with(".archext")
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && !name.contains("..")
        && name.len() <= 200
}

/// What the guest profile lets an extension read: the share's documents
/// (read-only, `ShareGuest`) and the selection.
pub const GUEST_READS: [&str; 2] = [
    crate::capabilities::DOCUMENTS,
    crate::capabilities::SELECTION,
];
/// What of the interface a guest's extension may use.
pub const GUEST_UI: [&str; 4] = [
    crate::capabilities::NOTIFY,
    crate::capabilities::PROMPTS,
    crate::capabilities::MENUS,
    crate::capabilities::BADGES,
];

/// The most an extension may reach as a guest.
///
/// Given what its manifest `asks` and the `hosts` the owner approved for
/// it: read-only documents of the share and the selection, no writes, no
/// actions past its own, read-only effects, `fetch` only to approved hosts
/// it also asks for, no folders, ports, language servers or peer
/// connections. Secrets are never the guest's: a guest host refuses
/// `[[secrets]]` outright.
#[must_use]
pub fn guest_capabilities(asks: &Capabilities, hosts: &[String]) -> Capabilities {
    let granted = Capabilities {
        reads: GUEST_READS.iter().map(|s| (*s).to_owned()).collect(),
        ui: GUEST_UI.iter().map(|s| (*s).to_owned()).collect(),
        hosts: hosts.to_vec(),
        effect: Reach::ReadOnly,
        ..Capabilities::default()
    };
    let mut caps = asks.within(&granted);
    // Nothing past the profile, whatever `within` keeps.
    caps.writes.clear();
    caps.actions.clear();
    caps.folders.clear();
    caps.ports.clear();
    caps.languages.clear();
    caps.webrtc.clear();
    caps.effect = Reach::ReadOnly;
    caps
}

/// Why a link's extension cannot be had by this guest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkMissing {
    /// No package: from the core, and this app's core has none that fits.
    NotInCore,
}

impl std::fmt::Display for LinkMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInCore => f.write_str("this app's core has no version that fits"),
        }
    }
}

/// What opening a share link takes, for its guest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkPlan {
    /// Running here already at a version that fits (the linked core's, or
    /// one the guest took before).
    pub ready: Vec<String>,
    /// To take from the link: fetch `<archive>` in [`packages_dir`], check it
    /// with [`verify_package`], load it within [`guest_capabilities`].
    pub fetch: Vec<(Requirement, LinkPackage)>,
    /// Cannot be had here; the link still opens, a required one's views
    /// say what they need.
    pub missing: Vec<(Requirement, LinkMissing)>,
}

impl LinkPlan {
    /// The prompt's line ("This link uses Meal plan"), when there is
    /// something to take.
    #[must_use]
    pub fn prompt(&self) -> Option<String> {
        let names: Vec<&str> = self
            .fetch
            .iter()
            .map(|(r, _)| {
                if r.name.is_empty() {
                    r.id.as_str()
                } else {
                    r.name.as_str()
                }
            })
            .collect();
        match names.as_slice() {
            [] => None,
            [one] => Some(format!("This link uses {one}")),
            [rest @ .., last] => Some(format!("This link uses {} and {last}", rest.join(", "))),
        }
    }
}

/// What a guest takes to open a link with `needs`, given what is running
/// here (`here`: each id and version, the linked core's among them).
#[must_use]
pub fn plan_link(needs: &LinkNeeds, here: &[(String, Version)]) -> LinkPlan {
    let mut out = LinkPlan::default();
    for r in &needs.requirements.extensions {
        let req = r.req();
        if here.iter().any(|(id, v)| id == &r.id && req.matches(v)) {
            out.ready.push(r.id.clone());
            continue;
        }
        match needs.package(&r.id) {
            Some(p) => out.fetch.push((r.clone(), p.clone())),
            None => out.missing.push((r.clone(), LinkMissing::NotInCore)),
        }
    }
    out
}

/// Why a link's package was refused.
#[cfg(feature = "archive")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageError {
    /// Its bytes are not what the link's file pinned.
    Sha256 { expected: String, got: String },
    /// The archive is unsigned, signed by a key not trusted here, changed.
    Archive(architect_extension_archive::ArchiveError),
    /// It is another extension or version than the link's package says.
    Mismatch(String),
    /// Its manifest does not read.
    Manifest(String),
    /// Its `platforms` leave this one out.
    NotHere(String),
}

#[cfg(feature = "archive")]
impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sha256 { expected, got } => {
                write!(f, "its sha256 is {got}, the link pinned {expected}")
            }
            Self::Archive(e) => write!(f, "the archive is refused: {e}"),
            Self::Mismatch(why) | Self::Manifest(why) | Self::NotHere(why) => f.write_str(why),
        }
    }
}

/// A link's package, checked: its manifest, what it may reach as a guest,
/// and its verified files (to install or load).
#[cfg(feature = "archive")]
#[derive(Debug)]
pub struct GuestPackage {
    pub manifest: crate::manifest::Manifest,
    /// [`guest_capabilities`] of its manifest's asks.
    pub capabilities: Capabilities,
    pub verified: architect_extension_archive::Verified,
}

/// Checks `bytes`, handed over as `package` of link `needs`.
///
/// For a guest on `platform` trusting `trust` (the org key, and the share
/// owner's publishers): the SHA-256 the link pinned, then the signature and
/// every file, then that it is the package's id and version and runs on
/// `platform`. The same check whichever way the bytes came (the node, a
/// peer).
///
/// # Errors
///
/// The first check that fails ([`PackageError`]).
#[cfg(feature = "archive")]
pub fn verify_package(
    bytes: &[u8],
    needs: &LinkNeeds,
    package: &LinkPackage,
    trust: &architect_extension_archive::Trust,
    platform: &str,
) -> Result<GuestPackage, PackageError> {
    use sha2::Digest as _;
    let got = hex::encode(sha2::Sha256::digest(bytes));
    let expected = package.sha256.to_ascii_lowercase();
    if got != expected {
        return Err(PackageError::Sha256 { expected, got });
    }
    let verified =
        architect_extension_archive::open(bytes, trust).map_err(PackageError::Archive)?;
    let text = verified
        .manifest()
        .ok_or_else(|| PackageError::Manifest("the archive has no extension.toml".to_owned()))?;
    let manifest = crate::manifest::Manifest::parse(text)
        .map_err(|e| PackageError::Manifest(e.to_string()))?;
    if manifest.id != package.id
        || verified.index.id != package.id
        || manifest.version.to_string() != package.version
    {
        return Err(PackageError::Mismatch(format!(
            "the link's package is {} {}, the archive is {} {}",
            package.id, package.version, manifest.id, manifest.version
        )));
    }
    if let Some(why) = crate::platform::not_here(&manifest.platforms, platform) {
        return Err(PackageError::NotHere(format!(
            "{why}: not on {}",
            crate::platform::name(platform)
        )));
    }
    let capabilities = guest_capabilities(&manifest.capabilities, needs.hosts_of(&package.id));
    Ok(GuestPackage {
        manifest,
        capabilities,
        verified,
    })
}

#[cfg(test)]
mod tests {
    use super::{LinkMissing, LinkNeeds, file_for, guest_capabilities, plan_link};
    use crate::manifest::{Capabilities, Reach};
    use semver::Version;

    const TEXT: &str = r#"
[[extensions]]
id = "health.meal-plan"
name = "Meal plan"
version = "^1.2"

[[extensions]]
id = "setlist"
required = false

[[packages]]
id = "health.meal-plan"
version = "1.2.0"
archive = "health.meal-plan-1.2.0.archext"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"

[hosts]
"health.meal-plan" = ["api.example.com"]
"#;

    #[test]
    fn a_links_needs_reuse_requirements_read_and_plan() {
        let needs = LinkNeeds::from_toml(TEXT).unwrap();
        assert_eq!(needs.requirements.extensions.len(), 2);
        assert!(!needs.requirements.extensions[1].required);
        assert_eq!(needs.requirements.extensions[1].version, "*");
        assert_eq!(needs.hosts_of("health.meal-plan"), ["api.example.com"]);
        assert_eq!(
            LinkNeeds::from_toml(&needs.to_toml().unwrap()).unwrap(),
            needs
        );

        let plan = plan_link(&needs, &[]);
        assert_eq!(plan.fetch.len(), 1);
        assert_eq!(plan.missing[0].1, LinkMissing::NotInCore);
        assert_eq!(plan.prompt().as_deref(), Some("This link uses Meal plan"));

        let here = vec![
            ("setlist".to_owned(), Version::new(0, 3, 0)),
            ("health.meal-plan".to_owned(), Version::new(1, 2, 5)),
        ];
        let plan = plan_link(&needs, &here);
        assert_eq!(plan.ready, ["health.meal-plan", "setlist"]);
        assert_eq!(plan.prompt(), None);
    }

    #[test]
    fn a_bad_entry_is_refused_with_its_name() {
        let zeros = "0000000000000000000000000000000000000000000000000000000000000000";
        for (bad, why) in [
            (
                TEXT.replace("health.meal-plan-1.2.0.archext", "../../x.archext"),
                "file name",
            ),
            (
                TEXT.replace("health.meal-plan-1.2.0.archext", "x.zip"),
                "file name",
            ),
            (TEXT.replace(zeros, ""), "sha256"),
            (
                TEXT.replace("[\"api.example.com\"]", "[\"*\"]"),
                "name each host",
            ),
            (
                TEXT.replace("id = \"setlist\"", "id = \"health.meal-plan\""),
                "twice",
            ),
            (
                TEXT.replace("version = \"1.2.0\"", "version = \"2.0.0\""),
                "not a version the link takes",
            ),
            (
                TEXT.replace("\"health.meal-plan\" = [", "\"other\" = ["),
                "does not name it",
            ),
            (format!("{TEXT}{}", "#".repeat(70 * 1024)), "more than"),
        ] {
            let e = LinkNeeds::from_toml(&bad).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        }
    }

    #[test]
    fn an_audiences_link_takes_its_shares_file_unless_it_has_its_own() {
        assert_eq!(
            file_for("abc", "abc", |_| false),
            "_architect/shares/abc.extensions.toml"
        );
        assert_eq!(
            file_for("viol", "abc", |_| false),
            "_architect/shares/abc.extensions.toml"
        );
        assert_eq!(
            file_for("viol", "abc", |_| true),
            "_architect/shares/viol.extensions.toml"
        );
    }

    #[test]
    fn a_guest_reads_the_share_and_fetches_only_approved_hosts() {
        let asks = Capabilities {
            reads: vec!["documents".into(), "org".into(), "files".into()],
            writes: vec!["documents".into()],
            hosts: vec!["api.example.com".into(), "tracker.example.net".into()],
            ui: vec!["notify".into(), "clipboard".into(), "pickers".into()],
            actions: vec!["TRANSPORT_".into()],
            effect: Reach::Destructive,
            folders: vec!["/tmp".into()],
            ..Capabilities::default()
        };
        let caps = guest_capabilities(
            &asks,
            &["api.example.com".to_owned(), "never.asked.org".to_owned()],
        );
        assert_eq!(caps.reads, ["documents"]);
        assert!(caps.writes.is_empty());
        assert_eq!(caps.hosts, ["api.example.com"]);
        assert_eq!(caps.ui, ["notify"]);
        assert!(caps.actions.is_empty() && caps.folders.is_empty());
        assert_eq!(caps.effect, Reach::ReadOnly);
    }
}
