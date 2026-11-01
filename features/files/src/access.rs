//! Who may do what, where: a member's role in the org, and grants on
//! folders.
//!
//! Capabilities are the primitive. A role is a bundle of them that holds
//! on every path of every root in the org ([`role_capabilities`]); a
//! [`Grant`] adds capabilities over one folder (its subtree) for one
//! person — a viewer who may write in `Deliverables`, a client given one
//! folder and nothing else. What a person holds at a path is the union
//! of the two ([`resolve`]).
//!
//! Outside every folder a person holds, a path is **absent**, not
//! forbidden ([`Access::Absent`]): "forbidden" would tell them it exists.
//! The folders above a grant are the one exception: the grantee can name
//! them (their grant does) and holds nothing there, so a browser can
//! walk down to the folder they may open.

use facet::Facet;

use architect_orgs::Role;

use crate::id::{GrantId, RootId};
use crate::path::RootPath;

/// One thing a person may do at a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(u8)]
pub enum Capability {
    /// See it, list it, read its bytes.
    Read,
    /// Save, make folders, move, copy, delete, restore, label versions.
    Write,
    /// Read past versions and their labels.
    History,
    /// Grant others what one holds here, and see who holds what.
    Share,
}

/// Every capability, in the order a set is reported.
pub const ALL: [Capability; 4] = [
    Capability::Read,
    Capability::Write,
    Capability::History,
    Capability::Share,
];

/// What a role that does not work in the org's files holds: look, and
/// look back.
pub const READER: [Capability; 2] = [Capability::Read, Capability::History];

/// What `role` holds on every path in the org: owners, admins and members
/// everything; viewers (and roles this build does not know) read.
#[must_use]
pub const fn role_capabilities(role: &Role) -> &'static [Capability] {
    match role {
        Role::Owner | Role::Admin | Role::Member => &ALL,
        Role::Viewer | Role::Other(_) => &READER,
    }
}

/// Whether `role` keeps the org's roots: renames and releases them.
#[must_use]
pub const fn stewards(role: &Role) -> bool {
    matches!(role, Role::Owner | Role::Admin)
}

/// Capabilities over a folder (and everything in it) for one person.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Grant {
    pub id: GrantId,
    pub root_id: RootId,
    /// The folder; the root itself for the whole root.
    pub path: RootPath,
    /// The person: their user id.
    pub user: String,
    pub capabilities: Vec<Capability>,
    pub granted_by: String,
    pub granted_us: i64,
    /// Past this it conveys nothing (checked on every read; nothing
    /// sweeps).
    pub expires_us: Option<i64>,
}

impl Grant {
    #[must_use]
    pub fn is_live(&self, now_us: i64) -> bool {
        self.expires_us.is_none_or(|at| at > now_us)
    }
}

/// What a caller may do at a path, resolved: the question a UI has.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Effective {
    pub root_id: RootId,
    pub path: RootPath,
    pub capabilities: Vec<Capability>,
}

/// What a person holds at one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// Nothing, and they may not know the path is there.
    Absent,
    /// These (possibly none: a folder above one of their grants).
    Holds(Vec<Capability>),
}

impl Access {
    #[must_use]
    pub fn allows(&self, cap: Capability) -> bool {
        matches!(self, Self::Holds(caps) if caps.contains(&cap))
    }

    #[must_use]
    pub const fn is_visible(&self) -> bool {
        matches!(self, Self::Holds(_))
    }

    #[must_use]
    pub fn capabilities(&self) -> Vec<Capability> {
        match self {
            Self::Absent => Vec::new(),
            Self::Holds(caps) => caps.clone(),
        }
    }
}

/// `caps` in [`ALL`]'s order, once each.
#[must_use]
pub fn canonical(mut caps: Vec<Capability>) -> Vec<Capability> {
    caps.sort_unstable();
    caps.dedup();
    caps
}

