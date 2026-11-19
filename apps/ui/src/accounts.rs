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
use architect_accounts::{AccountStore, Accounts, Issuer};
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
    let ui = use_accounts(Some(list), || {
        // The keychain in a release build; a file in a development build or
        // a test, which the keychain would keep asking the person about.
        let dir = app_dir_or_temp();
        let secrets = architect_accounts::secret_store(architect_accounts::DEFAULT_SERVICE, &dir);
        let store = AccountStore::open(&dir, secrets);
        Accounts::new(
            Arc::new(store),
            Issuer::new(architect_accounts::issuer_from_env()),
            CLIENT_ID,
        )
    });
    // A development window with no active account signs in by itself, once.
    use_hook(move || sign_in_at_start(&ui.accounts()));
    Some(ui)
}

/// What a development window signs in as at start (the email and the
/// password), read when the window's accounts open; set by the binary
/// (`architect_desktop`'s `dev::sign_in_from_env`, which decides whether
/// this window may). Never set: nothing happens.
static DEV_SIGN_IN: std::sync::OnceLock<fn() -> Option<(String, String)>> =
    std::sync::OnceLock::new();

/// Names what reads the development window's own sign-in (the dev stack's
/// demo account; `apps/desktop/src/dev.rs` `sign_in_from_env`). Called once,
/// before the window opens; a second call is ignored.
pub fn set_dev_sign_in(read: fn() -> Option<(String, String)>) {
    let _ = DEV_SIGN_IN.set(read);
}

/// Signs the window in with the development sign-in, when one is set and
/// no account is active: the issuer's password sign-in, as the sheet's
/// form does it, in the background. A failure is said once and shown on
/// the sheet; the window carries on signed out.
fn sign_in_at_start(accounts: &Accounts) {
    if accounts.active().is_some() {
        return;
    }
    let Some((email, password)) = DEV_SIGN_IN.get().and_then(|read| read()) else {
        return;
    };
    let accounts = accounts.clone();
    spawn(async move {
        if let Err(why) = accounts.sign_in_password(&email, &password).await {
            tracing::warn!(dev.sign_in = "failed", %why, "the development window's sign-in failed; signed out");
        }
    });
}

/// Extensions' node actions (`runs = "node"`, #583), run on the current
/// org's node over its lane as the active account
/// (`architect_extensions::node::NodeActionsClient`). Who that is and
/// which node are read at each run, so a sign-in or an org switch is
/// followed with nothing to update.
pub struct OrgNode(pub Accounts);

impl architect_extensions::node::NodeLane for OrgNode {
    fn run(&self, ext: &str, action: &str, args: String) -> architect_extensions::node::NodeFuture {
        let lane = self.0.org_lane();
        let (ext, action) = (ext.to_owned(), action.to_owned());
        Box::pin(async move {
            let (host, endpoint) = lane?;
            let client: architect_extensions::node::NodeActionsClient =
                architect::connect::Pool::new()
                    .client(&endpoint)
                    .await
                    .map_err(|e| format!("{host}: {e}"))?;
            client
                .run(ext, action, args)
                .await
                .map_err(|e| e.to_string())
        })
    }
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
