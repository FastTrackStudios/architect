//! The app's interface for extensions, in its window (feature `ui`).
//!
//! `crate::hostui`'s calls, drawn: [`WindowUi`] answers an extension's `notify`,
//! `clipboard_*`, `pick`, `prompt` and `menu` calls, and [`HostUiLayer`]
//! draws them over the window — toasts at its corner, a prompt as a
//! dialog, a menu at its point. `ExtensionOverlays` mounts the layer (and
//! gives the host its `WindowUi` unless the app gave an interface of its
//! own), so an app that mounts those has it all.
//!
//! The clipboard is the system's (`hostui::system_clipboard_read`); the
//! pickers are the platform's own (`rfd`, feature `pickers`: the portal on
//! Linux, the system's panels on macOS and Windows), the same the shell's
//! file inputs use; on iOS `UIKit`'s document and photo pickers
//! (`crate::pickers_ios`); without the feature, or on a platform without
//! them, a pick is [`HostError::Unsupported`].
//!
//! [`Decorations`] draws what extensions set on a row or a tab
//! (`Host::decorations_of`); a file tree puts one after each row's name.
//! The workspace's tabs draw theirs through `architect_workspace::TabBadges`,
//! which `use_extension_ui` provides.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dioxus::prelude::*;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use serde_json::Value;

use crate::host::{Host, now_ms};
use crate::hostcall::{Choice, HostError, Menu, MenuItem, Pick, Prompt, PromptKind, Target};
use crate::hostui::{Asked, Asker, HostUi, Notification};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How long a toast shows.
pub const TOAST_FOR: Duration = Duration::from_secs(6);

/// A notification showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub notification: Notification,
    /// When it goes, in ms since the epoch.
    pub until_ms: u64,
}

/// What an extension is asking the person, waiting for the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asking {
    Prompt(Prompt),
    Menu(Menu),
}

/// One question waiting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub id: u64,
    pub from: Asker,
    pub asking: Asking,
}

type Heard = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct State {
    toasts: Vec<Toast>,
    waiting: VecDeque<(Waiting, oneshot::Sender<Option<Value>>)>,
    next: u64,
    listeners: Vec<Heard>,
}

/// The window's interface for extensions. Cheap to clone; clones share it.
#[derive(Clone, Default)]
pub struct WindowUi(Arc<Mutex<State>>);

impl std::fmt::Debug for WindowUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = lock(&self.0);
        f.debug_struct("WindowUi")
            .field("toasts", &s.toasts.len())
            .field("waiting", &s.waiting.len())
            .finish_non_exhaustive()
    }
}

impl PartialEq for WindowUi {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl WindowUi {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `host`'s window interface: the one [`HostUiLayer`] draws, made the
    /// first time, and lent to its extensions unless the app gave them an
    /// interface of its own.
    #[must_use]
    pub fn lend(host: &Host) -> Self {
        host.window_slot(|| {
            let ui = Self::new();
            let _ = host.offer_ui(Arc::new(ui.clone()));
            ui
        })
    }

    /// The toasts showing now (the ones past their time gone).
    #[must_use]
    pub fn toasts(&self) -> Vec<Toast> {
        let now = now_ms();
        let mut s = lock(&self.0);
        s.toasts.retain(|t| t.until_ms > now);
        s.toasts.clone()
    }

    /// The questions waiting, the first the one showing.
    #[must_use]
    pub fn waiting(&self) -> Vec<Waiting> {
        lock(&self.0)
            .waiting
            .iter()
            .map(|(w, _)| w.clone())
            .collect()
    }

    /// The person's answer to question `id` (`None`: cancelled).
    pub fn answer(&self, id: u64, answer: Option<Value>) {
        let sender = {
            let mut s = lock(&self.0);
            let at = s.waiting.iter().position(|(w, _)| w.id == id);
            let taken = at.and_then(|i| s.waiting.remove(i)).map(|(_, tx)| tx);
            drop(s);
            taken
        };
        if let Some(tx) = sender {
            let _ = tx.send(answer);
        }
        self.tell();
    }

