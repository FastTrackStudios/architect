//! The accounts and the active one: what a switcher shows, with no secret
//! in it.

use architect_orgs::{Org, Orgs, Role, Selection};
use facet::Facet;

/// The id of the org that is the person's own on this device (their
/// vault, their notes): what the app is in signed out, and kept first in
/// an account's orgs while the issuer has no personal org for it.
pub const DEVICE_ORG: &str = "me";

/// The person's own org on this device ([`DEVICE_ORG`]).
#[must_use]
pub fn device_org() -> Org {
    Org::new(DEVICE_ORG, "Personal", Role::Owner)
        .personal()
        .color("#047857")
}

/// The orgs an account shows: the issuer's, with [`device_org`] first
/// when none of them is the person's own.
#[must_use]
pub fn with_device_org(mut orgs: Vec<Org>) -> Vec<Org> {
    if !orgs.iter().any(|o| o.personal) {
        orgs.insert(0, device_org());
    }
    orgs
}

/// One account at an issuer, as this device knows it.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct Account {
    /// The person's id at the issuer (`AuthUser::id`).
    pub id: String,
    /// The issuer's origin (`https://auth.fasttrackstudio.app`).
    pub issuer: String,
    pub email: Option<String>,
    pub name: Option<String>,
    /// A picture URL, when the issuer has one.
    pub image: Option<String>,
    /// The issuer refused its session (expired, revoked elsewhere): listed
    /// still, and signing in again brings it back.
    pub needs_sign_in: bool,
    /// The orgs it is in, as last fetched from the issuer, and the one it
    /// is in now: kept per account, so switching accounts brings back
    /// the org each was in, and the switcher works offline. `None` until
    /// the first fetch.
    #[facet(default)]
    pub orgs: Option<Orgs>,
}

impl Account {
    /// The account's key: its id at its issuer. What the secret store keeps
    /// its session under and `ACCOUNT_SWITCH` names it by.
    #[must_use]
    pub fn key(&self) -> String {
        key_of(&self.issuer, &self.id)
    }

    /// What a person calls it: the name, else the email, else the id.
    #[must_use]
    pub fn label(&self) -> String {
        self.name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .or_else(|| self.email.clone())
            .unwrap_or_else(|| self.id.clone())
    }

    /// Its orgs and the current one: as last fetched, else only
    /// [`device_org`].
    #[must_use]
    pub fn orgs(&self) -> Orgs {
        self.orgs
            .clone()
            .unwrap_or_else(|| Orgs::new(vec![device_org()]))
    }

    /// Whether `who` names it: its key, id or email (any case).
    #[must_use]
    pub fn is(&self, who: &str) -> bool {
        let who = who.trim();
        self.key() == who
            || self.id == who
            || self
                .email
                .as_deref()
                .is_some_and(|e| e.eq_ignore_ascii_case(who))
    }
}

/// `<id>@<issuer host>`.
#[must_use]
pub fn key_of(issuer: &str, id: &str) -> String {
    let host = issuer
        .split_once("://")
        .map_or(issuer, |(_, rest)| rest)
        .trim_end_matches('/');
    format!("{id}@{host}")
}

/// The accounts file: every account this device holds, and the active one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet)]
pub struct AccountsFile {
    /// The active account's [`Account::key`]; `None` is signed out.
    pub active: Option<String>,
    pub accounts: Vec<Account>,
}

impl AccountsFile {
    /// The active account.
    #[must_use]
    pub fn active(&self) -> Option<&Account> {
        let key = self.active.as_deref()?;
        self.accounts.iter().find(|a| a.key() == key)
    }

    /// The account `who` names (its key, id or email).
    #[must_use]
    pub fn find(&self, who: &str) -> Option<&Account> {
        self.accounts
            .iter()
            .find(|a| a.key() == who.trim())
            .or_else(|| self.accounts.iter().find(|a| a.is(who)))
    }

    /// Adds `account`, or updates it in place, and makes it the active one.
    pub fn upsert_active(&mut self, account: Account) {
        let key = account.key();
        if let Some(had) = self.accounts.iter_mut().find(|a| a.key() == key) {
            let mut account = account;
            // Signing in again keeps the org it was in.
            if account.orgs.is_none() {
                account.orgs = had.orgs.take();
            }
            *had = account;
        } else {
            self.accounts.push(account);
        }
        self.active = Some(key);
    }

    /// Takes `key` off; the next account becomes active when it was.
    pub fn remove(&mut self, key: &str) -> Option<Account> {
        let at = self.accounts.iter().position(|a| a.key() == key)?;
        let gone = self.accounts.remove(at);
        if self.active.as_deref() == Some(key) {
            self.active = self
                .accounts
                .iter()
                .find(|a| !a.needs_sign_in)
                .or_else(|| self.accounts.first())
                .map(Account::key);
        }
        Some(gone)
    }

    /// The orgs the active account is in and its current one; signed
    /// out, only [`device_org`].
    #[must_use]
    pub fn orgs(&self) -> Orgs {
        self.active()
            .map_or_else(|| Orgs::new(vec![device_org()]), Account::orgs)
    }

    /// Keeps `fresh` (the issuer's list) as `key`'s orgs; the org it was
    /// in stays current unless it is gone. `None` when no account is
    /// `key`.
    pub fn set_orgs(&mut self, key: &str, fresh: Vec<Org>) -> Option<Orgs> {
        let account = self.accounts.iter_mut().find(|a| a.key() == key)?;
        let fresh = with_device_org(fresh);
        let orgs = match account.orgs.take() {
            Some(mut had) => {
                had.refresh(fresh);
                had
            }
            None => Orgs::new(fresh),
        };
        account.orgs = Some(orgs.clone());
        Some(orgs)
    }

