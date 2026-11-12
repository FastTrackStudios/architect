//! The accounts in a window (feature `ui`).
//!
//! [`use_accounts`] is the handle, hearing the controller's changes;
//! [`AccountsUi::shell_account`] the shell's Account item (who is signed
//! in, the org switcher and the account switcher in its menu); [`AccountSheet`] signing in, the
//! accounts on this device and the active one's devices. Mount the sheet
//! once, over the app.

use std::sync::Arc;

use architect::action::ActionList;
use architect_shell::{self as shell, ShellTheme, icon};
use dioxus::prelude::*;
use futures_util::StreamExt as _;
use lucide_dioxus::{Globe, LogIn, LogOut, MonitorSmartphone, Share2, UserPlus, Users};

use crate::controller::{Accounts, Sheet, Status};
use crate::model::{Account, AccountsFile};

/// Over the shell, under the confirmation sheet (`architect_commands`).
const LAYER: u8 = 90;

/// The avatars' fills, by the account's key.
const MARKS: [&str; 6] = [
    "#1d4ed8", "#b45309", "#047857", "#7c3aed", "#be123c", "#0e7490",
];

/// The accounts in this window: a `Copy` handle over [`Accounts`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AccountsUi {
    accounts: CopyValue<Accounts>,
    rev: Signal<u64>,
}

/// The window's accounts, made once by `make`, provided as context, its
/// changes heard; with `list`, `ACCOUNT_*` go on it. Checks the active
/// account's session with the issuer at start.
pub fn use_accounts(list: Option<Arc<ActionList>>, make: impl FnOnce() -> Accounts) -> AccountsUi {
    let accounts = use_hook(|| CopyValue::new(make()));
    let mut rev = use_signal(|| 0_u64);
    use_hook(move || {
        let a = accounts.read().clone();
        let (tx, mut rx) = futures_channel::mpsc::unbounded::<()>();
        a.on_change(move || {
            let _ = tx.unbounded_send(());
        });
        if let Some(list) = list {
            let _ = crate::actions::register(&list, &a);
        }
        let heard = a.clone();
        spawn(async move {
            while rx.next().await.is_some() {
                rev.set(heard.rev());
            }
        });
        a.start();
    });
    use_context_provider(|| AccountsUi { accounts, rev })
}

/// The window's accounts, where [`use_accounts`] provided them.
#[must_use]
pub fn try_use_accounts() -> Option<AccountsUi> {
    try_use_context::<AccountsUi>()
}

impl AccountsUi {
    /// The controller.
    #[must_use]
    pub fn accounts(&self) -> Accounts {
        self.accounts.read().clone()
    }

    /// The accounts now; reading it redraws on a change.
    #[must_use]
    pub fn snapshot(&self) -> AccountsFile {
        let _ = (self.rev)();
        self.accounts.read().snapshot()
    }

    /// The active account's orgs and the current one; reading it redraws
    /// on a change.
    #[must_use]
    pub fn orgs(&self) -> architect_orgs::Orgs {
        let _ = (self.rev)();
        self.accounts.read().orgs()
    }

    /// Opens the sheet on `page`.
    pub fn open(&self, page: Sheet) {
        self.accounts.read().open_sheet(page);
    }

    /// The shell's Account item: the active account, or "Not signed in",
    /// its menu `base` (the app's own rows) after the switchers': the
    /// account's orgs (the current one ticked, All where it has several),
    /// the other accounts, add one, the accounts and devices, sign out.
    #[must_use]
    pub fn shell_account(&self, base: shell::Menu) -> shell::Account {
        let file = self.snapshot();
        let me = *self;
        let Some(active) = file.active().cloned() else {
            let mut menu = shell::Menu::new().item("Sign in…", icon!(LogIn), move |()| {
                me.accounts().sign_in()
            });
            if !file.accounts.is_empty() {
                menu = menu.item("Accounts…", icon!(Users), move |()| {
                    me.open(Sheet::Accounts);
                });
            }
            return shell::Account::new("Not signed in")
                .detail("Sign in to sync")
                .color("#52525b")
                .menu(append(menu.divider(), base));
        };
        let mut menu = self.org_rows(shell::Menu::new());
        let others: Vec<Account> = file
            .accounts
            .iter()
            .filter(|a| a.key() != active.key())
            .cloned()
            .collect();
        if !others.is_empty() {
            menu = menu.heading("Switch to");
            for other in others {
                let key = other.key();
                let label = other.email.clone().unwrap_or_else(|| other.label());
                menu = menu.item(label, None, move |()| {
                    let _ = me.accounts().switch(&key);
                });
                if other.needs_sign_in {
                    menu = menu.note("Sign in again");
                }
            }
            menu = menu.divider();
        }
        menu = menu
            .item("Add an account…", icon!(UserPlus), move |()| {
                me.accounts().sign_in();
            })
            .item(
                "Accounts and devices…",
                icon!(MonitorSmartphone),
                move |()| {
                    me.open(Sheet::Accounts);
                },
            )
            .divider();
        let menu = append(menu, base)
            .divider()
            .item("Sign out", icon!(LogOut), move |()| {
                let a = me.accounts();
                spawn(async move {
                    let _ = a.sign_out(None).await;
                });
            })
            .danger();
        let mut item = shell::Account::new(active.label())
            .color(mark(&active.key()))
            .menu(menu);
        if let Some(email) = active.email.clone().filter(|e| *e != active.label()) {
            item = item.detail(email);
        }
        if active.needs_sign_in {
            item = item.detail("Signed out: sign in again");
        }
        if let Some(image) = active.image {
            item = item.image(image);
        }
        item
    }
}

