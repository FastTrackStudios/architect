//! A space in an org: its scope, members, roles and grants, the org's
//! policy over them, and the rules they compile to.

use facet::Facet;

use architect_orgs::{OrgId, Role};
use architect_permissions::{Action, Principal, Resource, Rule, glob_matches};

/// What a space's own resource starts with: `spaces/<id>`, which every
/// member may read and a manager administers.
pub const SPACES_PREFIX: &str = "spaces/";

/// A space's id, minted by the server.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
pub struct SpaceId(pub String);

impl SpaceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The space's own resource, org-relative: `spaces/<id>`.
    #[must_use]
    pub fn resource(&self) -> String {
        format!("{SPACES_PREFIX}{}", self.0)
    }
}

impl std::fmt::Display for SpaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who a membership is for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(C)]
pub enum Subject {
    /// A person, by their identity server user id: a member of the
    /// space's org or of another one, or of none.
    User(String),
    /// A link guest: the holder of a share link, by the link's id
    /// (`Principal::Guest`). Links mint them; a guest has no account.
    Guest(String),
}

impl Subject {
    /// The subject a principal is, if spaces can hold it: a user or a
    /// guest. Services, hosts and anonymous callers are no subject.
    #[must_use]
    pub fn of(who: &Principal) -> Option<Self> {
        match who {
            Principal::User { user_id } => Some(Self::User(user_id.clone())),
            Principal::Guest { link_id, .. } => Some(Self::Guest(link_id.clone())),
            Principal::Service { .. }
            | Principal::Host { .. }
            | Principal::Extension { .. }
            | Principal::Anonymous => None,
        }
    }

    /// As the gate describes the principal (`user:<id>`, `guest:<id>`).
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::User(id) => format!("user:{id}"),
            Self::Guest(id) => format!("guest:{id}"),
        }
    }
}

/// How a subject stands to the org that owns a space: what the org's
/// policy caps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// A member of the org, with their role there.
    Member(Role),
    /// A person from another org (or none).
    External,
    /// A link guest.
    Guest,
}

impl Standing {
    /// `subject`'s standing, given their role in the org (`None`: not a
    /// member).
    #[must_use]
    pub fn of(subject: &Subject, org_role: Option<Role>) -> Self {
        match (subject, org_role) {
            (Subject::Guest(_), _) => Self::Guest,
            (Subject::User(_), Some(role)) => Self::Member(role),
            (Subject::User(_), None) => Self::External,
        }
    }

    /// Whether this is an owner or admin of the org: they manage every
    /// space in it.
    #[must_use]
    pub const fn stewards(&self) -> bool {
        matches!(self, Self::Member(Role::Owner | Role::Admin))
    }
}

/// A role: a named set of actions, held over the space's whole scope.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct SpaceRole {
    pub name: String,
    /// Permission actions (`read`, `comment`, `write`, `download`,
    /// `admin`, an app's own); `*` is every action.
    pub actions: Vec<String>,
}

impl SpaceRole {
    pub fn new(name: impl Into<String>, actions: &[&str]) -> Self {
        Self {
            name: name.into(),
            actions: actions.iter().map(|a| (*a).to_owned()).collect(),
        }
    }
}

/// See what is in the space.
pub const VIEWER: &str = "viewer";
/// See it and comment on it.
pub const COMMENTER: &str = "commenter";
/// Work in it: read, comment, write, download.
pub const EDITOR: &str = "editor";
/// Everything, `admin` included: change the space and its members.
pub const MANAGER: &str = "manager";

/// The roles every space has, unless it defines one of the same name.
#[must_use]
pub fn builtin_roles() -> Vec<SpaceRole> {
    vec![
        SpaceRole::new(VIEWER, &[Action::READ]),
        SpaceRole::new(COMMENTER, &[Action::READ, Action::COMMENT]),
        SpaceRole::new(
            EDITOR,
            &[
                Action::READ,
                Action::COMMENT,
                Action::WRITE,
                Action::DOWNLOAD,
            ],
        ),
        SpaceRole::new(MANAGER, &["*"]),
    ]
}

