//! What a project needs, and what an organization allows
//! (`docs/content/architecture/extensions.md`, "What a project needs").
//!
//! A project names the extensions it uses — each id, the versions it was
//! made with, and whether it is **required** (its sound or structure
//! depends on it) or **optional** (a convenience). Opening it, or joining
//! its live session, makes a [`Plan`]: what is here already, what to
//! install (one prompt for all of it), what to offer, and what cannot be
//! had — not in any registry, no version that fits, or not on the
//! organization's approved list ([`Policy`]).

use std::collections::BTreeMap;

use architect_orgs::{OrgId, Orgs};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::index::Listing;

/// One extension a project uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,
    /// What it is called, for the prompt ("This song uses Saturation").
    #[serde(default)]
    pub name: String,
    /// The versions it works with (`^1.2`); `*` for any.
    #[serde(default = "star")]
    pub version: String,
    /// The project depends on it; an optional one is offered, not asked.
    #[serde(default = "yes")]
    pub required: bool,
}

fn star() -> String {
    "*".to_owned()
}

const fn yes() -> bool {
    true
}

impl Requirement {
    /// A required extension, any version.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: String::new(),
            version: star(),
            required: true,
        }
    }

    #[must_use]
    pub fn versions(mut self, req: impl Into<String>) -> Self {
        self.version = req.into();
        self
    }

    #[must_use]
    pub const fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Its version range; `*` when it does not read.
    #[must_use]
    pub fn req(&self) -> VersionReq {
        VersionReq::parse(&self.version).unwrap_or(VersionReq::STAR)
    }
}

/// The extensions a project uses: kept in its document's `meta`
/// (`crdt` feature: [`crate::doc`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirements {
    pub extensions: Vec<Requirement>,
}

impl Requirements {
    /// Adds or replaces the requirement of the same id.
    pub fn need(&mut self, requirement: Requirement) {
        self.extensions.retain(|r| r.id != requirement.id);
        self.extensions.push(requirement);
    }

    /// As JSON text.
    ///
    /// # Errors
    ///
    /// The encoder's message.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }

    /// From JSON text.
    ///
    /// # Errors
    ///
    /// It is not JSON, or not requirements.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }
}

/// An organization's rule for extensions: those it approved (each with
/// the versions it approved), and whether members may install others.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    /// Approved ids, each with its version range (`*` for any).
    pub approved: BTreeMap<String, String>,
    /// Only the approved may be installed.
    pub only_approved: bool,
}

impl Policy {
    /// Members may install anything; `approved` is a recommendation.
    #[must_use]
    pub fn open() -> Self {
        Self::default()
    }

    /// Only what is approved.
    #[must_use]
    pub const fn only_approved() -> Self {
        Self {
            approved: BTreeMap::new(),
            only_approved: true,
        }
    }

    #[must_use]
    pub fn approve(mut self, id: impl Into<String>, versions: impl Into<String>) -> Self {
        self.approved.insert(id.into(), versions.into());
        self
    }

    /// Whether `id` at `version` may be installed: `Err(why)` when not.
    ///
    /// # Errors
    ///
    /// It is not approved, or not at this version.
    pub fn allows(&self, id: &str, version: &Version) -> Result<(), String> {
        match self.approved.get(id) {
            Some(range) => {
                let req = VersionReq::parse(range).unwrap_or(VersionReq::STAR);
                if req.matches(version) {
                    Ok(())
                } else {
                    Err(format!(
                        "the organization approved {id} {range}, not {version}"
                    ))
                }
            }
            None if self.only_approved => {
                Err(format!("{id} is not on the organization's approved list"))
            }
            None => Ok(()),
        }
    }
}

/// Each organization's [`Policy`], by its id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policies {
    pub by_org: BTreeMap<OrgId, Policy>,
}

impl Policies {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with(mut self, org: OrgId, policy: Policy) -> Self {
        self.by_org.insert(org, policy);
        self
    }