impl AccountsUi {
    /// The org switcher's rows: a heading, each org (its role noted), and
    /// All when there are several.
    fn org_rows(&self, mut menu: shell::Menu) -> shell::Menu {
        let orgs = self.orgs();
        let me = *self;
        menu = menu.heading("Organization");
        let current = orgs.current().map(|o| o.id.clone());
        for org in &orgs.orgs {
            let id = org.id.0.clone();
            menu = menu
                .check(
                    org.name.clone(),
                    current.as_ref() == Some(&org.id),
                    move |()| {
                        let _ = me.accounts().switch_org(&id);
                    },
                )
                .note(if org.personal {
                    "Yours".to_owned()
                } else {
                    org.role.label()
                });
        }
        if orgs.orgs.len() > 1 {
            menu = menu.check("All organizations", current.is_none(), move |()| {
                let _ = me.accounts().switch_org("all");
            });
        }
        if let Some(org) = orgs.current().filter(|o| o.id.0 != crate::DEVICE_ORG) {
            menu = menu.item(
                format!("Devices in {}…", org.name),
                icon!(MonitorSmartphone),
                move |()| me.open(Sheet::OrgDevices),
            );
            let share = self.clone();
            menu = menu.item("Share links…", icon!(Share2), move |()| {
                share.accounts().open_share_sheet(crate::ShareDraft {
                    vault: String::new(),
                    scope: architect_spaces::ShareScope::Vault,
                });
            });
        }
        menu.divider()
    }
}

fn append(mut menu: shell::Menu, base: shell::Menu) -> shell::Menu {
    menu.entries.extend(base.entries);
    menu
}

fn mark(key: &str) -> &'static str {
    let at = key
        .bytes()
        .fold(0_usize, |a, c| {
            a.wrapping_mul(31).wrapping_add(usize::from(c))
        })
        .checked_rem(MARKS.len())
        .unwrap_or(0);
    MARKS.get(at).copied().unwrap_or("#3f3f46")
}

fn initials(label: &str) -> String {
    label
        .split(|c: char| c.is_whitespace() || c == '@' || c == '.')
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

fn theme() -> ShellTheme {
    shell::try_use_shell().map(|s| s.theme).unwrap_or_default()
}

/// The sheet: signing in, or the accounts and devices. Mounted once over
/// the app (absolute against its root); draws nothing while closed.
#[component]
pub fn AccountSheet() -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let _ = (ui.rev)();
    let accounts = ui.accounts();
    let Some(page) = accounts.sheet() else {
        return rsx! {};
    };
    let t = theme();
    let close = accounts.clone();
    let esc = accounts;
    rsx! {
        div {
            "data-account-sheet": match page { Sheet::SignIn => "sign-in", Sheet::Accounts => "accounts", Sheet::OrgDevices => "org-devices", Sheet::Share => "share" },
            style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {LAYER}; background: {t.scrim}; \
                    display: flex; justify-content: center; align-items: center; font-family: {t.font};",
            onclick: move |_| close.close_sheet(),
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Named(NamedKey::Escape) {
                    esc.close_sheet();
                }
            },
            div {
                role: "dialog",
                "aria-label": match page { Sheet::SignIn => "Sign in", Sheet::Accounts => "Accounts and devices", Sheet::OrgDevices => "Org devices", Sheet::Share => "Share" },
                style: "width: 420px; max-width: 92%; max-height: 88%; overflow-y: auto; display: flex; flex-direction: column; \
                        gap: 16px; padding: 22px; border-radius: 14px; background: {t.pill}; border: 1px solid {t.rule}; \
                        box-shadow: {t.shadow_float}; color: {t.ink};",
                onclick: move |e| e.stop_propagation(),
                match page {
                    Sheet::SignIn => rsx! { SignInPage {} },
                    Sheet::Accounts => rsx! { AccountsPage {} },
                    Sheet::OrgDevices => rsx! { OrgDevicesPage {} },
                    Sheet::Share => rsx! { SharePage {} },
                }
            }
        }
    }
}