/// Actions on one resource inside the space, for one member: a
/// collaborator given one document of a project, or `write` on one
/// folder where their role only reads.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct SpaceGrant {
    /// An org-relative resource pattern inside the space's scope.
    pub resource: String,
    pub actions: Vec<String>,
}

impl SpaceGrant {
    pub fn new(resource: impl Into<String>, actions: &[&str]) -> Self {
        Self {
            resource: resource.into(),
            actions: actions.iter().map(|a| (*a).to_owned()).collect(),
        }
    }
}

/// One member of a space.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Member {
    pub subject: Subject,
    /// Their role over the whole scope; `None` for a member who holds
    /// only their grants.
    pub role: Option<String>,
    pub grants: Vec<SpaceGrant>,
    /// Who added them (`user:<id>`).
    pub added_by: String,
    pub added_us: i64,
    /// Past this the membership conveys nothing (checked on every call;
    /// nothing sweeps).
    pub expires_us: Option<i64>,
}

impl Member {
    #[must_use]
    pub fn is_live(&self, now_us: i64) -> bool {
        self.expires_us.is_none_or(|at| at > now_us)
    }
}

/// A space: a scope in one org, and who may do what in it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Space {
    pub id: SpaceId,
    pub org: OrgId,
    pub name: String,
    /// What it is to the app (`project`, `wiki`, `folder`, `document`,
    /// `session`): shown, never interpreted here.
    pub kind: String,
    /// What it covers: org-relative resource patterns (`vault/Alpha/`,
    /// `doc/<id>`, `files/<root>/Mixes/`), the permission resources the
    /// org's services already check.
    pub scope: Vec<String>,
    /// Carve-outs inside the scope (`vault/Alpha/Invoices/`): no member
    /// holds anything there through this space.
    pub exclude: Vec<String>,
    /// The space's own roles; a name shared with a built-in replaces it.
    pub roles: Vec<SpaceRole>,
    pub members: Vec<Member>,
    pub created_by: String,
    pub created_us: i64,
}

impl Space {
    /// The actions role `name` carries here: the space's own role of that
    /// name, else the built-in.
    #[must_use]
    pub fn role_actions(&self, name: &str) -> Option<Vec<String>> {
        self.roles
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.actions.clone())
            .or_else(|| {
                builtin_roles()
                    .into_iter()
                    .find(|r| r.name == name)
                    .map(|r| r.actions)
            })
    }

    #[must_use]
    pub fn member(&self, subject: &Subject) -> Option<&Member> {
        self.members.iter().find(|m| &m.subject == subject)
    }

    /// Whether `pattern` lies inside the scope: every resource it names
    /// is one some scope pattern covers.
    #[must_use]
    pub fn covers(&self, pattern: &str) -> bool {
        let head = head(pattern);
        self.scope.iter().any(|s| glob_matches(s, head)) && !reaches_wider(pattern, &self.scope)
    }

    /// Whether `path` is carved out of the space.
    #[must_use]
    pub fn excludes(&self, path: &str) -> bool {
        self.exclude.iter().any(|e| glob_matches(e, path))
    }

    /// What `member` holds here, as rules: their role's actions over each
    /// scope pattern, their grants, and the space itself (`read`, and
    /// `admin` with a role that has it), each capped at `cap` (the org's
    /// policy for how they stand; `None` caps nothing). Carve-outs are
    /// not in the rules: [`OrgSpaces::allows`] applies them.
    #[must_use]
    pub fn rules_for(&self, member: &Member, cap: Option<&[String]>) -> Vec<Rule> {
        let mut rules = Vec::new();
        let mut own = vec![Action::READ.to_owned()];
        if let Some(actions) = member.role.as_deref().and_then(|r| self.role_actions(r)) {
            let actions = capped(&actions, cap);
            if actions.iter().any(|a| a == "*" || a == Action::ADMIN) {
                own.push(Action::ADMIN.to_owned());
            }
            if !actions.is_empty() {
                rules.extend(self.scope.iter().map(|s| Rule {
                    resource: s.clone(),
                    actions: actions.clone(),
                }));
            }
        }
        for grant in &member.grants {
            let actions = capped(&grant.actions, cap);
            if !actions.is_empty() {
                rules.push(Rule {
                    resource: grant.resource.clone(),
                    actions,
                });
            }
        }
        rules.push(Rule {
            resource: self.id.resource(),
            actions: own,
        });
        rules
    }
}