    /// Takes toast `id` away.
    pub fn dismiss(&self, id: u64) {
        lock(&self.0).toasts.retain(|t| t.id != id);
        self.tell();
    }

    /// Calls `listener` on every change (from any thread).
    pub fn listen(&self, listener: impl Fn() + Send + Sync + 'static) {
        lock(&self.0).listeners.push(Arc::new(listener));
    }

    fn tell(&self) {
        let heard: Vec<Heard> = lock(&self.0).listeners.clone();
        for h in heard {
            h();
        }
    }

    const fn next_id(s: &mut State) -> u64 {
        s.next = s.next.wrapping_add(1);
        s.next
    }

    fn ask(&self, from: &Asker, asking: Asking) -> oneshot::Receiver<Option<Value>> {
        let (tx, rx) = oneshot::channel();
        {
            let mut s = lock(&self.0);
            let id = Self::next_id(&mut s);
            s.waiting.push_back((
                Waiting {
                    id,
                    from: from.clone(),
                    asking,
                },
                tx,
            ));
        }
        self.tell();
        rx
    }
}

impl HostUi for WindowUi {
    fn notify(&self, notification: Notification) -> Result<(), HostError> {
        {
            let mut s = lock(&self.0);
            let id = Self::next_id(&mut s);
            let until_ms =
                now_ms().saturating_add(u64::try_from(TOAST_FOR.as_millis()).unwrap_or(6000));
            s.toasts.push(Toast {
                id,
                notification,
                until_ms,
            });
            // The last few only.
            let over = s.toasts.len().saturating_sub(4);
            s.toasts.drain(..over);
        }
        self.tell();
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn clipboard_read(&self) -> Result<Option<String>, HostError> {
        crate::hostui::system_clipboard_read()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn clipboard_write(&self, text: &str) -> Result<(), HostError> {
        crate::hostui::system_clipboard_write(text)
    }

    fn pick(&self, from: &Asker, pick: Pick) -> Asked<Option<Vec<String>>> {
        let _ = from;
        native_pick(pick)
    }

    fn prompt(&self, from: &Asker, prompt: Prompt) -> Asked<Option<Value>> {
        let rx = self.ask(from, Asking::Prompt(prompt));
        Box::pin(async move { Ok(rx.await.unwrap_or(None)) })
    }

    fn menu(&self, from: &Asker, menu: Menu) -> Asked<Option<String>> {
        let rx = self.ask(from, Asking::Menu(menu));
        Box::pin(async move {
            Ok(rx
                .await
                .unwrap_or(None)
                .and_then(|v| v.as_str().map(str::to_owned)))
        })
    }
}

/// The platform's picker, on a thread of its own (a portal's or a panel's
/// wait is not the window's).
#[cfg(all(
    feature = "pickers",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn native_pick(pick: Pick) -> Asked<Option<Vec<String>>> {
    use crate::hostcall::PickKind;
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let mut dialog = rfd::AsyncFileDialog::new();
        if let Some(title) = &pick.title {
            dialog = dialog.set_title(title.as_str());
        }
        // No photo library here: a photo is an image file.
        if pick.kind.photos() && pick.filters.is_empty() {
            let images = crate::hostcall::image_filter();
            dialog = dialog.add_filter(images.name.as_str(), &images.extensions);
        }
        for f in &pick.filters {
            dialog = dialog.add_filter(f.name.as_str(), &f.extensions);
        }
        if let Some(folder) = &pick.folder {
            dialog = dialog.set_directory(folder);
        }
        if let Some(name) = &pick.name {
            dialog = dialog.set_file_name(name.as_str());
        }
        let path = |h: rfd::FileHandle| h.path().display().to_string();
        let picked = futures::executor::block_on(async move {
            match pick.kind {
                PickKind::Open | PickKind::Photo => dialog.pick_file().await.map(|h| vec![path(h)]),
                PickKind::OpenMany | PickKind::Photos => dialog
                    .pick_files()
                    .await
                    .map(|hs| hs.into_iter().map(path).collect()),
                PickKind::Folder => dialog.pick_folder().await.map(|h| vec![path(h)]),
                PickKind::Save => dialog.save_file().await.map(|h| vec![path(h)]),
            }
        });
        let _ = tx.send(picked);
    });
    Box::pin(async move { Ok(rx.await.unwrap_or(None)) })
}

/// `UIKit`'s pickers: the document picker (files, folders, a save as a
/// folder and a name) and the photo picker.
#[cfg(all(feature = "pickers", target_os = "ios"))]
fn native_pick(pick: Pick) -> Asked<Option<Vec<String>>> {
    crate::pickers_ios::pick(pick)
}

#[cfg(not(all(
    feature = "pickers",
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "ios"
    )
)))]
fn native_pick(pick: Pick) -> Asked<Option<Vec<String>>> {
    let _ = pick;
    Box::pin(async { Err(crate::hostui::unsupported("file pickers on this platform")) })
}