fn button_style(t: &ShellTheme, primary: bool) -> String {
    if primary {
        format!(
            "height: 38px; padding: 0 14px; border-radius: 9px; border: none; background: {}; color: {}; \
             font-family: {}; font-size: 14px; font-weight: 600; display: flex; align-items: center; \
             justify-content: center; gap: 8px;",
            t.accent, t.accent_ink, t.font
        )
    } else {
        format!(
            "height: 34px; padding: 0 12px; border-radius: 8px; border: 1px solid {}; background: transparent; \
             color: {}; font-family: {}; font-size: 13px; display: flex; align-items: center; justify-content: center; gap: 6px;",
            t.rule, t.ink, t.font
        )
    }
}

fn field_style(t: &ShellTheme) -> String {
    format!(
        "height: 38px; box-sizing: border-box; padding: 0 12px; border-radius: 9px; border: 1px solid {}; \
         background: {}; color: {}; font-family: {}; font-size: 14px;",
        t.rule, t.desk, t.ink, t.font
    )
}

fn host(origin: &str) -> String {
    origin
        .split_once("://")
        .map_or(origin, |(_, h)| h)
        .trim_end_matches('/')
        .to_owned()
}

#[component]
fn StatusLine(status: Status) -> Element {
    let t = theme();
    match status {
        Status::Idle => rsx! {},
        Status::Busy(s) => {
            rsx! { div { role: "status", style: "font-size: 13px; color: {t.ink_2};", "{s}" } }
        }
        Status::Done(s) => {
            rsx! { div { role: "status", style: "font-size: 13px; color: {t.ink_2};", "{s}" } }
        }
        Status::Failed(s) => {
            rsx! { div { role: "alert", style: "font-size: 13px; color: {t.danger};", "{s}" } }
        }
    }
}

/// The sign-in sheet: the browser sign-in under way (its code, the page
/// again, Cancel), or the button that starts it. No field here takes a
/// password: the person signs in on the issuer's own page
/// (`crate::browser`), which a password manager or a passkey fills.
#[component]
fn SignInPage() -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let _ = (ui.rev)();
    let accounts = ui.accounts();
    let t = theme();
    let status = accounts.status();
    let busy = matches!(status, Status::Busy(_));
    let wait = accounts.browser();
    let issuer = host(accounts.issuer().origin());
    let has_accounts = !accounts.snapshot().accounts.is_empty();
    let (browser, cancel, again, back, later) = (
        accounts.clone(),
        accounts.clone(),
        accounts.clone(),
        accounts.clone(),
        accounts.clone(),
    );
    let (again2, cancel2) = (accounts.clone(), accounts);
    let reopen = wait.as_ref().map(|w| w.url.clone());
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 4px;",
            div { style: "font-size: 18px; font-weight: 650;", "Sign in" }
            div { style: "font-size: 13px; color: {t.ink_2}; line-height: 1.4;",
                "Optional: the app works signed out. You sign in on {issuer}'s own page, in your browser, and come back signed in."
            }
        }
        if let Some(w) = wait {
            if w.user_code.is_empty() {
                // The browser sign-in: finished in the browser.
                div { "data-browser-wait": "true",
                    style: "display: flex; flex-direction: column; gap: 10px; padding: 14px; border-radius: 10px; background: {t.active};",
                    div { style: "font-size: 14px; color: {t.ink};",
                        if w.opened { "Finish signing in on the page that opened. You come back here signed in." } else { "Your browser did not open. Start again, or open this page:" }
                    }
                    if !w.opened {
                        div { style: "font-size: 12px; color: {t.ink_3}; word-break: break-all;", "{w.url}" }
                    }
                    div { style: "display: flex; gap: 8px;",
                        button { style: button_style(&t, false),
                            onclick: move |_| {
                                again.cancel_browser();
                                again.sign_in_browser();
                            },
                            Globe { size: 14 }
                            "Start again"
                        }
                        button { style: button_style(&t, false), onclick: move |_| cancel.cancel_browser(), "Cancel" }
                    }
                }
            } else {
                // A code to approve (the device grant).
                div { "data-browser-code": "{w.user_code}",
                    style: "display: flex; flex-direction: column; gap: 10px; padding: 14px; border-radius: 10px; background: {t.active};",
                    div { style: "font-size: 13px; color: {t.ink_2};",
                        if w.opened { "Sign in and approve this code on the page that opened:" } else { "Open the page below and approve this code:" }
                    }
                    div { style: "font-size: 26px; font-weight: 700; letter-spacing: 3px; font-family: ui-monospace, Menlo, monospace;", "{w.user_code}" }
                    div { style: "font-size: 12px; color: {t.ink_3}; word-break: break-all;", "{w.url}" }
                    div { style: "display: flex; gap: 8px;",
                        button { style: button_style(&t, false),
                            onclick: move |_| {
                                if let Some(url) = reopen.clone() {
                                    let a = again2.clone();
                                    let _ = crate::browser::open(&url, move |back| {
                                        if back.is_none() {
                                            a.cancel_browser();
                                        }
                                    });
                                }
                            },
                            Globe { size: 14 }
                            "Open the page"
                        }
                        button { style: button_style(&t, false), onclick: move |_| cancel2.cancel_browser(), "Cancel" }
                    }
                }
            }
        } else {
            button {
                "data-sign-in": "browser",
                style: button_style(&t, true),
                disabled: busy,
                onclick: move |_| browser.sign_in_browser(),
                Globe { size: 16 }
                "Sign in with your browser"
            }
        }
        StatusLine { status }
        div { style: "display: flex; justify-content: space-between; gap: 8px;",
            if has_accounts {
                button { style: button_style(&t, false), onclick: move |_| back.open_sheet(Sheet::Accounts), "Accounts" }
            } else {
                span {}
            }
            button { style: button_style(&t, false), onclick: move |_| later.close_sheet(), "Not now" }
        }
    }
}

