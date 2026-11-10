//! Sign-in in the Architect app (#188): the person's accounts at the issuer
//! (`architect_accounts`), their sessions in the platform keychain, the
//! Account item's switcher, the sign-in sheet, and `ACCOUNT_*` on the
//! app's Action List (MCP, the File menu, ⌘K). The window's orgs
//! (`architect_orgs::use_org`) are the active account's
//! ([`use_account_orgs`]): the issuer's list, the current org kept per
//! account, switched from the Account menu, the sidebar's switcher or
//! `ORG_SWITCH`.
//!
//! The issuer is FTS's (`auth.fasttrackstudio.app`) unless
//! `ARCHITECT_AUTH_ISSUER` names another; the keychain's entries are the
//! app's (`app.fasttrackstudio.architect`) unless
//! `ARCHITECT_KEYCHAIN_SERVICE` names another. The accounts file
//! (`accounts.json`, no secrets) is in the app's folder. The sandbox keeps
//! its made-up person and has none of this.

use std::sync::Arc;

use architect::action::ActionList;
use architect_accounts::ui::{AccountsUi, use_accounts};
use architect_accounts::{AccountStore, Accounts, Issuer, Keychain};
use architect_orgs::{Capabilities, OrgHandle, Orgs, Selection};
use dioxus::prelude::*;

use crate::distribution::app_dir_or_temp;

/// The client the issuer knows the app as: `architect`, the same public
/// client the web app signs in with (fts-auth registers it for
/// `architect.fasttrackstudio.app`, #431). The device grant takes any id,
/// but one name everywhere keeps the issuer's records and its approval
/// page consistent.
const CLIENT_ID: &str = "architect";

/// The app's accounts, `None` in the sandbox or a run that keeps nothing
/// (`files` false: a film, a test's document).
pub fn use_app_accounts(list: &Arc<ActionList>, files: bool) -> Option<AccountsUi> {
    // Fixed for the process: the hooks below run on every render or never.
    if crate::distribution().is_sandbox() || !files {
        return None;
    }
    let list = list.clone();
    Some(use_accounts(Some(list), || {
        let secrets = Arc::new(Keychain::from_env(architect_accounts::DEFAULT_SERVICE));
        let store = AccountStore::open(&app_dir_or_temp(), secrets);
        Accounts::new(
            Arc::new(store),
            Issuer::new(architect_accounts::issuer_from_env()),
            CLIENT_ID,
        )
    }))
}

/// The window's orgs as the active account's: provided from its cached
/// list (signed out, the person's own on this device), a switch in the
/// window kept for the account, and the window following every change
/// the account's orgs see (a fetch, a switch from the menu or an action,
/// another account).
pub fn use_account_orgs(accounts: AccountsUi, caps: Capabilities) -> OrgHandle {
    let on_change = use_callback(move |orgs: Orgs| {
        let who = match &orgs.selection {
            Selection::One(id) => id.0.clone(),
            Selection::All => "all".to_owned(),
        };
        let _ = accounts.accounts().switch_org(&who);
    });
    let handle = architect_orgs::use_orgs_provider(|| accounts.orgs(), caps, Some(on_change));
    use_effect(move || handle.replace(accounts.orgs()));
    handle
}
