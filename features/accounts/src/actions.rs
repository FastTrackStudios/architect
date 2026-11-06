//! `ACCOUNT_*` on an Action List (feature `actions`): MCP, the menus and ⌘K
//! reach sign-in, switching, signing out and the devices through them.
//!
//! No action takes or answers a secret: a password is typed in the sheet
//! (`ACCOUNT_SIGN_IN` opens it), and no session token leaves the keychain
//! through the list.

use std::sync::Arc;

use architect::action::{ActionError, ActionList, ActionMeta, Effect};
use facet::Facet;

use crate::controller::{Accounts, Sheet};
use crate::model::{Account, Device};

/// The actions' ids.
pub mod ids {
    pub const LIST: &str = "ACCOUNT_LIST";
    pub const SIGN_IN: &str = "ACCOUNT_SIGN_IN";
    pub const SWITCH: &str = "ACCOUNT_SWITCH";
    pub const SIGN_OUT: &str = "ACCOUNT_SIGN_OUT";
    pub const DEVICES: &str = "ACCOUNT_DEVICES";
    pub const FORGET_DEVICE: &str = "ACCOUNT_FORGET_DEVICE";
    /// Every id, in menu order.
    pub const ALL: [&str; 6] = [LIST, SIGN_IN, SWITCH, SIGN_OUT, DEVICES, FORGET_DEVICE];
}

macro_rules! metas {
    ($($name:ident = $id:ident, $display:literal, $desc:literal;)*) => {
        $(static $name: ActionMeta = ActionMeta {
            id: ids::$id,
            trait_name: "",
            method_name: "",
            display_name: $display,
            description: $desc,
            category: "Account",
            group: "Account",
            shortcut: "",
            toggleable: false,
            undo: false,
        };)*
    };
}

metas! {
    LIST = LIST, "Accounts", "The accounts signed in on this device (key, name, email, whether it must sign in again) and the active one, at which issuer. No session or token.";
    SIGN_IN = SIGN_IN, "Sign in…", "Opens the sign-in sheet. `method` browser starts a sign-in in the browser at once (the person approves a code on the issuer's page); otherwise the sheet offers the browser and an email and password, which only the person types.";
    SWITCH = SWITCH, "Switch account…", "Makes `account` (its key, id or email, from ACCOUNT_LIST) the active one. With no `account`, opens the accounts sheet to choose.";
    SIGN_OUT = SIGN_OUT, "Sign out", "Signs `account` (the active one when unset) out of this device: its session ends at the issuer and leaves the keychain.";
    DEVICES = DEVICES, "Devices", "The active account's devices at the issuer (id, name, platform, last seen, which one is this device).";
    FORGET_DEVICE = FORGET_DEVICE, "Forget a device", "Takes device `id` (from ACCOUNT_DEVICES) off the active account: a lost phone, an old laptop.";
}

/// One account, as [`ids::LIST`] lists it.
#[derive(Debug, Clone, Facet)]
pub struct AccountRow {
    /// What `ACCOUNT_SWITCH` and `ACCOUNT_SIGN_OUT` take.
    pub key: String,
    pub label: String,
    pub email: Option<String>,
    pub active: bool,
    /// Its session ended: `ACCOUNT_SIGN_IN` brings it back.
    pub needs_sign_in: bool,
}

impl AccountRow {
    fn of(a: &Account, active: bool) -> Self {
        Self {
            key: a.key(),
            label: a.label(),
            email: a.email.clone(),
            active,
            needs_sign_in: a.needs_sign_in,
        }
    }
}

/// What [`ids::LIST`] answers.
#[derive(Debug, Clone, Facet)]
pub struct ListOut {
    pub issuer: String,
    pub active: Option<String>,
    pub accounts: Vec<AccountRow>,
}

/// What [`ids::SIGN_IN`] takes.
#[derive(Debug, Clone, Default, Facet)]
pub struct SignInArgs {
    /// `browser` to start the browser sign-in at once.
    #[facet(default)]
    pub method: Option<String>,
}