#[component]
fn AccountsPage() -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let file = ui.snapshot();
    let accounts = ui.accounts();
    let t = theme();
    let active = file.active.clone();
    let devices = accounts.devices();
    let status = accounts.status();
    let (add, done) = (accounts.clone(), accounts);
    rsx! {
        div { style: "font-size: 18px; font-weight: 650;", "Accounts" }
        if file.accounts.is_empty() {
            div { style: "font-size: 13px; color: {t.ink_2};", "No account is signed in on this device." }
        }
        for account in file.accounts {
            AccountRowView { key: "{account.key()}", active: active.as_deref() == Some(account.key().as_str()), account }
        }
        button { style: button_style(&t, false), onclick: move |_| add.sign_in(),
            UserPlus { size: 14 }
            "Add an account"
        }
        StatusLine { status }
        if active.is_some() {
            div { style: "display: flex; flex-direction: column; gap: 8px; padding-top: 6px; border-top: 1px solid {t.rule};",
                div { style: "font-size: 14px; font-weight: 600; padding-top: 8px;", "This account's devices" }
                match devices {
                    None => rsx! { div { style: "font-size: 13px; color: {t.ink_3};", "Fetching the devices…" } },
                    Some(Err(why)) => rsx! { div { role: "alert", style: "font-size: 13px; color: {t.danger};", "{why}" } },
                    Some(Ok(list)) if list.is_empty() => rsx! { div { style: "font-size: 13px; color: {t.ink_3};", "No devices yet." } },
                    Some(Ok(list)) => rsx! {
                        for d in list {
                            DeviceRowView { key: "{d.id}", device: d }
                        }
                    },
                }
            }
        }
        div { style: "display: flex; justify-content: flex-end;",
            button { style: button_style(&t, false), onclick: move |_| done.close_sheet(), "Done" }
        }
    }
}

#[component]
fn AccountRowView(account: Account, active: bool) -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let t = theme();
    let key = account.key();
    let label = account.label();
    let detail = account.email.clone().filter(|e| *e != label);
    let fill = mark(&key);
    let letters = initials(&label);
    let (switch_key, out_key) = (key.clone(), key);
    let needs = account.needs_sign_in;
    let row_fill = if active {
        t.active.to_string()
    } else {
        "transparent".to_owned()
    };
    rsx! {
        div { "data-account": "{label}",
            style: "display: flex; align-items: center; gap: 10px; padding: 8px; border-radius: 10px; background: {row_fill};",
            div { style: "flex-shrink: 0; width: 32px; height: 32px; border-radius: 16px; background: {fill}; color: white; \
                          display: flex; align-items: center; justify-content: center; font-size: 12px; font-weight: 600;",
                "{letters}"
            }
            div { style: "flex: 1 1 0; min-width: 0; display: flex; flex-direction: column;",
                div { style: "font-size: 14px; font-weight: 550; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{label}" }
                if let Some(d) = detail {
                    div { style: "font-size: 12px; color: {t.ink_2};", "{d}" }
                }
                if needs {
                    div { style: "font-size: 12px; color: {t.danger};", "Signed out: sign in again" }
                } else if active {
                    div { style: "font-size: 12px; color: {t.ink_3};", "Active" }
                }
            }
            if needs {
                button { style: button_style(&t, false), onclick: move |_| ui.accounts().sign_in(), "Sign in" }
            } else if !active {
                button { style: button_style(&t, false), onclick: move |_| { let _ = ui.accounts().switch(&switch_key); }, "Switch" }
            }
            button { style: button_style(&t, false), "aria-label": "Sign out of {label}",
                onclick: move |_| {
                    let (a, k) = (ui.accounts(), out_key.clone());
                    spawn(async move { let _ = a.sign_out(Some(&k)).await; });
                },
                "Sign out"
            }
        }
    }
}