    /// Moves `key` to `selection`. `None` when no account is `key` or the
    /// selection names an org it is not in.
    pub fn switch_org(&mut self, key: &str, selection: Selection) -> Option<Orgs> {
        let account = self.accounts.iter_mut().find(|a| a.key() == key)?;
        let mut orgs = account.orgs();
        if !orgs.switch(selection) {
            return None;
        }
        account.orgs = Some(orgs.clone());
        Some(orgs)
    }

    /// As JSON.
    ///
    /// # Errors
    ///
    /// The encoder's message.
    pub fn to_json(&self) -> Result<String, String> {
        facet_json::to_string(self).map_err(|e| e.to_string())
    }

    /// From JSON.
    ///
    /// # Errors
    ///
    /// The decoder's message.
    pub fn from_json(json: &str) -> Result<Self, String> {
        facet_json::from_str(json).map_err(|e| e.to_string())
    }
}

/// One of the account's devices, from the issuer's address book
/// (`DeviceService::list_devices`).
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// `macos`, `linux`, `ios`, …
    pub platform: String,
    /// When it last told the issuer it is signed in (RFC 3339).
    pub last_seen: String,
    /// This device.
    pub this_one: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, email: &str) -> Account {
        Account {
            id: id.to_owned(),
            issuer: "https://auth.example.test".to_owned(),
            email: Some(email.to_owned()),
            name: None,
            image: None,
            needs_sign_in: false,
            orgs: None,
        }
    }

    #[test]
    fn an_account_is_named_by_key_id_or_email() {
        let a = account("u1", "Ada@Example.test");
        assert_eq!(a.key(), "u1@auth.example.test");
        assert!(a.is("u1@auth.example.test") && a.is("u1") && a.is("ada@example.test"));
        assert!(!a.is("u2"));
        assert_eq!(a.label(), "Ada@Example.test");
    }

    #[test]
    fn signing_in_again_updates_and_removing_the_active_one_moves_on() {
        let mut file = AccountsFile::default();
        file.upsert_active(account("u1", "ada@example.test"));
        file.upsert_active(account("u2", "grace@example.test"));
        file.upsert_active(account("u1", "ada@example.test"));
        assert_eq!(file.accounts.len(), 2);
        assert_eq!(file.active().map(|a| a.id.as_str()), Some("u1"));
        file.remove("u1@auth.example.test");
        assert_eq!(file.active().map(|a| a.id.as_str()), Some("u2"));
        file.remove("u2@auth.example.test");
        assert_eq!(file.active, None);
    }

    #[test]
    fn the_file_round_trips_and_holds_no_session() {
        let mut file = AccountsFile::default();
        file.upsert_active(account("u1", "ada@example.test"));
        let json = file.to_json().unwrap_or_default();
        assert!(!json.contains("token"), "{json}");
        assert_eq!(AccountsFile::from_json(&json).ok(), Some(file));
    }

    #[test]
    fn each_account_keeps_its_own_org() {
        let mut file = AccountsFile::default();
        assert_eq!(
            file.orgs().current().map(|o| o.id.0.clone()),
            Some(DEVICE_ORG.to_owned())
        );
        file.upsert_active(account("u1", "ada@example.test"));
        file.upsert_active(account("u2", "grace@example.test"));
        let acme = Org::new("o-acme", "Acme Records", Role::Admin);
        let indie = Org::new("o-indie", "Indie Collective", Role::Owner);
        file.set_orgs("u1@auth.example.test", vec![acme.clone()]);
        file.set_orgs("u2@auth.example.test", vec![acme.clone(), indie.clone()]);
        // No personal org at the issuer: the device's comes first.
        assert_eq!(file.orgs().orgs.len(), 3);
        assert!(
            file.switch_org("u2@auth.example.test", Selection::One(indie.id.clone()))
                .is_some()
        );
        assert!(
            file.switch_org("u1@auth.example.test", Selection::One(indie.id.clone()))
                .is_none()
        );
        file.active = Some("u1@auth.example.test".to_owned());
        assert_eq!(
            file.orgs().current().map(|o| o.id.0.clone()),
            Some(DEVICE_ORG.to_owned())
        );
        file.active = Some("u2@auth.example.test".to_owned());
        assert_eq!(file.orgs().current(), Some(&indie));
        // A fresh list keeps the current org; signing in again keeps both.
        file.set_orgs("u2@auth.example.test", vec![indie.clone(), acme]);
        file.upsert_active(account("u2", "grace@example.test"));
        assert_eq!(file.orgs().current(), Some(&indie));
        // Removed from it: back to the person's own.
        file.set_orgs("u2@auth.example.test", vec![]);
        assert_eq!(
            file.orgs().current().map(|o| o.id.0.clone()),
            Some(DEVICE_ORG.to_owned())
        );
        let json = file.to_json().unwrap_or_default();
        assert_eq!(AccountsFile::from_json(&json).ok(), Some(file));
        // A file from before orgs were kept still reads.
        let old = r#"{"active":null,"accounts":[{"id":"u1","issuer":"https://a.test","email":null,"name":null,"image":null,"needs_sign_in":false}]}"#;
        assert!(
            AccountsFile::from_json(old)
                .is_ok_and(|f| f.accounts.first().is_some_and(|a| a.orgs.is_none()))
        );
    }
}