/// What an org allows its spaces to give, by how a member stands. Bound
/// on every check, so tightening it takes effect on the next call.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct SpacePolicy {
    /// People from other orgs may hold anything through a space.
    pub external_members: bool,
    /// Link guests may hold anything through a space.
    pub guests: bool,
    /// Members below admin may create spaces (owners and admins always
    /// may).
    pub members_create: bool,
    /// The most a space gives a person from another org; `*` caps
    /// nothing.
    pub external_cap: Vec<String>,
    /// The most a space gives a link guest.
    pub guest_cap: Vec<String>,
}

impl Default for SpacePolicy {
    /// People from other orgs may work in a space but not manage it;
    /// guests may read and comment.
    fn default() -> Self {
        Self {
            external_members: true,
            guests: true,
            members_create: true,
            external_cap: [
                Action::READ,
                Action::COMMENT,
                Action::WRITE,
                Action::DOWNLOAD,
            ]
            .map(str::to_owned)
            .to_vec(),
            guest_cap: [Action::READ, Action::COMMENT].map(str::to_owned).to_vec(),
        }
    }
}

impl SpacePolicy {
    /// Whether a space may give someone of this standing anything.
    #[must_use]
    pub const fn admits(&self, standing: &Standing) -> bool {
        match standing {
            Standing::Member(_) => true,
            Standing::External => self.external_members,
            Standing::Guest => self.guests,
        }
    }

    /// The cap on what a space gives this standing (`None`: uncapped).
    /// The org's own members are capped by nothing: their space role
    /// adds to their org role.
    #[must_use]
    pub fn cap(&self, standing: &Standing) -> Option<&[String]> {
        match standing {
            Standing::Member(_) => None,
            Standing::External => Some(&self.external_cap),
            Standing::Guest => Some(&self.guest_cap),
        }
    }
}

/// One org's spaces and its policy over them: the unit stored and
/// swapped whole on every change.
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct OrgSpaces {
    pub policy: SpacePolicy,
    pub spaces: Vec<Space>,
}

/// What one subject holds in one space.
#[derive(Debug, Clone)]
pub struct Holding<'a> {
    pub space: &'a Space,
    pub member: &'a Member,
    pub rules: Vec<Rule>,
}

impl OrgSpaces {
    #[must_use]
    pub fn space(&self, id: &SpaceId) -> Option<&Space> {
        self.spaces.iter().find(|s| &s.id == id)
    }

    pub fn space_mut(&mut self, id: &SpaceId) -> Option<&mut Space> {
        self.spaces.iter_mut().find(|s| &s.id == id)
    }

    /// Every space `subject` is a live member of, and what they hold in
    /// it under the policy. Nothing at all for a standing the policy
    /// does not admit.
    #[must_use]
    pub fn holdings(
        &self,
        subject: &Subject,
        standing: &Standing,
        now_us: i64,
    ) -> Vec<Holding<'_>> {
        if !self.policy.admits(standing) {
            return Vec::new();
        }
        let cap = self.policy.cap(standing);
        self.spaces
            .iter()
            .filter_map(|space| {
                let member = space.member(subject).filter(|m| m.is_live(now_us))?;
                Some(Holding {
                    space,
                    member,
                    rules: space.rules_for(member, cap),
                })
            })
            .collect()
    }

    /// Whether `subject` may do `action` on the org-relative `resource`
    /// through any of the org's spaces.
    ///
    /// A resource with a wildcard (`notes/**`: the gate's method-level
    /// question) is allowed when they may do it to *something* it covers;
    /// the handler then checks the exact resource. A concrete resource
    /// must be inside a rule and outside the space's carve-outs.
    #[must_use]
    pub fn allows(
        &self,
        subject: &Subject,
        standing: &Standing,
        resource: &str,
        action: &str,
        now_us: i64,
    ) -> bool {
        let action = Action::new(action);
        let coarse = is_pattern(resource);
        let what = Resource::new(resource);
        self.holdings(subject, standing, now_us).iter().any(|h| {
            if coarse {
                h.rules
                    .iter()
                    .any(|r| has_action(r, &action) && overlaps(&r.resource, resource))
            } else {
                !h.space.excludes(resource) && h.rules.iter().any(|r| r.permits(&what, &action))
            }
        })
    }
}