#[component]
fn DeviceRowView(device: crate::model::Device) -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let t = theme();
    let seen = device
        .last_seen
        .get(..10)
        .unwrap_or(&device.last_seen)
        .to_owned();
    let id = device.id.clone();
    rsx! {
        div { "data-device": "{device.name}",
            style: "display: flex; align-items: center; gap: 10px; padding: 6px 8px;",
            MonitorSmartphone { size: 16, color: t.ink_2.to_string() }
            div { style: "flex: 1 1 0; min-width: 0; display: flex; flex-direction: column;",
                div { style: "font-size: 13px;", "{device.name}" }
                div { style: "font-size: 12px; color: {t.ink_3};",
                    if device.this_one { "{device.platform} · this device" } else { "{device.platform} · last seen {seen}" }
                }
            }
            if !device.this_one {
                button { style: button_style(&t, false), "aria-label": "Forget {device.name}",
                    onclick: move |_| {
                        let (a, id) = (ui.accounts(), id.clone());
                        spawn(async move { let _ = a.forget_device(&id).await; });
                    },
                    "Forget"
                }
            }
        }
    }
}

/// The current org's enrolled devices, at the node hosting the org:
/// each with Revoke, and a form to enrol one (`ORG_DEVICES`,
/// `ORG_ENROL`, `ORG_REVOKE` do the same from the Action List).
#[component]
fn OrgDevicesPage() -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let _ = (ui.rev)();
    let accounts = ui.accounts();
    let t = theme();
    let field = field_style(&t);
    let org = accounts
        .orgs()
        .current()
        .map_or_else(|| "this org".to_owned(), |o| o.name.clone());
    let known = accounts.org_host();
    let mut host = use_signal(|| known.clone().unwrap_or_default());
    let mut endpoint = use_signal(String::new);
    let mut name = use_signal(String::new);
    let mut role = use_signal(|| "member".to_owned());
    let mut problem = use_signal(|| None::<String>);
    let seen = accounts.org_devices_seen();
    let (ask, enrol, done) = (accounts.clone(), accounts.clone(), accounts);
    rsx! {
        div { style: "font-size: 18px; font-weight: 650;", "Devices in {org}" }
        div { style: "font-size: 13px; color: {t.ink_2};",
            "Enrolled at the node that hosts the org. An enrolled device syncs the org's vaults as its role allows; revoking it refuses its next call."
        }
        div { style: "display: flex; gap: 8px; align-items: center;",
            input { id: "org-host", placeholder: "Host node (https://…)", value: "{host}", style: "{field} flex: 1 1 0;",
                oninput: move |e: FormEvent| host.set(e.value()) }
            button { style: button_style(&t, false),
                onclick: move |_| {
                    let (a, h) = (ask.clone(), host.peek().clone());
                    spawn(async move { let _ = a.org_devices(Some(&h)).await; });
                },
                "Refresh"
            }
        }
        match seen {
            None if known.is_none() => rsx! { div { style: "font-size: 13px; color: {t.ink_3};", "Give the address of the node that hosts the org." } },
            None => rsx! { div { style: "font-size: 13px; color: {t.ink_3};", "Fetching the devices…" } },
            Some(Err(why)) => rsx! { div { role: "alert", style: "font-size: 13px; color: {t.danger};", "{why}" } },
            Some(Ok(list)) if list.is_empty() => rsx! { div { style: "font-size: 13px; color: {t.ink_3};", "No device is enrolled yet." } },
            Some(Ok(list)) => rsx! {
                for peer in list {
                    OrgDeviceRowView { key: "{peer.endpoint}", peer }
                }
            },
        }
        div { style: "display: flex; flex-direction: column; gap: 8px; padding-top: 8px; border-top: 1px solid {t.rule};",
            div { style: "font-size: 14px; font-weight: 600; padding-top: 8px;", "Enrol a device" }
            input { id: "org-enrol-endpoint", placeholder: "Its endpoint id", value: "{endpoint}", style: "{field}",
                oninput: move |e: FormEvent| endpoint.set(e.value()) }
            input { id: "org-enrol-name", placeholder: "Its name", value: "{name}", style: "{field}",
                oninput: move |e: FormEvent| name.set(e.value()) }
            select { id: "org-enrol-role", style: "{field}", value: "{role}",
                onchange: move |e: FormEvent| role.set(e.value()),
                option { value: "viewer", "Viewer" }
                option { value: "member", "Member" }
                option { value: "admin", "Admin" }
            }
            if let Some(why) = problem() {
                div { role: "alert", style: "font-size: 13px; color: {t.danger};", "{why}" }
            }
            button { style: button_style(&t, true),
                onclick: move |_| {
                    let a = enrol.clone();
                    let (h, e, n, r) = (host.peek().clone(), endpoint.peek().clone(), name.peek().clone(), role.peek().clone());
                    spawn(async move {
                        let host = Some(h.as_str()).filter(|h| !h.trim().is_empty());
                        match a.org_enrol(host, &e, &n, "unknown", architect_orgs::Role::parse(&r)).await {
                            Ok(_) => { problem.set(None); endpoint.set(String::new()); name.set(String::new()); }
                            Err(why) => problem.set(Some(why)),
                        }
                    });
                },
                "Enrol"
            }
        }
        div { style: "display: flex; justify-content: flex-end;",
            button { style: button_style(&t, false), onclick: move |_| done.close_sheet(), "Done" }
        }
    }
}