/// The window's interface ([`WindowUi::lend`]), its revision a signal
/// that moves on every change.
fn use_window_ui(host: &Host) -> (WindowUi, Signal<u64>) {
    let ui = use_hook(|| WindowUi::lend(host));
    let mut revision = use_signal(|| 0_u64);
    let heard = ui.clone();
    use_future(move || {
        let ui = heard.clone();
        async move {
            let (tx, mut rx) = mpsc::unbounded::<()>();
            ui.listen(move || {
                let _ = tx.unbounded_send(());
            });
            loop {
                // Wake for a change, or for the next toast's time.
                let soonest = ui.toasts().iter().map(|t| t.until_ms).min();
                let wait = soonest.map_or(Duration::from_secs(3600), |until| {
                    Duration::from_millis(until.saturating_sub(now_ms()).saturating_add(20))
                });
                let timer = futures_timer::Delay::new(wait);
                futures::pin_mut!(timer);
                if let futures::future::Either::Left((None, _)) =
                    futures::future::select(rx.next(), timer).await
                {
                    break;
                }
                while rx.try_recv().is_ok() {}
                revision.with_mut(|r| *r = r.wrapping_add(1));
            }
        }
    });
    (ui, revision)
}

/// How high extensions' toasts, prompts and menus are stacked.
///
/// Over everything, a tour's layer too (`architect_guide::tour::TourLayer`, 200),
/// so a prompt a step brings up is answered, not hidden under its card.
pub const PROMPT_LAYER: u32 = 300;

/// How high the toolbar editor (its bar, settings sheet and pickers) is
/// stacked: over a tour's card (200), which otherwise sits on the bar's
/// Done at the window's bottom middle, and under extensions' prompts
/// ([`PROMPT_LAYER`]), which an edit may bring up.
pub(crate) const EDITOR_LAYER: u32 = 250;
const _: () = assert!(EDITOR_LAYER > 200 && EDITOR_LAYER < PROMPT_LAYER);

/// Extensions' toasts, prompts and menus, over the window: mount it once,
/// last, at the root (`ExtensionOverlays` does), over a tour's layer
/// ([`PROMPT_LAYER`]).
#[component]
pub fn HostUiLayer(host: Host) -> Element {
    let (ui, revision) = use_window_ui(&host);
    let _ = revision();
    let toasts = ui.toasts();
    let first = ui.waiting().into_iter().next();
    let shown = toasts.len();
    let look = layer_look(&crate::window_ui::shell_theme());
    rsx! {
        if shown > 0 {
            div {
                style: "position: absolute; right: 16px; bottom: 16px; z-index: {PROMPT_LAYER}; display: flex; \
                        flex-direction: column; gap: 8px; max-width: 360px; {look}",
                "data-ext-toasts": "{shown}",
                for toast in toasts {
                    ToastRow { key: "{toast.id}", ui: ui.clone(), toast }
                }
            }
        }
        if let Some(waiting) = first {
            match waiting.asking.clone() {
                Asking::Prompt(prompt) => rsx! {
                    PromptDialog { key: "{waiting.id}", ui, id: waiting.id, from: waiting.from.name, prompt, look }
                },
                Asking::Menu(menu) => rsx! {
                    MenuAt { key: "{waiting.id}", ui, id: waiting.id, menu, look }
                },
            }
        }
    }
}