    /// The policy where the window is: its org's, none in All (or an org
    /// with no policy).
    #[must_use]
    pub fn current(&self, orgs: &Orgs) -> Option<&Policy> {
        orgs.current().and_then(|o| self.by_org.get(&o.id))
    }
}

/// Why a project's extension cannot be had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Missing {
    /// No registry has it.
    NotFound,
    /// No version that the project's range takes.
    NoVersion { range: String },
    /// The organization does not allow it.
    NotApproved(String),
    /// It cannot run here: its kind (a process on a phone), or a process
    /// with builds, none for this platform (`process: no build for …`).
    CannotRun(String),
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("no registry here has it"),
            Self::NoVersion { range } => write!(f, "no version here fits {range}"),
            Self::NotApproved(why) => f.write_str(why),
            Self::CannotRun(why) => write!(f, "it cannot run here ({why})"),
        }
    }
}

/// What opening (or joining) a project takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Here already, at a version that fits.
    pub ready: Vec<String>,
    /// Required, and to be installed: one prompt, then loaded.
    pub install: Vec<Listing>,
    /// Optional, and installable: offered, not asked.
    pub offer: Vec<Listing>,
    /// Cannot be had: the project still opens, the extension's data is
    /// kept, a required one's views say what they need.
    pub missing: Vec<(Requirement, Missing)>,
}

impl Plan {
    /// Whether the project opens with nothing to ask.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.install.is_empty() && self.missing.iter().all(|(r, _)| !r.required)
    }

    /// The prompt's line: "This project uses Saturation and Lyrics Lane".
    #[must_use]
    pub fn prompt(&self) -> Option<String> {
        self.prompt_for("project")
    }

    /// [`Self::prompt`] for `what` opens: "This vault uses Word Count".
    #[must_use]
    pub fn prompt_for(&self, what: &str) -> Option<String> {
        let names: Vec<&str> = self.install.iter().map(|l| l.name.as_str()).collect();
        match names.as_slice() {
            [] => None,
            [one] => Some(format!("This {what} uses {one}")),
            [rest @ .., last] => Some(format!("This {what} uses {} and {last}", rest.join(", "))),
        }
    }
}

