//! Sign-in in the Architect app (#188): the person's accounts at the issuer
//! (`architect_accounts`), their sessions in the platform keychain, the
//! Account item's switcher, the sign-in sheet, and `ACCOUNT_*` on the
//! app's Action List (MCP, the File menu, ⌘K).
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

use crate::mode::{Mode, app_dir_or_temp};

/// What the issuer's approval page calls the app.
const CLIENT_ID: &str = "Architect";

/// The app's accounts, `None` in the sandbox or a run that keeps nothing
/// (`files` false: a film, a test's document).
pub fn use_app_accounts(list: &Arc<ActionList>, files: bool) -> Option<AccountsUi> {
    // Fixed for the process: the hooks below run on every render or never.
    if Mode::current().is_sandbox() || !files {
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