#[component]
fn OrgDeviceRowView(peer: architect_devices::Peer) -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let t = theme();
    let role = peer
        .orgs()
        .first()
        .map_or_else(String::new, |(_, role)| role.label());
    let endpoint = peer.endpoint.clone();
    let short: String = peer.endpoint.chars().take(12).collect();
    rsx! {
        div { "data-org-device": "{peer.name}",
            style: "display: flex; align-items: center; gap: 10px; padding: 6px 8px;",
            MonitorSmartphone { size: 16, color: t.ink_2.to_string() }
            div { style: "flex: 1 1 0; min-width: 0; display: flex; flex-direction: column;",
                div { style: "font-size: 13px;", "{peer.name}" }
                div { style: "font-size: 12px; color: {t.ink_3};", "{role} · {peer.platform} · {short}…" }
            }
            button { style: button_style(&t, false), "aria-label": "Revoke {peer.name}",
                onclick: move |_| {
                    let (a, e) = (ui.accounts(), endpoint.clone());
                    spawn(async move { let _ = a.org_revoke(None, &e).await; });
                },
                "Revoke"
            }
        }
    }
}

/// The Share sheet (#374): share a note, folder, tag or vault from the
/// node that hosts the org, with an audience, an access and an expiry;
/// the org's shares listed, each with its link and Revoke (`SHARE_*` do
/// the same from the Action List).
#[component]
fn SharePage() -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let _ = (ui.rev)();
    let accounts = ui.accounts();
    let t = theme();
    let field = field_style(&t);
    let (draft, made) = accounts.share_draft();
    let draft = draft.unwrap_or(crate::ShareDraft {
        vault: String::new(),
        scope: architect_spaces::ShareScope::Vault,
    });
    let what = draft.scope.describe();
    let known = accounts.org_host();
    let mut host = use_signal(|| known.clone().unwrap_or_default());
    let mut audience = use_signal(|| "link".to_owned());
    let mut emails = use_signal(String::new);
    let mut access = use_signal(|| "read".to_owned());
    let mut expires = use_signal(|| "never".to_owned());
    let mut listen = use_signal(|| architect_spaces::Listen::default().label().to_owned());
    let mut problem = use_signal(|| None::<String>);
    let seen = accounts.shares_seen();
    let made_url = made.as_ref().and_then(|s| accounts.share_url(s));
    let (ask, make, done) = (accounts.clone(), accounts.clone(), accounts);
    let draft_make = draft;
    rsx! {
        div { style: "font-size: 18px; font-weight: 650;", "Share {what}" }
        div { style: "font-size: 13px; color: {t.ink_2};",
            "The node that hosts the org serves these notes, and only these, on a link of their own. Links to other notes show as plain text."
        }
        if known.is_none() {
            input { id: "share-host", placeholder: "Host node (https://…)", value: "{host}", style: "{field}",
                oninput: move |e: FormEvent| host.set(e.value()) }
        }
        label { style: "display: flex; flex-direction: column; gap: 4px; font-size: 13px;", "Who can open it"
            Chips { id: "share-audience", options: vec![("link", "Anyone with the link"), ("public", "Public"), ("org", "Org members"), ("invited", "Invited people")].into_iter().map(|(v, l): (&str, &str)| (v.to_owned(), l.to_owned())).collect(), now: audience(), onpick: move |v: String| audience.set(v) }
        }
        if audience() == "invited" {
            input { id: "share-emails", placeholder: "Their emails, separated by commas", value: "{emails}", style: "{field}",
                oninput: move |e: FormEvent| emails.set(e.value()) }
        }
        div { style: "display: flex; gap: 8px;",
            label { style: "flex: 1 1 0; display: flex; flex-direction: column; gap: 4px; font-size: 13px;", "They can"
                Chips { id: "share-access", options: vec![("read", "Read"), ("comment", "Read and comment")].into_iter().map(|(v, l): (&str, &str)| (v.to_owned(), l.to_owned())).collect(), now: access(), onpick: move |v: String| access.set(v) }
            }
            label { style: "flex: 1 1 0; display: flex; flex-direction: column; gap: 4px; font-size: 13px;", "Open until"
                Chips { id: "share-expires", options: vec![("never", "Until revoked"), ("24h", "A day"), ("7d", "A week"), ("30d", "30 days")].into_iter().map(|(v, l): (&str, &str)| (v.to_owned(), l.to_owned())).collect(), now: expires(), onpick: move |v: String| expires.set(v) }
            }
        }
        label { style: "display: flex; flex-direction: column; gap: 4px; font-size: 13px;", "It plays"
            Chips { id: "share-listen", options: listen_options(), now: listen(), onpick: move |v: String| listen.set(v) }
        }
        if let Some(why) = problem() {
            div { role: "alert", style: "font-size: 13px; color: {t.danger};", "{why}" }
        }
        button { id: "share-create", style: button_style(&t, true),
            onclick: move |_| {
                let a = make.clone();
                let draft = draft_make.clone();
                let (h, who, list, acc, exp, plays) = (host.peek().clone(), audience.peek().clone(), emails.peek().clone(), access.peek().clone(), expires.peek().clone(), listen.peek().clone());
                spawn(async move {
                    let who = if who == "invited" { format!("invited:{list}") } else { who };
                    let made = async {
                        let audience = architect_spaces::Audience::parse(&who)?;
                        let access = crate::share_actions::parse_access(&acc)?;
                        let expires_at = crate::share_actions::parse_expiry(&exp, now_secs())?;
                        let listen = architect_spaces::Listen::parse(&plays)?;
                        let host = Some(h.as_str()).filter(|h| !h.trim().is_empty());
                        a.share_create(host, architect_spaces::NewShare {
                            vault: draft.vault.clone(),
                            scope: draft.scope.clone(),
                            title: None,
                            audience,
                            access,
                            expires_at,
                            domain: None,
                            listen,
                        }).await
                    }.await;
                    match made {
                        Ok(_) => problem.set(None),
                        Err(why) => problem.set(Some(why)),
                    }
                });
            },
            "Create link"
        }
        if let Some(url) = made_url {
            div { "data-share-made": "{url}", style: "display: flex; flex-direction: column; gap: 4px;",
                div { style: "font-size: 13px; color: {t.ink_2};", "The link: select it and copy." }
                input { id: "share-url", readonly: true, value: "{url}", style: "{field}" }
            }
        }
        div { style: "display: flex; flex-direction: column; gap: 6px; padding-top: 8px; border-top: 1px solid {t.rule};",
            div { style: "display: flex; align-items: center; gap: 8px; padding-top: 8px;",
                div { style: "flex: 1 1 0; font-size: 14px; font-weight: 600;", "Share links" }
                button { style: button_style(&t, false),
                    onclick: move |_| {
                        let (a, h) = (ask.clone(), host.peek().clone());
                        spawn(async move { let _ = a.shares(Some(h.as_str()).filter(|h| !h.trim().is_empty())).await; });
                    },
                    "Refresh"
                }
            }
            match seen {
                None if known.is_none() => rsx! { div { style: "font-size: 13px; color: {t.ink_3};", "Give the address of the node that hosts the org." } },
                None => rsx! { div { style: "font-size: 13px; color: {t.ink_3};", "Fetching the shares…" } },
                Some(Err(why)) => rsx! { div { role: "alert", style: "font-size: 13px; color: {t.danger};", "{why}" } },
                Some(Ok(list)) if list.is_empty() => rsx! { div { style: "font-size: 13px; color: {t.ink_3};", "Nothing is shared yet." } },
                Some(Ok(list)) => rsx! {
                    for share in list.into_iter().rev() {
                        ShareRowView { key: "{share.code}", share }
                    }
                },
            }
        }
        div { style: "display: flex; justify-content: flex-end;",
            button { style: button_style(&t, false), onclick: move |_| done.close_sheet(), "Done" }
        }
    }
}

