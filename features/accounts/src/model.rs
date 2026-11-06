//! The accounts and the active one: what a switcher shows, with no secret
//! in it.

use facet::Facet;

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
}
