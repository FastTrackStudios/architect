//! A person's organizations, which one a window is in, and what their
//! role there lets them do.

use std::collections::{BTreeMap, BTreeSet};

use facet::Facet;

/// An organization's id: the identity server's (a UUID), shared by every
/// app.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
pub struct OrgId(pub String);

impl OrgId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// A member's role, the identity server's: the same in every app.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(C)]
pub enum Role {
    Owner,
    Admin,
    Member,
    Viewer,
    /// A role the server has that this build does not know.
    Other(String),
}

impl Role {
    /// From the identity server's role name.
    #[must_use]
    pub fn parse(name: &str) -> Self {
        match name.trim().to_lowercase().as_str() {
            "owner" => Self::Owner,
            "admin" => Self::Admin,
            "member" => Self::Member,
            "viewer" | "guest" => Self::Viewer,
            other => Self::Other(other.to_owned()),
        }
    }

    /// As shown (`Admin`).
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Owner => "Owner".to_owned(),
            Self::Admin => "Admin".to_owned(),
            Self::Member => "Member".to_owned(),
            Self::Viewer => "Viewer".to_owned(),
            Self::Other(n) => n.clone(),
        }
    }
}

/// One of the person's organizations.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Org {
    pub id: OrgId,
    pub name: String,
    pub slug: String,
    /// The person's role in it.
    pub role: Role,
    /// Their own: what they make before sharing it.
    pub personal: bool,
    pub members: u32,
    /// The mark's fill in a switcher (`#1d4ed8`).
    pub color: String,
}

impl Org {
    pub fn new(id: impl Into<String>, name: impl Into<String>, role: Role) -> Self {
        let name = name.into();
        let slug = name.to_lowercase().replace(' ', "-");
        Self {
            id: OrgId::new(id),
            name,
            slug,
            role,
            personal: false,
            members: 1,
            color: "#3f3f46".to_owned(),
        }
    }

    #[must_use]
    pub const fn personal(mut self) -> Self {
        self.personal = true;
        self
    }

    #[must_use]
    pub const fn members(mut self, n: u32) -> Self {
        self.members = n;
        self
    }

    #[must_use]
    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = color.into();
        self
    }
}

/// What a window is in: one organization, or all of them (for reading:
/// a library across them; writing always picks one).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(C)]
pub enum Selection {
    One(OrgId),
    All,
}

/// The app's capabilities by role: roles are the org's, what they let a
/// member do is the app's (`files.write`, `studio.mix`). A member may also
/// hold a grant of their own in one org.
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct Capabilities {
    pub by_role: BTreeMap<String, BTreeSet<String>>,
    /// Extra grants, by org id: the person's own in that org.
    pub grants: BTreeMap<String, BTreeSet<String>>,
}

impl Capabilities {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `role` may do each of `caps`.
    #[must_use]
    pub fn allow<I: IntoIterator<Item = S>, S: Into<String>>(
        mut self,
        role: &Role,
        caps: I,
    ) -> Self {
        self.by_role
            .entry(role.label())
            .or_default()
            .extend(caps.into_iter().map(Into::into));
        self
    }

    /// Owners and admins may do everything a member may, and owners
    /// everything an admin may: give each the lower roles' too.
    #[must_use]
    pub fn inherit(mut self) -> Self {
        let order = [Role::Viewer, Role::Member, Role::Admin, Role::Owner];
        let mut acc: BTreeSet<String> = BTreeSet::new();
        for role in order {
            let own = self.by_role.entry(role.label()).or_default();
            own.extend(acc.iter().cloned());
            acc.clone_from(own);
        }
        self
    }

    /// The person has `cap` in org `org` too.
    pub fn grant(&mut self, org: &OrgId, cap: impl Into<String>) {
        self.grants
            .entry(org.0.clone())
            .or_default()
            .insert(cap.into());
    }

    /// Whether `role` in `org` may do `cap`.
    #[must_use]
    pub fn can(&self, org: &OrgId, role: &Role, cap: &str) -> bool {
        self.by_role
            .get(&role.label())
            .is_some_and(|c| c.contains(cap))
            || self.grants.get(&org.0).is_some_and(|c| c.contains(cap))
    }
}

/// The person's orgs and what the window is in: kept on the device so
/// the switcher works offline, refreshed from the identity server.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Orgs {
    pub orgs: Vec<Org>,
    pub selection: Selection,
}

impl Default for Orgs {
    fn default() -> Self {
        Self {
            orgs: Vec::new(),
            selection: Selection::All,
        }
    }
}

impl Orgs {
    /// The orgs, the first personal one selected (or the first).
    #[must_use]
    pub fn new(orgs: Vec<Org>) -> Self {
        let first = orgs
            .iter()
            .find(|o| o.personal)
            .or_else(|| orgs.first())
            .map_or(Selection::All, |o| Selection::One(o.id.clone()));
        Self {
            orgs,
            selection: first,
        }
    }

    #[must_use]
    pub fn get(&self, id: &OrgId) -> Option<&Org> {
        self.orgs.iter().find(|o| &o.id == id)
    }

    /// The org `who` names: its id, its slug, or its name (any case).
    #[must_use]
    pub fn find(&self, who: &str) -> Option<&Org> {
        let who = who.trim();
        self.orgs
            .iter()
            .find(|o| o.id.0 == who || o.slug == who)
            .or_else(|| self.orgs.iter().find(|o| o.name.eq_ignore_ascii_case(who)))
    }

    /// The org the window is in, unless it is in All.
    #[must_use]
    pub fn current(&self) -> Option<&Org> {
        match &self.selection {
            Selection::One(id) => self.get(id),
            Selection::All => None,
        }
    }

    /// Moves the window to `selection`. False when it names an org the
    /// person is not in.
    pub fn switch(&mut self, selection: Selection) -> bool {
        if let Selection::One(id) = &selection {
            if self.get(id).is_none() {
                return false;
            }
        }
        self.selection = selection;
        true
    }

    /// The server's fresh list: kept; a selection whose org is gone moves
    /// to the personal org (the person left, or was removed).
    pub fn refresh(&mut self, orgs: Vec<Org>) {
        self.orgs = orgs;
        if let Selection::One(id) = &self.selection {
            if self.get(id).is_none() {
                *self = Self::new(std::mem::take(&mut self.orgs));
            }
        }
    }

    /// Whether the person may do `cap` where the window is (in All, only
    /// what they may do in every org).
    #[must_use]
    pub fn can(&self, caps: &Capabilities, cap: &str) -> bool {
        match &self.selection {
            Selection::One(id) => self.get(id).is_some_and(|o| caps.can(&o.id, &o.role, cap)),
            Selection::All => {
                !self.orgs.is_empty() && self.orgs.iter().all(|o| caps.can(&o.id, &o.role, cap))
            }
        }
    }

    /// The device's cache (facet JSON).
    ///
    /// # Errors
    ///
    /// The encoder's message.
    pub fn to_json(&self) -> Result<String, String> {
        facet_json::to_string(self).map_err(|e| e.to_string())
    }

    /// # Errors
    ///
    /// The decoder's message.
    pub fn from_json(json: &str) -> Result<Self, String> {
        facet_json::from_str(json).map_err(|e| e.to_string())
    }
}