/// Whether a resource is a pattern (`notes/**`, `vault/Alpha/`) rather
/// than one resource.
#[must_use]
pub fn is_pattern(resource: &str) -> bool {
    resource.contains('*') || resource.ends_with('/')
}

/// A pattern's fixed part: everything before its trailing wildcard or
/// prefix slash (`vault/Alpha/**` and `vault/Alpha/` → `vault/Alpha`;
/// `**` → empty).
#[must_use]
pub fn head(pattern: &str) -> &str {
    let p = pattern.strip_suffix("**").unwrap_or(pattern);
    p.strip_suffix('/').unwrap_or(p)
}

/// Whether two patterns share any resource: one's fixed part is inside
/// the other. Conservative for mid-pattern wildcards, which the gate's
/// method-level question is only ever a pre-filter for.
#[must_use]
pub fn overlaps(a: &str, b: &str) -> bool {
    let (ha, hb) = (head(a), head(b));
    ha.is_empty() || hb.is_empty() || glob_matches(a, hb) || glob_matches(b, ha)
}

fn has_action(rule: &Rule, action: &Action) -> bool {
    rule.actions
        .iter()
        .any(|a| a == "*" || a == action.as_str())
}

/// Whether `pattern` names resources beyond its fixed part that no
/// scope pattern reaches: `vault/Alpha/**` inside a scope of the single
/// document `vault/Alpha` does.
fn reaches_wider(pattern: &str, scope: &[String]) -> bool {
    let open = is_pattern(pattern);
    open && !scope
        .iter()
        .any(|s| is_pattern(s) && (head(s).is_empty() || glob_matches(s, head(pattern))))
}

/// `actions` limited to `cap` (`None` or a `*` in it: unlimited). A `*`
/// in `actions` becomes the cap itself.
#[must_use]
pub fn capped(actions: &[String], cap: Option<&[String]>) -> Vec<String> {
    let Some(cap) = cap.filter(|c| !c.iter().any(|a| a == "*")) else {
        return actions.to_vec();
    };
    if actions.iter().any(|a| a == "*") {
        return cap.to_vec();
    }
    actions
        .iter()
        .filter(|a| cap.contains(a))
        .cloned()
        .collect()
}

/// What a person asks for when they create a space.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct NewSpace {
    pub name: String,
    pub kind: String,
    pub scope: Vec<String>,
    pub exclude: Vec<String>,
}

/// A change to a space's name or reach; `None` leaves a field as it is.
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct SpacePatch {
    pub name: Option<String>,
    pub scope: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
}

/// One space as a list shows it, with the caller's role in it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct SpaceSummary {
    pub id: SpaceId,
    pub name: String,
    pub kind: String,
    /// The caller's role; `None` when they hold only grants, or manage
    /// it as the org's owner or admin without being a member.
    pub role: Option<String>,
    pub members: u32,
}

/// What changed, in an audit entry.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum SpaceChange {
    Created {
        name: String,
        scope: Vec<String>,
    },
    Updated {
        patch: SpacePatch,
    },
    Deleted,
    MemberSet {
        subject: Subject,
        role: Option<String>,
    },
    MemberRemoved {
        subject: Subject,
    },
    Granted {
        subject: Subject,
        grant: SpaceGrant,
    },
    GrantRevoked {
        subject: Subject,
        resource: String,
    },
    RoleSet {
        role: SpaceRole,
    },
    RoleRemoved {
        name: String,
    },
    PolicySet {
        policy: SpacePolicy,
    },
}