/// The layers a link plays to (#528), as [`Chips`] offers them.
fn listen_options() -> Vec<(String, String)> {
    [
        ("page", "Page"),
        ("reference", "Reference"),
        ("full", "Full multitracks"),
    ]
    .into_iter()
    .map(|(v, l)| (v.to_owned(), l.to_owned()))
    .collect()
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[component]
fn ShareRowView(share: architect_spaces::Share) -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let t = theme();
    let field = field_style(&t);
    let accounts = ui.accounts();
    let url = accounts.share_url(&share).unwrap_or_default();
    let state = match share.state(now_secs()) {
        architect_spaces::ShareState::Live => "live",
        architect_spaces::ShareState::Expired => "expired",
        architect_spaces::ShareState::Revoked => "revoked",
    };
    let live = state == "live";
    let dim = if live { "1" } else { "0.55" };
    let title = share
        .title
        .clone()
        .unwrap_or_else(|| share.scope.describe());
    let audience = share.audience.label();
    let access = crate::share_actions::access_label(share.access);
    let plays = share.listen.label();
    let code = share.code.clone();
    let plays_code = share.code.clone();
    // Why changing how far it plays failed, shown under it.
    let mut plays_problem = use_signal(|| None::<String>);
    rsx! {
        div { "data-share": "{share.code}", "data-share-state": "{state}",
            style: "display: flex; flex-direction: column; gap: 4px; padding: 6px 8px; opacity: {dim};",
            div { style: "display: flex; align-items: center; gap: 10px;",
                Share2 { size: 16, color: t.ink_2.to_string() }
                div { style: "flex: 1 1 0; min-width: 0; display: flex; flex-direction: column;",
                    div { style: "font-size: 13px;", "{title}" }
                    div { style: "font-size: 12px; color: {t.ink_3};", "{audience} · {access} · plays {plays} · {state}" }
                }
                if live {
                    button { style: button_style(&t, false), "aria-label": "Revoke {title}",
                        onclick: move |_| {
                            let (a, c) = (ui.accounts(), code.clone());
                            spawn(async move { let _ = a.share_revoke(None, &c).await; });
                        },
                        "Revoke"
                    }
                }
            }
            if live {
                input { readonly: true, value: "{url}", style: "{field} font-size: 12px;" }
                Chips { id: "share-listen-{share.code}", options: listen_options(), now: share.listen.label().to_owned(),
                    onpick: move |v: String| {
                        let (a, c) = (ui.accounts(), plays_code.clone());
                        spawn(async move {
                            let changed = match architect_spaces::Listen::parse(&v) {
                                Ok(l) => a.share_update(None, &c, architect_spaces::ShareChange::Listen(l)).await.map(|_| ()),
                                Err(why) => Err(why),
                            };
                            plays_problem.set(changed.err());
                        });
                    }
                }
                if let Some(why) = plays_problem() {
                    div { role: "alert", style: "font-size: 12px; color: {t.danger};", "{why}" }
                }
                for audience in share.audiences.iter() {
                    AudiencePlays { key: "{audience.code}", code: share.code.clone(), name: audience.name.clone(), audience_code: audience.code.clone(), now: share.listen_for(Some(audience)).label().to_owned() }
                }
            }
        }
    }
}

