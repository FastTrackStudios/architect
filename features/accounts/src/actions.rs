//! `ACCOUNT_*` and `ORG_*` on an Action List (feature `actions`): MCP, the
//! menus and ⌘K reach sign-in, switching, signing out, the devices, and the
//! active account's orgs through them.
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
    pub const ORG_LIST: &str = "ORG_LIST";
    pub const ORG_SWITCH: &str = "ORG_SWITCH";
    pub const ORG_DEVICES: &str = "ORG_DEVICES";
    pub const ORG_ENROL: &str = "ORG_ENROL";
    pub const ORG_REVOKE: &str = "ORG_REVOKE";
    /// Every id, in menu order.
    pub const ALL: [&str; 11] = [
        LIST,
        SIGN_IN,
        SWITCH,
        SIGN_OUT,
        DEVICES,
        FORGET_DEVICE,
        ORG_LIST,
        ORG_SWITCH,
        ORG_DEVICES,
        ORG_ENROL,
        ORG_REVOKE,
    ];
}

macro_rules! metas {
    ($($name:ident = $id:ident, $display:literal, $desc:literal;)*) => {
        metas! { @in "Account"; $($name = $id, $display, $desc;)* }
    };
    (@in $group:literal; $($name:ident = $id:ident, $display:literal, $desc:literal;)*) => {
        $(static $name: ActionMeta = ActionMeta {
            id: ids::$id,
            trait_name: "",
            method_name: "",
            display_name: $display,
            description: $desc,
            category: $group,
            group: $group,
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

metas! { @in "Organization";
    ORG_LIST = ORG_LIST, "Organizations", "The orgs the active account is in (id, name, slug, role, whether it is the person's own) and the one it is in now; fetched fresh from the issuer, else the device's cached list with `stale` saying why. Signed out, only the person's own org on this device.";
    ORG_SWITCH = ORG_SWITCH, "Switch organization…", "Makes `org` (its id, slug or name from ORG_LIST, or `all` for every org) the one the active account is in. Kept per account; the issuer is told it is the session's active org.";
    ORG_DEVICES = ORG_DEVICES, "Org devices", "The devices enrolled in the current org (endpoint, name, platform, role, who enrolled it): all of them for an owner or admin, the person's own otherwise. Asked of the node that hosts the org: `host` (its HTTP origin, kept for the org) or the one given before.";
    ORG_ENROL = ORG_ENROL, "Enrol a device…", "Enrols device `endpoint` (its iroh endpoint id) as `name` in the current org with `role` (viewer, member, admin; default member; at most the person's own), at the node that hosts the org (`host`, or the one given before). It can then sync the org's vaults with that node as its role allows. Enrolling again changes its role.";
    ORG_REVOKE = ORG_REVOKE, "Revoke a device", "Withdraws device `endpoint`'s enrolment in the current org (the person who enrolled it, or an owner or admin): its next call to the hosting node is refused and its connections close.";
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
        Some(Sheet::OrgDevices) => "org-devices",
        None => "closed",
    }
    .to_owned()
}

/// Registers `ACCOUNT_*` and `ORG_*` on `list`, over `accounts`. Each
/// change is a `Changed { topic: "accounts" }` on the list.
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
    register_orgs(list, accounts)
}

/// What [`ids::DEVICES`] answers.
#[derive(Debug, Clone, Facet)]
pub struct DevicesOut {
    pub devices: Vec<Device>,
}

/// One org, as [`ids::ORG_LIST`] lists it.
#[derive(Debug, Clone, Facet)]
pub struct OrgRow {
    /// What `ORG_SWITCH` takes (so do `slug` and `name`).
    pub id: String,
    pub name: String,
    pub slug: String,
    /// The person's role there (`Owner`, `Admin`, `Member`, `Viewer`).
    pub role: String,
    /// The person's own.
    pub personal: bool,
    /// The one the account is in now.
    pub current: bool,
}

/// What [`ids::ORG_LIST`] and [`ids::ORG_SWITCH`] answer.
#[derive(Debug, Clone, Facet)]
pub struct OrgsOut {
    /// The active account's key; `None` signed out.
    pub account: Option<String>,
    /// The current org's id; `None` in All.
    pub current: Option<String>,
    pub orgs: Vec<OrgRow>,
    /// Why this is the cached list: the issuer's answer, when it failed.
    pub stale: Option<String>,
}

impl OrgsOut {
    fn of(accounts: &Accounts, stale: Option<String>) -> Self {
        let orgs = accounts.orgs();
        let current = orgs.current().map(|o| o.id.0.clone());
        Self {
            account: accounts.active().map(|a| a.key()),
            orgs: orgs
                .orgs
                .iter()
                .map(|o| OrgRow {
                    id: o.id.0.clone(),
                    name: o.name.clone(),
                    slug: o.slug.clone(),
                    role: o.role.label(),
                    personal: o.personal,
                    current: current.as_ref() == Some(&o.id.0),
                })
                .collect(),
            current,
            stale,
        }
    }
}

/// One enrolled device, as [`ids::ORG_DEVICES`] lists it.
#[derive(Debug, Clone, Facet)]
pub struct OrgDeviceRow {
    /// Its iroh endpoint id: what `ORG_REVOKE` takes.
    pub endpoint: String,
    pub name: String,
    pub platform: String,
    /// Its role in the org.
    pub role: String,
    /// Who enrolled it.
    pub by: String,
}

impl OrgDeviceRow {
    fn of(peer: &architect_devices::Peer) -> Self {
        let grant = peer.grants.iter().find_map(|g| match g {
            architect_devices::Grant::Org { role, by, .. } => Some((role.label(), by.clone())),
            _ => None,
        });
        let (role, by) = grant.unwrap_or_default();
        Self {
            endpoint: peer.endpoint.clone(),
            name: peer.name.clone(),
            platform: peer.platform.clone(),
            role,
            by,
        }
    }
}

/// What [`ids::ORG_DEVICES`] answers.
#[derive(Debug, Clone, Facet)]
pub struct OrgDevicesOut {
    /// The node asked.
    pub host: Option<String>,
    pub devices: Vec<OrgDeviceRow>,
}

/// What [`ids::ORG_DEVICES`] takes.
#[derive(Debug, Clone, Default, Facet)]
pub struct OrgHostArgs {
    /// The hosting node's HTTP origin; kept for the org. Unset: the one
    /// given before.
    #[facet(default)]
    pub host: Option<String>,
}

/// What [`ids::ORG_ENROL`] takes.
#[derive(Debug, Clone, Facet)]
pub struct OrgEnrolArgs {
    #[facet(default)]
    pub host: Option<String>,
    /// The device's iroh endpoint id.
    pub endpoint: String,
    pub name: String,
    #[facet(default)]
    pub platform: Option<String>,
    /// `viewer`, `member` (default) or `admin`.
    #[facet(default)]
    pub role: Option<String>,
}

/// What [`ids::ORG_ENROL`] answers.
#[derive(Debug, Clone, Facet)]
pub struct OrgEnrolOut {
    pub device: OrgDeviceRow,
}

/// What [`ids::ORG_REVOKE`] takes.
#[derive(Debug, Clone, Facet)]
pub struct OrgRevokeArgs {
    #[facet(default)]
    pub host: Option<String>,
    pub endpoint: String,
}

/// What [`ids::ORG_SWITCH`] takes.
#[derive(Debug, Clone, Facet)]
pub struct OrgArgs {
    /// An org's id, slug or name, or `all`.
    pub org: String,
}

fn register_orgs(list: &Arc<ActionList>, accounts: &Accounts) -> Result<(), ActionError> {
    let a = accounts.clone();
    list.register_async(&ORG_LIST, Effect::ReadOnly, move |(): ()| {
        let a = a.clone();
        async move {
            let stale = if a.active().is_some() {
                a.refresh_orgs().await.err()
            } else {
                None
            };
            Ok(OrgsOut::of(&a, stale))
        }
    })?;
    let a = accounts.clone();
    list.register(&ORG_SWITCH, Effect::Mutating, move |args: OrgArgs| {
        a.switch_org(&args.org)?;
        Ok(OrgsOut::of(&a, a.orgs_stale()))
    })?;
    let a = accounts.clone();
    list.register_async(&ORG_DEVICES, Effect::ReadOnly, move |args: OrgHostArgs| {
        let a = a.clone();
        async move {
            let devices = a.org_devices(args.host.as_deref()).await?;
            Ok(OrgDevicesOut {
                host: a.org_host(),
                devices: devices.iter().map(OrgDeviceRow::of).collect(),
            })
        }
    })?;
    let a = accounts.clone();
    list.register_async(&ORG_ENROL, Effect::Mutating, move |args: OrgEnrolArgs| {
        let a = a.clone();
        async move {
            let role = architect_orgs::Role::parse(args.role.as_deref().unwrap_or("member"));
            let peer = a
                .org_enrol(
                    args.host.as_deref(),
                    &args.endpoint,
                    &args.name,
                    args.platform.as_deref().unwrap_or("unknown"),
                    role,
                )
                .await?;
            Ok(OrgEnrolOut {
                device: OrgDeviceRow::of(&peer),
            })
        }
    })?;
    let a = accounts.clone();
    list.register_async(&ORG_REVOKE, Effect::Mutating, move |args: OrgRevokeArgs| {
        let a = a.clone();
        async move {
            a.org_revoke(args.host.as_deref(), &args.endpoint).await?;
            let devices = a
                .org_devices_seen()
                .and_then(Result::ok)
                .unwrap_or_default();
            Ok(OrgDevicesOut {
                host: a.org_host(),
                devices: devices.iter().map(OrgDeviceRow::of).collect(),
            })
        }
    })?;
    Ok(())
}