/// One change to an org's spaces: who made it, when, and what it was.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct SpaceAudit {
    pub at_us: i64,
    /// The space; `None` for an org-wide change (its policy).
    pub space: Option<SpaceId>,
    /// Who (`user:<id>`, `service:<name>`).
    pub by: String,
    pub change: SpaceChange,
}

/// Why a spaces call failed.
#[derive(Debug, Clone, PartialEq, Eq, Facet, thiserror::Error)]
#[repr(u8)]
pub enum SpacesFault {
    #[error("the call is in no org")]
    NoOrg,
    #[error("no caller: spaces need a person")]
    Unauthenticated,
    /// No such space, or one the caller may not see.
    #[error("no space {0}")]
    NotFound(SpaceId),
    #[error("not permitted: {0}")]
    Denied(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("the org's policy refuses it: {0}")]
    Policy(String),
    #[error("spaces: {0}")]
    Internal(String),
}

/// Check a space's reach: a scope of org-relative patterns, carve-outs
/// inside it.
///
/// Files grants are whole folders, so a carve-out cannot sit inside a
/// files scope pattern: the files feature would still convey it.
pub fn validate_reach(scope: &[String], exclude: &[String]) -> Result<(), SpacesFault> {
    if scope.is_empty() {
        return Err(SpacesFault::Invalid("a space covers something".into()));
    }
    for pattern in scope.iter().chain(exclude) {
        validate_pattern(pattern)?;
    }
    for carve in exclude {
        let head = head(carve);
        if !scope.iter().any(|s| glob_matches(s, head)) {
            return Err(SpacesFault::Invalid(format!(
                "{carve} is not inside the space"
            )));
        }
        if scope
            .iter()
            .any(|s| s.starts_with(FILES_PREFIX) && glob_matches(s, head))
        {
            return Err(SpacesFault::Invalid(format!(
                "{carve} is inside a files folder, and a files grant is the whole folder"
            )));
        }
    }
    Ok(())
}

/// What the files feature's resources start with.
pub const FILES_PREFIX: &str = "files/";