/// How far one audience's link of share `code` plays (#528), as a
/// choice of the three layers.
#[component]
fn AudiencePlays(code: String, name: String, audience_code: String, now: String) -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let t = theme();
    let label = name.clone();
    let mut problem = use_signal(|| None::<String>);
    rsx! {
        div { "data-share-audience": "{label}",
            style: "display: flex; align-items: center; gap: 8px; font-size: 12px;",
            div { style: "flex: 0 0 96px; overflow: hidden; text-overflow: ellipsis; color: {t.ink_2};", "{label}" }
            Chips { id: "share-listen-{audience_code}", options: listen_options(), now,
                onpick: move |v: String| {
                    let (a, c, n) = (ui.accounts(), code.clone(), name.clone());
                    spawn(async move {
                        let changed = match architect_spaces::Listen::parse(&v) {
                            Ok(l) => a.share_audience_listen(None, &c, &n, Some(l)).await.map(|_| ()),
                            Err(why) => Err(why),
                        };
                        problem.set(changed.err());
                    });
                }
            }
        }
        if let Some(why) = problem() {
            div { role: "alert", style: "font-size: 12px; color: {t.danger};", "{why}" }
        }
    }
}

/// One choice of a few, as a row of buttons (Blitz draws no `select`).
#[component]
fn Chips(
    id: String,
    options: Vec<(String, String)>,
    now: String,
    onpick: EventHandler<String>,
) -> Element {
    let t = theme();
    rsx! {
        div { id: "{id}", role: "radiogroup", style: "display: flex; flex-wrap: wrap; gap: 6px;",
            for (value, label) in options {
                button {
                    key: "{value}",
                    role: "radio",
                    "aria-checked": if value == now { "true" } else { "false" },
                    style: format!("{} height: 32px; font-size: 13px;", button_style(&t, value == now)),
                    onclick: move |_| onpick.call(value.clone()),
                    "{label}"
                }
            }
        }
    }
}