/// The shell's face and colours, as the CSS this layer's rows read.
///
/// The layer is mounted at the window's root, over the app: Blitz's
/// default face there is a serif, and the theme's `--card` and friends
/// were not the person's theme (a light theme drew a black prompt). So
/// the layer sets them itself, from the shell's theme, as the toolbars and
/// extension windows draw theirs.
fn layer_look(t: &architect_shell::ShellTheme) -> String {
    format!(
        "font-family: {font}; --background: {desk}; --foreground: {ink}; --card: {pill}; \
         --card-foreground: {ink}; --popover: {pill}; --popover-foreground: {ink}; --border: {rule}; \
         --input: {rule}; --muted-foreground: {ink_3}; --primary: {accent}; --primary-foreground: {accent_ink}; \
         --secondary: {active}; --secondary-foreground: {ink}; --destructive: {danger};",
        font = t.font,
        desk = t.desk,
        ink = t.ink,
        pill = t.pill,
        rule = t.rule,
        ink_3 = t.ink_3,
        accent = t.accent,
        accent_ink = t.accent_ink,
        active = t.active,
        danger = t.danger,
    )
}

fn kind_colour(kind: &str) -> &'static str {
    match kind {
        "success" => "var(--primary)",
        "warning" => "var(--warning, #d97706)",
        "error" => "var(--destructive)",
        _ => "var(--border)",
    }
}

#[component]
fn ToastRow(ui: WindowUi, toast: Toast) -> Element {
    let n = toast.notification.clone();
    let edge = kind_colour(&n.kind);
    let id = toast.id;
    rsx! {
        div {
            style: "display: flex; gap: 10px; align-items: flex-start; padding: 10px 12px; border-radius: 10px; \
                    background: var(--card); color: var(--card-foreground); border: 1px solid var(--border); \
                    border-left: 3px solid {edge}; box-shadow: 0 8px 24px rgba(0,0,0,0.35); font-size: 13px;",
            role: "status",
            "data-ext-toast": "{n.kind}",
            div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px;",
                div { style: "font-size: 11px; color: var(--muted-foreground);",
                    {n.title.clone().unwrap_or_else(|| n.from.name.clone())}
                }
                div { "{n.text}" }
            }
            button {
                style: "border: none; background: transparent; color: var(--muted-foreground); cursor: pointer;",
                "aria-label": "Dismiss",
                onclick: move |_| ui.dismiss(id),
                "✕"
            }
        }
    }
}

const BUTTON: &str = "padding: 6px 14px; border-radius: 6px; border: 1px solid var(--border); \
                      background: var(--secondary); color: var(--secondary-foreground); cursor: pointer; font-size: 13px;";
const PRIMARY: &str = "padding: 6px 14px; border-radius: 6px; border: 1px solid var(--primary); \
                       background: var(--primary); color: var(--primary-foreground); cursor: pointer; font-size: 13px;";

