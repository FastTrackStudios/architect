//! The accounts in a window (feature `ui`).
//!
//! [`use_accounts`] is the handle, hearing the controller's changes;
//! [`AccountsUi::shell_account`] the shell's Account item (who is signed
//! in, and the switcher in its menu); [`AccountSheet`] signing in, the
//! accounts on this device and the active one's devices. Mount the sheet
//! once, over the app.

use std::sync::Arc;

use architect::action::ActionList;
use architect_shell::{self as shell, ShellTheme, icon};
use dioxus::prelude::*;
use futures_util::StreamExt as _;
use lucide_dioxus::{Globe, LogIn, LogOut, MonitorSmartphone, UserPlus, Users};

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

    /// Opens the sheet on `page`.
    pub fn open(&self, page: Sheet) {
        self.accounts.read().open_sheet(page);
    }

    /// The shell's Account item: the active account, or "Not signed in",
    /// its menu `base` (the app's own rows) after the switcher's: the other
    /// accounts, add one, the accounts and devices, sign out.
    #[must_use]
    pub fn shell_account(&self, base: shell::Menu) -> shell::Account {
        let file = self.snapshot();
        let me = *self;
        let Some(active) = file.active().cloned() else {
            let mut menu = shell::Menu::new()
                .item("Sign in…", icon!(LogIn), move |()| me.open(Sheet::SignIn));
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
        let mut menu = shell::Menu::new();
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
                me.open(Sheet::SignIn);
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
            "data-account-sheet": if page == Sheet::SignIn { "sign-in" } else { "accounts" },
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
                "aria-label": if page == Sheet::SignIn { "Sign in" } else { "Accounts and devices" },
                style: "width: 420px; max-width: 92%; max-height: 88%; overflow-y: auto; display: flex; flex-direction: column; \
                        gap: 16px; padding: 22px; border-radius: 14px; background: {t.pill}; border: 1px solid {t.rule}; \
                        box-shadow: {t.shadow_float}; color: {t.ink};",
                onclick: move |e| e.stop_propagation(),
                if page == Sheet::SignIn {
                    SignInPage {}
                } else {
                    AccountsPage {}
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

/// What the password field shows for each character.
const DOT: char = '•';

/// A password field: focusable, typed into by keys, its text drawn as dots
/// (Blitz has no masked input). Return calls `on_enter`. While it has the
/// focus the shell is told a field is typed in, so no single-key command
/// fires.
#[component]
fn PasswordField(value: Signal<String>, on_enter: Callback<()>) -> Element {
    let t = theme();
    let mut focused = use_signal(|| false);
    let mut value = value;
    let dots = DOT.to_string().repeat(value.read().chars().count());
    let ring = if focused() {
        t.accent.to_string()
    } else {
        t.rule.to_string()
    };
    let field = field_style(&t);
    rsx! {
        div {
            id: "account-password",
            role: "textbox",
            "aria-label": "Password",
            tabindex: "0",
            style: "{field} display: flex; align-items: center; outline: none; border-color: {ring}; letter-spacing: 2px; overflow: hidden;",
            onfocus: move |_| {
                focused.set(true);
                shell::set_typing(true);
            },
            onblur: move |_| {
                focused.set(false);
                shell::set_typing(false);
            },
            onkeydown: move |e: KeyboardEvent| {
                let held = e.modifiers();
                match e.key() {
                    Key::Named(NamedKey::Enter) => {
                        e.prevent_default();
                        on_enter.call(());
                    }
                    Key::Named(NamedKey::Backspace) => {
                        e.prevent_default();
                        e.stop_propagation();
                        value.with_mut(|v| {
                            v.pop();
                        });
                    }
                    Key::Character(c) if !held.ctrl() && !held.meta() => {
                        e.prevent_default();
                        e.stop_propagation();
                        value.with_mut(|v| v.push_str(&c));
                    }
                    _ => {}
                }
            },
            span { "{dots}" }
            if focused() {
                span { style: "flex-shrink: 0; width: 1px; height: 18px; margin-left: 1px; background: {t.ink};" }
            }
        }
    }
}

#[component]
fn SignInPage() -> Element {
    let Some(ui) = try_use_accounts() else {
        return rsx! {};
    };
    let _ = (ui.rev)();
    let accounts = ui.accounts();
    let t = theme();
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let status = accounts.status();
    let busy = matches!(status, Status::Busy(_));
    let wait = accounts.browser();
    let issuer = host(accounts.issuer().origin());
    let has_accounts = !accounts.snapshot().accounts.is_empty();
    let mut submit = {
        let a = accounts.clone();
        move || {
            let (e, p) = (email.peek().clone(), password.peek().clone());
            password.set(String::new());
            let a = a.clone();
            spawn(async move {
                let _ = a.sign_in_password(&e, &p).await;
            });
        }
    };
    let on_enter = {
        let mut submit = submit.clone();
        move |e: KeyboardEvent| {
            if e.key() == Key::Named(NamedKey::Enter) {
                e.prevent_default();
                submit();
            }
        }
    };
    let mut enter = submit.clone();
    let (browser, cancel, reopen, back, later) = (
        accounts.clone(),
        accounts.clone(),
        wait.as_ref().map(|w| w.url.clone()),
        accounts.clone(),
        accounts,
    );
    let field = field_style(&t);
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 4px;",
            div { style: "font-size: 18px; font-weight: 650;", "Sign in" }
            div { style: "font-size: 13px; color: {t.ink_2}; line-height: 1.4;",
                "Optional: the app works signed out. Sign in at {issuer} to use your account and see its devices here."
            }
        }
        if let Some(w) = wait {
            div { "data-browser-code": "{w.user_code}",
                style: "display: flex; flex-direction: column; gap: 10px; padding: 14px; border-radius: 10px; background: {t.active};",
                div { style: "font-size: 13px; color: {t.ink_2};",
                    if w.opened { "Approve this code on the page that opened in your browser:" } else { "Open the link below and approve this code:" }
                }
                div { style: "font-size: 26px; font-weight: 700; letter-spacing: 3px; font-family: ui-monospace, Menlo, monospace;", "{w.user_code}" }
                div { style: "font-size: 12px; color: {t.ink_3}; word-break: break-all;", "{w.url}" }
                div { style: "display: flex; gap: 8px;",
                    button { style: button_style(&t, false),
                        onclick: move |_| {
                            if let Some(url) = reopen.clone() {
                                let _ = webbrowser::open(&url);
                            }
                        },
                        Globe { size: 14 }
                        "Open the page"
                    }
                    button { style: button_style(&t, false), onclick: move |_| cancel.cancel_browser(), "Cancel" }
                }
            }
        } else {
            button {
                "data-sign-in": "browser",
                style: button_style(&t, true),
                disabled: busy,
                onclick: move |_| browser.sign_in_browser(),
                Globe { size: 16 }
                "Continue in the browser"
            }
            div { style: "font-size: 12px; color: {t.ink_3}; text-align: center;", "or with your email and password" }
            label { r#for: "account-email", style: "font-size: 13px; color: {t.ink_2};", "Email" }
            input {
                id: "account-email",
                r#type: "email",
                value: "{email}",
                autocomplete: "username",
                style: "{field}",
                oninput: move |e: FormEvent| email.set(e.value()),
                onkeydown: on_enter,
            }
            label { r#for: "account-password", style: "font-size: 13px; color: {t.ink_2};", "Password" }
            // Blitz draws an `input type=password` as typed: the field is
            // the sheet's own, its dots drawn here.
            PasswordField { value: password, on_enter: move |()| enter() }
            button {
                "data-sign-in": "password",
                style: button_style(&t, false),
                disabled: busy,
                onclick: move |_| submit(),
                LogIn { size: 14 }
                "Sign in"
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
        button { style: button_style(&t, false), onclick: move |_| add.open_sheet(Sheet::SignIn),
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
                button { style: button_style(&t, false), onclick: move |_| ui.open(Sheet::SignIn), "Sign in" }
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