/// One org-relative resource pattern.
pub fn validate_pattern(pattern: &str) -> Result<(), SpacesFault> {
    let bad = |why: &str| Err(SpacesFault::Invalid(format!("{pattern:?}: {why}")));
    if pattern.trim().is_empty() {
        return bad("empty");
    }
    if pattern.starts_with('/') || OrgId::of_path(pattern).is_some() {
        return bad("resources are org-relative (`vault/…`, not `/…` or `org/<id>/…`)");
    }
    if pattern.starts_with(SPACES_PREFIX) {
        return bad("`spaces/` is the spaces' own");
    }
    if pattern.split('/').any(|s| s == "..") {
        return bad("no `..`");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(scope: &[&str], exclude: &[&str]) -> Space {
        Space {
            id: SpaceId::new("s1"),
            org: OrgId::new("hp"),
            name: "Alpha".into(),
            kind: "project".into(),
            scope: scope.iter().map(|s| (*s).to_owned()).collect(),
            exclude: exclude.iter().map(|s| (*s).to_owned()).collect(),
            roles: Vec::new(),
            members: Vec::new(),
            created_by: "user:alice".into(),
            created_us: 0,
        }
    }

    fn member(subject: Subject, role: Option<&str>, grants: Vec<SpaceGrant>) -> Member {
        Member {
            subject,
            role: role.map(str::to_owned),
            grants,
            added_by: "user:alice".into(),
            added_us: 0,
            expires_us: None,
        }
    }

    fn carol() -> Subject {
        Subject::User("carol".into())
    }

    fn org_of(spaces: Vec<Space>) -> OrgSpaces {
        OrgSpaces {
            policy: SpacePolicy::default(),
            spaces,
        }
    }

    #[test]
    fn a_commenter_reads_and_comments_inside_and_nothing_outside() {
        let mut s = space(&["vault/Alpha/", "doc/d1"], &["vault/Alpha/Invoices/"]);
        s.members.push(member(carol(), Some(COMMENTER), Vec::new()));
        let org = org_of(vec![s]);
        let ext = Standing::External;
        let may = |what: &str, action: &str| org.allows(&carol(), &ext, what, action, 0);

        assert!(may("vault/Alpha/plan.md", "read"));
        assert!(may("vault/Alpha/plan.md", "comment"));
        assert!(may("doc/d1", "comment"));
        assert!(!may("vault/Alpha/plan.md", "write"));
        assert!(!may("vault/Beta/plan.md", "read"));
        assert!(!may("doc/d2", "read"));
        // Carved out.
        assert!(!may("vault/Alpha/Invoices/q3.md", "read"));
        // The space itself: read, not admin.
        assert!(may("spaces/s1", "read"));
        assert!(!may("spaces/s1", "admin"));
    }

    #[test]
    fn a_coarse_question_is_allowed_when_something_under_it_is() {
        let mut s = space(&["vault/Alpha/"], &[]);
        s.members.push(member(carol(), Some(COMMENTER), Vec::new()));
        let org = org_of(vec![s]);
        let may =
            |what: &str, action: &str| org.allows(&carol(), &Standing::External, what, action, 0);
        assert!(may("vault/**", "read"));
        assert!(may("**", "comment"));
        assert!(may("spaces/**", "read"));
        assert!(may("vault/Alpha/sub/**", "read"));
        assert!(!may("vault/**", "write"));
        assert!(!may("files/**", "read"));
    }

    #[test]
    fn the_policy_caps_outsiders_and_guests_but_not_the_orgs_members() {
        let mut s = space(&["vault/Alpha/"], &[]);
        s.members.push(member(carol(), Some(MANAGER), Vec::new()));
        let guest = Subject::Guest("l1".into());
        s.members
            .push(member(guest.clone(), Some(EDITOR), Vec::new()));
        let bob = Subject::User("bob".into());
        s.members
            .push(member(bob.clone(), Some(EDITOR), Vec::new()));
        let mut org = org_of(vec![s]);
        let at = "vault/Alpha/a.md";

        // A manager from another org: everything up to the cap — no admin.
        assert!(org.allows(&carol(), &Standing::External, at, "write", 0));
        assert!(!org.allows(&carol(), &Standing::External, "spaces/s1", "admin", 0));
        // The same role held by a member of the org would manage it.
        assert!(org.allows(
            &carol(),
            &Standing::Member(Role::Viewer),
            "spaces/s1",
            "admin",
            0
        ));
        // A guest editor only reads and comments.
        assert!(org.allows(&guest, &Standing::Guest, at, "comment", 0));
        assert!(!org.allows(&guest, &Standing::Guest, at, "write", 0));
        // An org viewer made an editor here writes here: the space adds.
        assert!(org.allows(&bob, &Standing::Member(Role::Viewer), at, "write", 0));

        // Turning outsiders off is retroactive.
        org.policy.external_members = false;
        assert!(!org.allows(&carol(), &Standing::External, at, "read", 0));
        org.policy.guests = false;
        assert!(!org.allows(&guest, &Standing::Guest, at, "read", 0));
    }

    #[test]
    fn a_grant_reaches_one_resource_and_an_expired_member_nothing() {
        let mut s = space(&["vault/Alpha/"], &[]);
        s.members.push(member(
            carol(),
            None,
            vec![SpaceGrant::new(
                "vault/Alpha/brief.md",
                &["read", "comment"],
            )],
        ));
        let mut dan = member(Subject::User("dan".into()), Some(EDITOR), Vec::new());
        dan.expires_us = Some(10);
        s.members.push(dan);
        let org = org_of(vec![s]);
        let ext = Standing::External;
        assert!(org.allows(&carol(), &ext, "vault/Alpha/brief.md", "comment", 0));
        assert!(!org.allows(&carol(), &ext, "vault/Alpha/plan.md", "read", 0));
        let dan = Subject::User("dan".into());
        assert!(org.allows(&dan, &ext, "vault/Alpha/x", "write", 9));
        assert!(!org.allows(&dan, &ext, "vault/Alpha/x", "read", 10));
    }

    #[test]
    fn a_space_role_replaces_a_builtin_and_adds_new_ones() {
        let mut s = space(&["doc/d1"], &[]);
        s.roles.push(SpaceRole::new(VIEWER, &["read", "download"]));
        s.roles
            .push(SpaceRole::new("reviewer", &["read", "approve"]));
        assert_eq!(
            s.role_actions(VIEWER),
            Some(vec!["read".to_owned(), "download".to_owned()])
        );
        assert_eq!(
            s.role_actions(EDITOR).map(|a| a.len()),
            Some(4),
            "untouched built-ins stay"
        );
        s.members
            .push(member(carol(), Some("reviewer"), Vec::new()));
        let org = org_of(vec![s]);
        assert!(!org.allows(&carol(), &Standing::External, "doc/d1", "approve", 0));
        // `approve` is outside the default external cap; an org member holds it.
        assert!(org.allows(
            &carol(),
            &Standing::Member(Role::Member),
            "doc/d1",
            "approve",
            0
        ));
    }

    #[test]
    fn covers_keeps_grants_inside_the_scope() {
        let s = space(&["vault/Alpha/", "doc/d1"], &[]);
        assert!(s.covers("vault/Alpha/brief.md"));
        assert!(s.covers("vault/Alpha/sub/"));
        assert!(s.covers("vault/Alpha/**"));
        assert!(s.covers("doc/d1"));
        assert!(
            !s.covers("doc/d1/**"),
            "a single document does not cover a subtree"
        );
        assert!(!s.covers("vault/**"));
        assert!(!s.covers("vault/Beta/x"));
        assert!(!s.covers("**"));
    }

    #[test]
    fn reach_is_org_relative_and_carve_outs_stay_inside() {
        let ok = |scope: &[&str], exclude: &[&str]| {
            validate_reach(
                &scope.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
                &exclude.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            )
            .is_ok()
        };
        assert!(ok(&["vault/Alpha/"], &["vault/Alpha/Invoices/"]));
        assert!(!ok(&[], &[]));
        assert!(!ok(&["org/fts/vault/"], &[]));
        assert!(!ok(&["/vault"], &[]));
        assert!(!ok(&["spaces/x"], &[]));
        assert!(!ok(&["vault/../x"], &[]));
        assert!(!ok(&["vault/Alpha/"], &["vault/Beta/"]));
        assert!(!ok(&["files/r1/Mixes/"], &["files/r1/Mixes/old/"]));
    }

    #[test]
    fn capping() {
        let all = vec!["*".to_owned()];
        let rc: Vec<String> = vec!["read".into(), "comment".into()];
        assert_eq!(capped(&all, Some(&rc)), rc);
        assert_eq!(capped(&rc, Some(&all)), rc);
        assert_eq!(capped(&rc, None), rc);
        assert_eq!(
            capped(&["write".to_owned()], Some(&rc)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn overlap() {
        assert!(overlaps("vault/Alpha/", "vault/**"));
        assert!(overlaps("**", "files/**"));
        assert!(overlaps("vault/", "vault/Alpha/**"));
        assert!(overlaps("doc/d1", "doc/**"));
        assert!(!overlaps("doc/d1", "vault/**"));
        assert!(!overlaps("vault/Alpha/", "vault/Beta/**"));
        assert_eq!(head("vault/Alpha/**"), "vault/Alpha");
        assert_eq!(head("**"), "");
        assert!(is_pattern("vault/") && is_pattern("a/*/b") && !is_pattern("doc/d1"));
    }

    #[test]
    fn subjects_are_users_and_guests() {
        assert_eq!(
            Subject::of(&Principal::User {
                user_id: "u".into()
            }),
            Some(Subject::User("u".into()))
        );
        assert_eq!(
            Subject::of(&Principal::Guest {
                link_id: "l".into(),
                display: None
            })
            .map(|s| s.describe()),
            Some("guest:l".into())
        );
        assert_eq!(Subject::of(&Principal::Service { name: "x".into() }), None);
        assert_eq!(Subject::of(&Principal::Anonymous), None);
        assert_eq!(
            Standing::of(&carol(), Some(Role::Admin)),
            Standing::Member(Role::Admin)
        );
        assert!(Standing::Member(Role::Owner).stewards());
        assert!(!Standing::External.stewards());
    }
}