/// What opening a project with `requirements` takes, given what is
/// `here` (each loaded id and version), what the registries `offer`,
/// whether a kind `runs` here, and the organization's `policy`.
#[must_use]
pub fn plan(
    requirements: &Requirements,
    here: &[(String, Version)],
    offer: &[Listing],
    runs: impl Fn(&str) -> bool,
    policy: Option<&Policy>,
) -> Plan {
    let mut out = Plan::default();
    for r in &requirements.extensions {
        let req = r.req();
        if here.iter().any(|(id, v)| id == &r.id && req.matches(v)) {
            out.ready.push(r.id.clone());
            continue;
        }
        let mut versions: Vec<&Listing> = offer.iter().filter(|l| l.id == r.id).collect();
        if versions.is_empty() {
            out.missing.push((r.clone(), Missing::NotFound));
            continue;
        }
        versions.retain(|l| req.matches(&l.version));
        versions.sort_by(|a, b| b.version.cmp(&a.version));
        let Some(newest) = versions.first() else {
            out.missing.push((
                r.clone(),
                Missing::NoVersion {
                    range: r.version.clone(),
                },
            ));
            continue;
        };
        if !runs(&newest.kind) {
            out.missing
                .push((r.clone(), Missing::CannotRun(newest.kind.clone())));
            continue;
        }
        // A process with builds, none for this platform.
        if let Some(why) = newest.no_build_for(crate::index::host_targets()) {
            out.missing.push((
                r.clone(),
                Missing::CannotRun(format!("{}: {why}", newest.kind)),
            ));
            continue;
        }
        let allowed = versions
            .iter()
            .find(|l| policy.is_none_or(|p| p.allows(&l.id, &l.version).is_ok()));
        match allowed {
            Some(l) if r.required => out.install.push((*l).clone()),
            Some(l) => out.offer.push((*l).clone()),
            None => {
                let why = policy
                    .and_then(|p| p.allows(&newest.id, &newest.version).err())
                    .unwrap_or_default();
                out.missing.push((r.clone(), Missing::NotApproved(why)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Missing, Policies, Policy, Requirement, Requirements, plan};
    use crate::index::{Listing, Location};
    use architect_orgs::{Org, OrgId, Orgs, Role};
    use semver::Version;

    fn listing(id: &str, v: &str, kind: &str) -> Listing {
        Listing {
            id: id.into(),
            name: id.into(),
            version: Version::parse(v).unwrap(),
            kind: kind.into(),
            description: String::new(),
            category: String::new(),
            location: Location::Path(id.into()),
            platforms: Vec::new(),
            changelog: String::new(),
        }
    }

    fn needs() -> Requirements {
        let mut r = Requirements::default();
        r.need(
            Requirement::new("fts.metronome")
                .versions("^0.1")
                .named("Metronome"),
        );
        r.need(Requirement::new("acme.saturation").named("Saturation"));
        r.need(Requirement::new("acme.lyrics").optional());
        r.need(Requirement::new("acme.gone"));
        r
    }

    #[test]
    fn opening_a_project_installs_what_it_needs_and_offers_the_rest() {
        let offer = vec![
            listing("fts.metronome", "0.1.4", "luau"),
            listing("fts.metronome", "0.2.0", "luau"),
            listing("acme.saturation", "1.0.0", "wasm"),
            listing("acme.lyrics", "2.0.0", "data"),
        ];
        let p = plan(&needs(), &[], &offer, |_| true, None);
        assert_eq!(p.install.len(), 2);
        assert_eq!(
            p.install[0].version.to_string(),
            "0.1.4",
            "the newest that fits"
        );
        assert_eq!(p.offer.len(), 1);
        assert_eq!(
            p.missing,
            vec![(Requirement::new("acme.gone"), Missing::NotFound)]
        );
        assert_eq!(
            p.prompt().as_deref(),
            Some("This project uses fts.metronome and acme.saturation")
        );
        assert!(!p.is_ready());

        let here = vec![("fts.metronome".to_owned(), Version::new(0, 1, 0))];
        let p = plan(&needs(), &here, &offer, |k| k != "wasm", None);
        assert_eq!(p.ready, vec!["fts.metronome".to_owned()]);
        assert!(
            p.missing
                .iter()
                .any(|(r, m)| r.id == "acme.saturation" && *m == Missing::CannotRun("wasm".into()))
        );
    }

    #[test]
    fn an_organization_installs_only_what_it_approved() {
        let offer = vec![
            listing("fts.metronome", "0.1.4", "luau"),
            listing("fts.metronome", "0.1.2", "luau"),
            listing("acme.saturation", "1.0.0", "wasm"),
        ];
        let policy = Policy::only_approved().approve("fts.metronome", "<=0.1.2");
        let mut needs = Requirements::default();
        needs.need(Requirement::new("fts.metronome").versions("^0.1"));
        needs.need(Requirement::new("acme.saturation"));
        let p = plan(&needs, &[], &offer, |_| true, Some(&policy));
        assert_eq!(p.install.len(), 1);
        assert_eq!(
            p.install[0].version.to_string(),
            "0.1.2",
            "the newest it approved"
        );
        assert!(
            matches!(&p.missing[0].1, Missing::NotApproved(why) if why.contains("approved list"))
        );

        let orgs = Orgs::new(vec![Org::new("fts", "FTS", Role::Member)]);
        let policies = Policies::new().with(OrgId::new("fts"), policy.clone());
        assert_eq!(policies.current(&orgs), Some(&policy));
    }

    #[test]
    fn requirements_round_trip_as_json() {
        let r = needs();
        let json = r.to_json().unwrap();
        assert!(json.contains("\"fts.metronome\""));
        assert_eq!(Requirements::from_json(&json).unwrap(), r);
        let minimal = Requirements::from_json(r#"{"extensions":[{"id":"x"}]}"#).unwrap();
        assert_eq!(minimal.extensions[0].version, "*");
        assert!(minimal.extensions[0].required);
    }
}