#[component]
fn PromptDialog(ui: WindowUi, id: u64, from: String, prompt: Prompt, look: String) -> Element {
    let mut text = use_signal(|| prompt.default.clone().unwrap_or_default());
    let hidden = prompt.kind == PromptKind::Secret;
    let (yes, no) = (ui.clone(), ui.clone());
    let submit = move || {
        let answer = match prompt.kind {
            PromptKind::Text | PromptKind::Secret => Value::String(text.peek().clone()),
            PromptKind::Confirm => Value::Bool(true),
            PromptKind::PickOne => return,
        };
        yes.answer(id, Some(answer));
    };
    let ok = submit.clone();
    let on_enter = submit;
    let cancel = move || {
        // A confirm dismissed is a no; anything else, a cancel.
        let answer = (prompt.kind == PromptKind::Confirm).then_some(Value::Bool(false));
        no.answer(id, answer);
    };
    let on_escape = cancel.clone();
    let on_backdrop = cancel.clone();
    let on_cancel = cancel;
    let choices = prompt.choices.clone();
    rsx! {
        div {
            style: "position: absolute; inset: 0; z-index: {PROMPT_LAYER + 1}; display: flex; align-items: center; \
                    justify-content: center; background: rgba(0,0,0,0.45); {look}",
            onclick: move |_| on_backdrop(),
            div {
                style: "width: 420px; max-width: calc(100% - 32px); display: flex; flex-direction: column; gap: 12px; \
                        padding: 18px; border-radius: 12px; background: var(--card); color: var(--card-foreground); \
                        border: 1px solid var(--border); box-shadow: 0 16px 44px rgba(0,0,0,0.5);",
                role: "dialog",
                "aria-modal": "true",
                "data-ext-prompt": "{prompt_kind(prompt.kind)}",
                onclick: move |e| e.stop_propagation(),
                onkeydown: move |e: KeyboardEvent| match e.key() {
                    Key::Named(NamedKey::Escape) => on_escape(),
                    Key::Named(NamedKey::Enter) if prompt.kind != PromptKind::PickOne => on_enter(),
                    _ => {}
                },
                div { style: "font-size: 11px; color: var(--muted-foreground);", "{from}" }
                div { style: "font-size: 15px; font-weight: 600;", "{prompt.title}" }
                if let Some(message) = &prompt.message {
                    div { style: "font-size: 13px; color: var(--muted-foreground);", "{message}" }
                }
                if matches!(prompt.kind, PromptKind::Text | PromptKind::Secret) {
                    div { style: "position: relative; display: flex; flex-direction: column;",
                        input {
                            r#type: if hidden { "password" } else { "text" },
                            // A secret's text is not drawn (Blitz draws a
                            // password field's text): dots over it say how
                            // much was typed.
                            style: if hidden { SECRET_FIELD } else { FIELD },
                            value: "{text}",
                            placeholder: prompt.placeholder.clone().unwrap_or_default(),
                            autofocus: true,
                            oninput: move |e| text.set(e.value()),
                        }
                        if hidden {
                            div {
                                style: "position: absolute; left: 11px; top: 0; bottom: 0; display: flex; \
                                        align-items: center; pointer-events: none; font-size: 13px; \
                                        letter-spacing: 1px; color: var(--foreground);",
                                {MASK.to_string().repeat(text.read().chars().count())}
                            }
                        }
                    }
                }
                if prompt.kind == PromptKind::PickOne {
                    div { style: "display: flex; flex-direction: column; gap: 4px; max-height: 320px; overflow-y: auto;",
                        for choice in choices {
                            ChoiceRow { key: "{choice.id}", ui: ui.clone(), id, choice }
                        }
                    }
                }
                div { style: "display: flex; justify-content: flex-end; gap: 8px;",
                    button { style: BUTTON, onclick: move |_| on_cancel(), "Cancel" }
                    if prompt.kind != PromptKind::PickOne {
                        button { style: PRIMARY, onclick: move |_| ok(), "OK" }
                    }
                }
            }
        }
    }
}

/// What a secret's field shows for each character.
const MASK: char = '•';

/// A prompt's text field.
const FIELD: &str = "padding: 7px 10px; border-radius: 6px; border: 1px solid var(--input); \
                     background: var(--background); color: var(--foreground); font-size: 13px;";
/// A secret's: its text not drawn.
const SECRET_FIELD: &str = "padding: 7px 10px; border-radius: 6px; border: 1px solid var(--input); \
                            background: var(--background); color: transparent; \
                            caret-color: var(--foreground); font-size: 13px;";

