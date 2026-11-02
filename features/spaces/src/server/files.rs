//! A space over a folder grants its members that folder in the files
//! feature (feature `files`): generated from the space on every call,
//! never kept as files grants of their own.
//!
//! A scope pattern `files/<root id>/<path>` (`files/<root>/Mixes/`, a
//! whole root `files/<root>/`, one file `files/<root>/Mixes/final.wav`)
//! becomes a [`Grant`] on that path for each person who holds actions
//! there: `read` is `Read` and `History`, `write` is `Write`, `admin` is
//! `Share`, `*` all four. A pattern with a wildcard below the root names
//! no one folder and grants nothing in files.

use architect_files::server::GrantSource;
use architect_files::{Capability, Grant, GrantId, RootId, RootPath};
use architect_orgs::OrgId;
use architect_permissions::{Action, Principal, Rule};
use uuid::Uuid;

use super::Spaces;
use crate::model::{FILES_PREFIX, Holding, Subject};

/// The namespace a generated grant's id is derived in (a v5 UUID of
/// `architect.spaces.files`), so the same membership names the same
/// grant on every call.
const GRANT_NAMESPACE: Uuid = Uuid::from_bytes([
    0x2f, 0x9c, 0x41, 0x7e, 0x06, 0xd3, 0x5a, 0x8b, 0xb1, 0x4e, 0x93, 0x27, 0xc8, 0x5d, 0x10, 0x6a,
]);

impl GrantSource for Spaces {
    fn grants(&self, org: &OrgId, user: &str, now_us: i64) -> Vec<Grant> {
        let who = Principal::User {
            user_id: user.to_owned(),
        };
        let subject = Subject::User(user.to_owned());
        let standing = self.standing(&who, &subject, org);
        let state = self.snapshot(org);
        state
            .holdings(&subject, &standing, now_us)
            .iter()
            .flat_map(|h| h.rules.iter().filter_map(|r| to_grant(h, user, r)))
            .collect()
    }
}

/// The files grant `rule` is, for `user`, if it names a files path.
fn to_grant(holding: &Holding<'_>, user: &str, rule: &Rule) -> Option<Grant> {
    let rest = rule.resource.strip_prefix(FILES_PREFIX)?;
    let (root, path) = rest.split_once('/').unwrap_or((rest, ""));
    let root = RootId::new(Uuid::parse_str(root).ok()?);
    let path = path.strip_suffix("**").unwrap_or(path);
    let path = path.strip_suffix('/').unwrap_or(path);
    if path.contains('*') {
        return None;
    }
    let path = if path.is_empty() {
        RootPath::root()
    } else {
        RootPath::parse(path).ok()?
    };
    let capabilities = capabilities(&rule.actions);
    if capabilities.is_empty() {
        return None;
    }
    let id = Uuid::new_v5(
        &GRANT_NAMESPACE,
        format!("{}/{user}/{}", holding.space.id, rule.resource).as_bytes(),
    );
    Some(Grant {
        id: GrantId::new(id),
        root_id: root,
        path,
        user: user.to_owned(),
        capabilities,
        granted_by: format!("space:{}", holding.space.id),
        granted_us: holding.member.added_us,
        expires_us: holding.member.expires_us,
    })
}

/// What files capabilities permission actions are.
fn capabilities(actions: &[String]) -> Vec<Capability> {
    let mut caps = Vec::new();
    for action in actions {
        match action.as_str() {
            "*" => caps.extend(architect_files::access::ALL),
            Action::READ => caps.extend([Capability::Read, Capability::History]),
            Action::WRITE => caps.push(Capability::Write),
            Action::ADMIN => caps.push(Capability::Share),
            _ => {}
        }
    }
    architect_files::access::canonical(caps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Member, Space, SpaceId};

    fn holding_of<'a>(space: &'a Space, member: &'a Member) -> Holding<'a> {
        Holding {
            space,
            member,
            rules: Vec::new(),
        }
    }

    #[test]
    fn a_files_rule_becomes_a_grant_on_its_folder() {
        let root = Uuid::from_u128(9);
        let space = Space {
            id: SpaceId::new("s"),
            org: OrgId::new("hp"),
            name: "Mixes".into(),
            kind: "folder".into(),
            scope: Vec::new(),
            exclude: Vec::new(),
            roles: Vec::new(),
            members: Vec::new(),
            created_by: String::new(),
            created_us: 0,
        };
        let member = Member {
            subject: Subject::User("carol".into()),
            role: None,
            grants: Vec::new(),
            added_by: String::new(),
            added_us: 5,
            expires_us: Some(50),
        };
        let h = holding_of(&space, &member);
        let grant = |resource: String, actions: &[&str]| {
            to_grant(&h, "carol", &Rule::new(resource, actions))
        };

        let g = grant(format!("files/{root}/Mixes/"), &["read", "comment"]).unwrap();
        assert_eq!(g.root_id, RootId::new(root));
        assert_eq!(g.path.as_str(), "Mixes");
        assert_eq!(g.capabilities, vec![Capability::Read, Capability::History]);
        assert_eq!((g.granted_us, g.expires_us), (5, Some(50)));
        assert_eq!(g.granted_by, "space:s");
        // Stable across calls.
        assert_eq!(
            g.id,
            grant(format!("files/{root}/Mixes/"), &["read"]).unwrap().id
        );

        let whole = grant(format!("files/{root}/**"), &["*"]).unwrap();
        assert!(whole.path.is_root());
        assert_eq!(whole.capabilities, architect_files::access::ALL.to_vec());

        assert!(grant(format!("files/{root}/*/stems"), &["read"]).is_none());
        assert!(grant("files/not-a-uuid/x".into(), &["read"]).is_none());
        assert!(grant("vault/x".into(), &["read"]).is_none());
        assert!(grant(format!("files/{root}/x"), &["comment"]).is_none());
    }
}