/// An account named, or none.
#[derive(Debug, Clone, Default, Facet)]
pub struct AccountArgs {
    /// Its key, id or email.
    #[facet(default)]
    pub account: Option<String>,
}

/// What the sheet does now.
#[derive(Debug, Clone, Facet)]
pub struct SheetOut {
    /// `sign-in`, `accounts`, or `closed`.
    pub sheet: String,
}

/// A device named.
#[derive(Debug, Clone, Facet)]
pub struct DeviceArgs {
    pub id: String,
}

/// What a change answers: the account it was about.
#[derive(Debug, Clone, Facet)]
pub struct Changed {
    pub account: Option<AccountRow>,
    /// What happened, for the person.
    pub said: String,
}

fn sheet_name(sheet: Option<Sheet>) -> String {
    match sheet {
        Some(Sheet::SignIn) => "sign-in",
        Some(Sheet::Accounts) => "accounts",
        None => "closed",
    }
    .to_owned()
}

/// Registers `ACCOUNT_*` on `list`, over `accounts`. Each change is a
/// `Changed { topic: "accounts" }` on the list.
///
/// # Errors
///
/// An id already on the list.
pub fn register(list: &Arc<ActionList>, accounts: &Accounts) -> Result<(), ActionError> {
    let told = Arc::downgrade(list);
    accounts.on_change(move || {
        if let Some(list) = told.upgrade() {
            list.changed("accounts", "");
        }
    });
    let a = accounts.clone();
    list.register(&LIST, Effect::ReadOnly, move |(): ()| {
        let file = a.snapshot();
        Ok(ListOut {
            issuer: a.issuer().origin().to_owned(),
            active: file.active.clone(),
            accounts: file
                .accounts
                .iter()
                .map(|x| AccountRow::of(x, file.active.as_deref() == Some(&x.key())))
                .collect(),
        })
    })?;
    let a = accounts.clone();
    list.register(&SIGN_IN, Effect::Mutating, move |args: SignInArgs| {
        a.open_sheet(Sheet::SignIn);
        if args.method.as_deref() == Some("browser") {
            a.sign_in_browser();
        }
        Ok(SheetOut {
            sheet: sheet_name(a.sheet()),
        })
    })?;
    let a = accounts.clone();
    list.register(&SWITCH, Effect::Mutating, move |args: AccountArgs| {
        if let Some(who) = args.account.as_deref().filter(|s| !s.trim().is_empty()) {
            let account = a.switch(who)?;
            Ok(Changed {
                said: format!("Switched to {}", account.label()),
                account: Some(AccountRow::of(&account, true)),
            })
        } else {
            a.open_sheet(Sheet::Accounts);
            Ok(Changed {
                account: None,
                said: "Opened the accounts".to_owned(),
            })
        }
    })?;
    let a = accounts.clone();
    list.register_async(&SIGN_OUT, Effect::Destructive, move |args: AccountArgs| {
        let a = a.clone();
        async move {
            let account = a.sign_out(args.account.as_deref()).await?;
            Ok(Changed {
                said: format!("Signed out of {}", account.label()),
                account: Some(AccountRow::of(&account, false)),
            })
        }
    })?;
    let a = accounts.clone();
    list.register_async(&DEVICES, Effect::ReadOnly, move |(): ()| {
        let a = a.clone();
        async move { a.refresh_devices().await.map(|d| DevicesOut { devices: d }) }
    })?;
    let a = accounts.clone();
    list.register_async(
        &FORGET_DEVICE,
        Effect::Destructive,
        move |args: DeviceArgs| {
            let a = a.clone();
            async move {
                a.forget_device(&args.id).await?;
                Ok(Changed {
                    account: None,
                    said: format!("Forgot device {}", args.id),
                })
            }
        },
    )?;
    Ok(())
}

/// What [`ids::DEVICES`] answers.
#[derive(Debug, Clone, Facet)]
pub struct DevicesOut {
    pub devices: Vec<Device>,
}