const fn prompt_kind(kind: PromptKind) -> &'static str {
    match kind {
        PromptKind::Text => "text",
        PromptKind::Confirm => "confirm",
        PromptKind::PickOne => "pick_one",
        PromptKind::Secret => "secret",
    }
}

#[component]
fn ChoiceRow(ui: WindowUi, id: u64, choice: Choice) -> Element {
    let chosen = choice.id.clone();
    rsx! {
        button {
            style: "display: flex; flex-direction: column; align-items: flex-start; gap: 2px; padding: 8px 10px; \
                    border-radius: 6px; border: 1px solid var(--border); background: var(--background); \
                    color: var(--foreground); cursor: pointer; text-align: left; font-size: 13px;",
            "data-choice": "{choice.id}",
            onclick: move |_| ui.answer(id, Some(Value::String(chosen.clone()))),
            span { "{choice.label}" }
            if let Some(detail) = &choice.detail {
                span { style: "font-size: 11px; color: var(--muted-foreground);", "{detail}" }
            }
        }
    }
}

#[component]
fn MenuAt(ui: WindowUi, id: u64, menu: Menu, look: String) -> Element {
    let away = ui.clone();
    let (x, y) = (menu.x.unwrap_or(-1.0), menu.y.unwrap_or(-1.0));
    let place = if x < 0.0 || y < 0.0 {
        "left: 50%; top: 40%; transform: translate(-50%, -50%);".to_owned()
    } else {
        format!("left: {x}px; top: {y}px;")
    };
    rsx! {
        div {
            style: "position: absolute; inset: 0; z-index: {PROMPT_LAYER + 1}; {look}",
            onclick: move |_| away.answer(id, None),
            oncontextmenu: {
                let away = ui.clone();
                move |e: MouseEvent| {
                    e.prevent_default();
                    away.answer(id, None);
                }
            },
            onkeydown: {
                let away = ui;
                move |e: KeyboardEvent| if e.key() == Key::Named(NamedKey::Escape) { away.answer(id, None) }
            },
            div {
                style: "position: absolute; {place} min-width: 180px; padding: 4px; display: flex; flex-direction: column; \
                        border-radius: 8px; background: var(--popover); color: var(--popover-foreground); \
                        border: 1px solid var(--border); box-shadow: 0 10px 30px rgba(0,0,0,0.45); font-size: 13px;",
                role: "menu",
                "data-ext-menu": "{menu.items.len()}",
                onclick: move |e| e.stop_propagation(),
                for (i, item) in menu.items.iter().cloned().enumerate() {
                    MenuRow { key: "{i}", ui: ui.clone(), id, item }
                }
            }
        }
    }
}

#[component]
fn MenuRow(ui: WindowUi, id: u64, item: MenuItem) -> Element {
    if item.separator {
        return rsx! { div { style: "height: 1px; margin: 4px 6px; background: var(--border);" } };
    }
    let chosen = item.id.clone();
    let disabled = item.disabled;
    let ink = if disabled {
        "var(--muted-foreground)"
    } else {
        "inherit"
    };
    rsx! {
        button {
            style: "display: flex; align-items: center; gap: 8px; padding: 6px 10px; border: none; border-radius: 5px; \
                    background: transparent; color: {ink}; text-align: left; cursor: pointer; font-size: 13px;",
            role: "menuitem",
            "data-menu-item": "{item.id}",
            disabled,
            onclick: move |_| if !disabled { ui.answer(id, Some(Value::String(chosen.clone()))) },
            if let Some(icon) = &item.icon {
                IconOrText { name: icon.clone(), size: 14, color: "currentColor".to_owned() }
            }
            span { style: "flex: 1;", "{item.label}" }
            if let Some(keys) = &item.shortcut {
                span { style: "font-size: 11px; color: var(--muted-foreground);", "{keys}" }
            }
        }
    }
}