/// What `user` holds at `path` in `root`: `baseline` (their role's, empty
/// for a person in no role here) and every live grant of theirs whose
/// folder contains the path.
///
/// No baseline and no grant over the path: [`Access::Absent`] — unless a
/// grant of theirs lies beneath it, which makes it a folder they can name
/// and hold nothing in.
#[must_use]
pub fn resolve(
    baseline: &[Capability],
    grants: &[Grant],
    user: Option<&str>,
    root: RootId,
    path: &RootPath,
    now_us: i64,
) -> Access {
    let mut caps = baseline.to_vec();
    let (mut covered, mut above_a_grant) = (false, false);
    let theirs = grants
        .iter()
        .filter(|g| g.root_id == root && user.is_some_and(|u| g.user == u) && g.is_live(now_us));
    for grant in theirs {
        if path.is_within(&grant.path) {
            caps.extend(grant.capabilities.iter().copied());
            covered = true;
        } else if grant.path.is_within(path) {
            above_a_grant = true;
        }
    }
    if baseline.is_empty() && !covered && !above_a_grant {
        return Access::Absent;
    }
    Access::Holds(canonical(caps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn p(s: &str) -> RootPath {
        RootPath::parse(s).unwrap()
    }

    fn grant(root: RootId, path: &str, user: &str, caps: &[Capability]) -> Grant {
        Grant {
            id: GrantId::new(Uuid::from_u128(7)),
            root_id: root,
            path: p(path),
            user: user.into(),
            capabilities: caps.to_vec(),
            granted_by: "owner".into(),
            granted_us: 0,
            expires_us: None,
        }
    }

    #[test]
    fn a_role_holds_everywhere_in_the_org() {
        let root = RootId::new(Uuid::from_u128(1));
        let viewer = role_capabilities(&Role::Viewer);
        let at = resolve(viewer, &[], Some("v"), root, &p("any/where"), 0);
        assert!(at.allows(Capability::Read));
        assert!(!at.allows(Capability::Write));
        assert_eq!(role_capabilities(&Role::Member), &ALL);
        assert!(stewards(&Role::Admin) && !stewards(&Role::Member));
    }

    #[test]
    fn a_grant_adds_to_the_role_in_its_folder_only() {
        let root = RootId::new(Uuid::from_u128(1));
        let grants = [grant(root, "Deliverables", "v", &[Capability::Write])];
        let viewer = role_capabilities(&Role::Viewer);
        assert!(
            resolve(
                viewer,
                &grants,
                Some("v"),
                root,
                &p("Deliverables/mix.wav"),
                0
            )
            .allows(Capability::Write)
        );
        assert!(
            !resolve(viewer, &grants, Some("v"), root, &p("Stems"), 0).allows(Capability::Write)
        );
        // Another root, another person: the grant says nothing there.
        let other = RootId::new(Uuid::from_u128(2));
        assert!(
            !resolve(viewer, &grants, Some("v"), other, &p("Deliverables"), 0)
                .allows(Capability::Write)
        );
        assert!(
            !resolve(viewer, &grants, Some("w"), root, &p("Deliverables"), 0)
                .allows(Capability::Write)
        );
    }

    #[test]
    fn without_a_role_outside_a_grant_is_absent_and_above_it_empty() {
        let root = RootId::new(Uuid::from_u128(1));
        let grants = [grant(
            root,
            "Client/Deliverables",
            "guest",
            &[Capability::Read],
        )];
        let at = |path: &str| resolve(&[], &grants, Some("guest"), root, &p(path), 0);
        assert!(at("Client/Deliverables/a.wav").allows(Capability::Read));
        assert_eq!(at("Client"), Access::Holds(Vec::new()));
        assert_eq!(at(""), Access::Holds(Vec::new()));
        assert_eq!(at("Client/Stems"), Access::Absent);
        assert_eq!(at("Other"), Access::Absent);
    }

    #[test]
    fn an_expired_grant_conveys_nothing() {
        let root = RootId::new(Uuid::from_u128(1));
        let mut g = grant(root, "a", "guest", &[Capability::Read]);
        g.expires_us = Some(10);
        let grants = [g];
        assert!(resolve(&[], &grants, Some("guest"), root, &p("a"), 9).allows(Capability::Read));
        assert_eq!(
            resolve(&[], &grants, Some("guest"), root, &p("a"), 10),
            Access::Absent
        );
    }

    #[test]
    fn overlapping_grants_report_one_set_in_order() {
        let root = RootId::new(Uuid::from_u128(1));
        let grants = [
            grant(root, "a", "u", &[Capability::Share, Capability::Read]),
            grant(root, "a/b", "u", &[Capability::Read, Capability::Write]),
        ];
        assert_eq!(
            resolve(&[], &grants, Some("u"), root, &p("a/b/c"), 0),
            Access::Holds(vec![Capability::Read, Capability::Write, Capability::Share])
        );
    }
}