/// An icon by name (`star`), or the text it is (an emoji).
#[component]
pub fn IconOrText(name: String, size: usize, color: String) -> Element {
    let named = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if named {
        let icon = architect_shell::named_icon(&name);
        return icon(architect_shell::Icon {
            size,
            color: color.into(),
        });
    }
    rsx! { span { style: "font-size: {size}px; line-height: 1;", "{name}" } }
}

/// The decorations' revision, as a signal: it moves when an extension
/// sets one, and when one loads or unloads (its manifest's go with it).
#[must_use]
pub fn use_decorations(host: &Host) -> Signal<u64> {
    let mut revision = use_signal(|| host.decorations().revision());
    let h = host.clone();
    use_future(move || {
        let h = h.clone();
        async move {
            let (tx, mut rx) = mpsc::unbounded::<()>();
            let again = tx.clone();
            h.decorations().listen(move || {
                let _ = tx.unbounded_send(());
            });
            h.on_event(move |_| {
                let _ = again.unbounded_send(());
            });
            while rx.next().await.is_some() {
                while rx.try_recv().is_ok() {}
                revision.with_mut(|r| *r = r.wrapping_add(1));
            }
        }
    });
    revision
}

/// What extensions set on `target` (a file-tree row: `Target::file(path)`;
/// a tab: `Target::tab(…)`), drawn after its label: each one's icon, then
/// its badge.
#[component]
pub fn Decorations(host: Host, target: Target) -> Element {
    let revision = use_decorations(&host);
    let _ = revision();
    let all = host.decorations_of(&target);
    if all.is_empty() {
        return rsx! {};
    }
    rsx! {
        span {
            style: "display: inline-flex; align-items: center; gap: 4px; flex: 0 0 auto;",
            "data-decorations": "{target.kind}:{target.id}",
            for (i, d) in all.into_iter().enumerate() {
                Decorated { key: "{i}", from: d.from, decoration: d.decoration }
            }
        }
    }
}

/// A theme token's colour (`primary`), or the CSS colour given.
fn colour(c: &str) -> String {
    if c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '-') {
        format!("var(--{c})")
    } else {
        c.to_owned()
    }
}

#[component]
fn Decorated(from: String, decoration: crate::hostcall::Decoration) -> Element {
    let tip = decoration.tooltip.clone().unwrap_or_default();
    let fill = colour(decoration.color.as_deref().unwrap_or("secondary"));
    let ink = if decoration.color.is_some() {
        "var(--primary-foreground)"
    } else {
        "var(--secondary-foreground)"
    };
    rsx! {
        span {
            style: "display: inline-flex; align-items: center; gap: 3px;",
            title: "{tip}",
            "data-decoration-from": "{from}",
            if let Some(icon) = &decoration.icon {
                IconOrText { name: icon.clone(), size: 12, color: "currentColor".to_owned() }
            }
            if let Some(badge) = &decoration.badge {
                span {
                    style: "padding: 0 5px; min-width: 16px; height: 16px; box-sizing: border-box; border-radius: 8px; \
                            display: inline-flex; align-items: center; justify-content: center; font-size: 10px; \
                            font-weight: 600; background: {fill}; color: {ink};",
                    "{badge}"
                }
            }
        }
    }
}

/// Provides the workspace's tabs their icons and badges: what extensions
/// set on `Target::tab(<view kind>)` and `Target::tab(<pane id>)`
/// (`use_extension_ui` calls it).
pub fn use_tab_badges(host: &Host) {
    let host = host.clone();
    let revision = use_decorations(&host);
    use_context_provider(move || {
        architect_workspace::TabBadges::new(move |kind, pane| {
            // Read in the tab's render: it draws again as they change.
            let _ = revision();
            let (by_kind, by_pane) = (Target::tab(kind), Target::tab(pane));
            if host.decorations_of(&by_kind).is_empty() && host.decorations_of(&by_pane).is_empty()
            {
                return None;
            }
            Some(rsx! {
                Decorations { host: host.clone(), target: by_kind }
                Decorations { host: host.clone(), target: by_pane }
            })
        })
    });
}
