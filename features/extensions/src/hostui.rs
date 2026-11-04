//! The app's interface, as the host service lends it to extensions (#306).
//!
//! Notifications, the clipboard, pickers and save dialogs, prompts, a
//! context menu at a point, and icons and badges on file-tree rows and
//! tabs. The calls are `crate::hostcall`'s (`notify`, `clipboard_read`,
//! `clipboard_write`, `pick`, `prompt`, `menu`, `decorate`), each gated by
//! `capabilities.ui` (`crate::capabilities::UI`); the host answers them
//! through the app's [`HostUi`] ([`Host::set_ui`](crate::host::Host::set_ui)):
//!
//! - [`NoUi`], the default: notifications go to the extension's log only,
//!   and the rest are [`HostError::Unsupported`] (a headless host has no
//!   one to ask);
//! - [`FakeUi`], for tests and headless runs: it keeps the notifications
//!   and a clipboard of its own, and answers every question at once, with
//!   the answers it was given or deterministic ones;
//! - the window's, `crate::host_ui_view::WindowUi` (feature `ui`): toasts,
//!   a dialog, a menu over the app's window, the platform's pickers (feature
//!   `pickers`), the system clipboard.
//!
//! Decorations are not the interface's to keep: [`Decorations`] holds
//! what each extension set, and with the `app.decorations` contribution
//! point (a manifest's `[[contribute."app.decorations"]]`) is what a file
//! tree or a tab bar consults
//! ([`Host::decorations_of`](crate::host::Host::decorations_of)).

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use serde_json::Value;

use crate::hostcall::{
    Decoration, HostError, HostRequest, Menu, Pick, PickKind, Prompt, PromptKind, Target,
};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An answer the person gives later.
pub type Asked<T> = Pin<Box<dyn Future<Output = Result<T, HostError>> + Send>>;

/// The extension asking: what the interface names it by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Asker {
    pub id: String,
    /// Its manifest's name.
    pub name: String,
}

/// A notification an extension shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Notification {
    pub from: Asker,
    pub text: String,
    /// `info`, `success`, `warning` or `error`.
    pub kind: String,
    pub title: Option<String>,
}

/// The app's interface, for extensions: every method's default is
/// "this app cannot" ([`HostError::Unsupported`]).
pub trait HostUi: Send + Sync {
    /// Shows a notification (it is in the extension's log already).
    fn notify(&self, notification: Notification) -> Result<(), HostError> {
        let _ = notification;
        Ok(())
    }

    /// The clipboard's text, when it holds some.
    fn clipboard_read(&self) -> Result<Option<String>, HostError> {
        Err(unsupported("the clipboard"))
    }

    /// Puts `text` on the clipboard.
    fn clipboard_write(&self, text: &str) -> Result<(), HostError> {
        let _ = text;
        Err(unsupported("the clipboard"))
    }

    /// A picker or save dialog: the paths picked (one, but for
    /// `open_many`), none when the person cancelled.
    fn pick(&self, from: &Asker, pick: Pick) -> Asked<Option<Vec<String>>> {
        let _ = (from, pick);
        Box::pin(async { Err(unsupported("file pickers")) })
    }

    /// A question: the answer ([`Prompt`]'s), none when cancelled.
    fn prompt(&self, from: &Asker, prompt: Prompt) -> Asked<Option<Value>> {
        let _ = (from, prompt);
        Box::pin(async { Err(unsupported("prompts")) })
    }

    /// A context menu: the id of the item chosen, none when dismissed.
    fn menu(&self, from: &Asker, menu: Menu) -> Asked<Option<String>> {
        let _ = (from, menu);
        Box::pin(async { Err(unsupported("context menus")) })
    }
}

/// [`HostError::Unsupported`], saying what.
#[must_use]
pub fn unsupported(what: &str) -> HostError {
    HostError::Unsupported {
        what: format!("this app has no {what} for extensions"),
    }
}

/// No interface: a headless host's. Notifications are log lines; the rest
/// are [`HostError::Unsupported`].
#[derive(Clone, Copy, Debug, Default)]
pub struct NoUi;

impl HostUi for NoUi {}

/// One question a [`FakeUi`] was asked (a `pick`, `prompt` or `menu`
/// request), and by whom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub from: String,
    pub request: HostRequest,
}

#[derive(Debug, Default)]
struct Fake {
    notices: Vec<Notification>,
    clipboard: Option<String>,
    answers: VecDeque<Value>,
    asked: Vec<Question>,
}

/// An interface that answers at once, for tests and headless runs. Cheap
/// to clone; clones share it.
///
/// Each question takes the next answer given ([`Self::answer`]: JSON as
/// the call answers, `null` to cancel); with none left it answers the
/// same way every time: a text prompt its default (or `""`), a confirm
/// yes, a pick-one its first choice, a menu its first item that may be
/// chosen, a picker cancelled.
#[derive(Clone, Debug, Default)]
pub struct FakeUi(Arc<Mutex<Fake>>);

impl FakeUi {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The next question's answer.
    #[must_use]
    pub fn answer(self, value: Value) -> Self {
        lock(&self.0).answers.push_back(value);
        self
    }

    /// The notifications shown, in order.
    #[must_use]
    pub fn notices(&self) -> Vec<Notification> {
        lock(&self.0).notices.clone()
    }

    /// Its clipboard's text.
    #[must_use]
    pub fn clipboard(&self) -> Option<String> {
        lock(&self.0).clipboard.clone()
    }

    /// Puts `text` on its clipboard (what a person copied).
    pub fn copy(&self, text: &str) {
        lock(&self.0).clipboard = Some(text.to_owned());
    }

    /// The questions asked, in order.
    #[must_use]
    pub fn asked(&self) -> Vec<Question> {
        lock(&self.0).asked.clone()
    }

    fn next(&self, from: &Asker, request: HostRequest) -> Option<Value> {
        let mut f = lock(&self.0);
        f.asked.push(Question {
            from: from.id.clone(),
            request,
        });
        f.answers.pop_front()
    }
}

impl HostUi for FakeUi {
    fn notify(&self, notification: Notification) -> Result<(), HostError> {
        lock(&self.0).notices.push(notification);
        Ok(())
    }

    fn clipboard_read(&self) -> Result<Option<String>, HostError> {
        Ok(self.clipboard())
    }

    fn clipboard_write(&self, text: &str) -> Result<(), HostError> {
        self.copy(text);
        Ok(())
    }

    fn pick(&self, from: &Asker, pick: Pick) -> Asked<Option<Vec<String>>> {
        let answer = self.next(from, HostRequest::Pick(pick));
        let paths = match answer {
            None | Some(Value::Null) => None,
            Some(Value::String(p)) => Some(vec![p]),
            Some(Value::Array(ps)) => Some(
                ps.iter()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect(),
            ),
            Some(other) => Some(vec![other.to_string()]),
        };
        Box::pin(async move { Ok(paths) })
    }

    fn prompt(&self, from: &Asker, prompt: Prompt) -> Asked<Option<Value>> {
        let given = self.next(from, HostRequest::Prompt(prompt.clone()));
        let answer = given.map_or_else(
            || match prompt.kind {
                PromptKind::Text => Some(Value::String(prompt.default.unwrap_or_default())),
                PromptKind::Confirm => Some(Value::Bool(true)),
                PromptKind::PickOne => prompt.choices.first().map(|c| Value::String(c.id.clone())),
            },
            |v| (!v.is_null()).then_some(v),
        );
        Box::pin(async move { Ok(answer) })
    }

    fn menu(&self, from: &Asker, menu: Menu) -> Asked<Option<String>> {
        let given = self.next(from, HostRequest::Menu(menu.clone()));
        let answer = given.map_or_else(
            || {
                menu.items
                    .iter()
                    .find(|i| !i.separator && !i.disabled)
                    .map(|i| i.id.clone())
            },
            |v| v.as_str().map(str::to_owned),
        );
        Box::pin(async move { Ok(answer) })
    }
}

/// A pick's answer as the call gives it: a path, a list of them
/// (`open_many`), or `null`.
#[must_use]
pub fn pick_answer(kind: PickKind, paths: Option<Vec<String>>) -> Value {
    match (kind, paths) {
        (_, None) => Value::Null,
        (PickKind::OpenMany, Some(ps)) => Value::from(ps),
        (_, Some(ps)) => ps.into_iter().next().map_or(Value::Null, Value::String),
    }
}

/// The point decorations are contributed to by manifests
/// (`[[contribute."app.decorations"]]`, with `target = {kind, id}` and a
/// [`Decoration`]'s fields); the host owns it.
pub const DECORATIONS_POINT: &str = "app.decorations";

/// [`DECORATIONS_POINT`]'s declaration.
#[must_use]
pub fn decorations_point() -> crate::interop::PointDecl {
    let text = |what: &str| serde_json::json!({ "type": "string", "description": what });
    let schema = serde_json::json!({
        "type": "object",
        "required": ["target"],
        "properties": {
            "target": {
                "type": "object",
                "required": ["kind", "id"],
                "properties": {
                    "kind": text("file (a file-tree row) or tab"),
                    "id": text("the file's path, or the tab's"),
                },
            },
            "icon": text("an icon's name, or a character"),
            "badge": text("a short text after the label"),
            "color": text("the badge's colour: a theme token or a CSS colour"),
            "tooltip": text("what hovering it says"),
        },
    });
    crate::interop::PointDecl {
        id: DECORATIONS_POINT.to_owned(),
        title: "Icons and badges".to_owned(),
        description: "Icons and badges on file-tree rows and tabs.".to_owned(),
        schema: Some(toml::Value::String(schema.to_string())),
        deliver: None,
    }
}

/// One decoration on a target, and who set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Decorated {
    /// The extension's id.
    pub from: String,
    pub decoration: Decoration,
}

type Heard = Arc<dyn Fn() + Send + Sync>;

/// What extensions set on rows and tabs at run time (`decorate`), by
/// target: one decoration an extension a target. Cheap to share inside
/// the host.
#[derive(Default)]
pub struct Decorations {
    set: Mutex<BTreeMap<Target, Vec<Decorated>>>,
    revision: AtomicU64,
    listeners: Mutex<Vec<Heard>>,
}

impl std::fmt::Debug for Decorations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decorations")
            .field("targets", &lock(&self.set).len())
            .field("revision", &self.revision())
            .finish_non_exhaustive()
    }
}

impl Decorations {
    /// `from`'s decoration on `target`, in place of the one it set there
    /// (`None`: takes it off).
    pub fn set(&self, from: &str, target: &Target, decoration: Option<Decoration>) {
        {
            let mut set = lock(&self.set);
            let on = set.entry(target.clone()).or_default();
            on.retain(|d| d.from != from);
            if let Some(decoration) = decoration {
                on.push(Decorated {
                    from: from.to_owned(),
                    decoration,
                });
            }
            if on.is_empty() {
                set.remove(target);
            }
        }
        self.changed();
    }

    /// Everything `from` set goes (it unloaded).
    pub fn withdraw(&self, from: &str) {
        let had = {
            let mut set = lock(&self.set);
            let before: usize = set.values().map(Vec::len).sum();
            for on in set.values_mut() {
                on.retain(|d| d.from != from);
            }
            set.retain(|_, on| !on.is_empty());
            before != set.values().map(Vec::len).sum::<usize>()
        };
        if had {
            self.changed();
        }
    }

    /// What is set on `target`, in the order it was set.
    #[must_use]
    pub fn of(&self, target: &Target) -> Vec<Decorated> {
        lock(&self.set).get(target).cloned().unwrap_or_default()
    }

    /// Everything set, by target.
    #[must_use]
    pub fn all(&self) -> BTreeMap<Target, Vec<Decorated>> {
        lock(&self.set).clone()
    }

    /// Moves on every change.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    /// Calls `listener` on every change (from any thread).
    pub fn listen(&self, listener: impl Fn() + Send + Sync + 'static) {
        lock(&self.listeners).push(Arc::new(listener));
    }

    /// Tells every listener (the points' decorations changed too).
    pub(crate) fn changed(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
        let heard: Vec<Heard> = lock(&self.listeners).iter().map(Arc::clone).collect();
        for h in heard {
            h();
        }
    }
}

/// The system clipboard's text, through the platform's tools
/// (`wl-paste`, `xclip`, `pbpaste`): what a window's interface reads.
///
/// # Errors
///
/// [`HostError::Unsupported`] with none of them; [`HostError::Failed`]
/// when one fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn system_clipboard_read() -> Result<Option<String>, HostError> {
    let tries: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else {
        &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["--clipboard", "--output"]),
        ]
    };
    for (program, args) in tries {
        match std::process::Command::new(program)
            .args(*args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
        {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).into_owned();
                return Ok((!text.is_empty()).then_some(text));
            }
            // An empty clipboard is a failure to wl-paste: nothing there.
            Ok(_) => return Ok(None),
            Err(_) => {}
        }
    }
    Err(unsupported("clipboard tool (wl-paste, xclip or pbpaste)"))
}

/// Puts `text` on the system clipboard (`wl-copy`, `xclip`, `pbcopy`).
///
/// # Errors
///
/// [`HostError::Unsupported`] with none of them; [`HostError::Failed`]
/// when one fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn system_clipboard_write(text: &str) -> Result<(), HostError> {
    use std::io::Write as _;
    let tries: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };
    for (program, args) in tries {
        let Ok(mut child) = std::process::Command::new(program)
            .args(*args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        let wrote = child
            .stdin
            .take()
            .map(|mut stdin| stdin.write_all(text.as_bytes()));
        let status = child.wait();
        return match (wrote, status) {
            (Some(Ok(())), Ok(s)) if s.success() => Ok(()),
            (_, status) => Err(HostError::Failed {
                message: format!("{program} did not take the text ({status:?})"),
            }),
        };
    }
    Err(unsupported("clipboard tool (wl-copy, xclip or pbcopy)"))
}

#[cfg(test)]
mod tests {
    use super::{Asker, Decorations, FakeUi, HostUi as _, pick_answer};
    use crate::hostcall::{
        Choice, Decoration, Menu, MenuItem, PickKind, Prompt, PromptKind, Target,
    };
    use futures::executor::block_on;
    use serde_json::{Value, json};

    fn asker() -> Asker {
        Asker {
            id: "lab.ask".to_owned(),
            name: "Ask".to_owned(),
        }
    }

    #[test]
    fn a_fake_answers_what_it_was_given_then_the_same_way_every_time() {
        let ui = FakeUi::new().answer(json!("given")).answer(Value::Null);
        let text = Prompt {
            kind: PromptKind::Text,
            title: "Name?".to_owned(),
            default: Some("untitled".to_owned()),
            ..Prompt::default()
        };
        assert_eq!(
            block_on(ui.prompt(&asker(), text.clone())),
            Ok(Some(json!("given")))
        );
        assert_eq!(
            block_on(ui.prompt(&asker(), text.clone())),
            Ok(None),
            "cancelled"
        );
        assert_eq!(
            block_on(ui.prompt(&asker(), text)),
            Ok(Some(json!("untitled")))
        );
        let one = Prompt {
            kind: PromptKind::PickOne,
            choices: vec![Choice {
                id: "b".to_owned(),
                label: "B".to_owned(),
                detail: None,
            }],
            ..Prompt::default()
        };
        assert_eq!(block_on(ui.prompt(&asker(), one)), Ok(Some(json!("b"))));
        let menu = Menu {
            items: vec![
                MenuItem {
                    id: "x".to_owned(),
                    disabled: true,
                    ..MenuItem::default()
                },
                MenuItem {
                    separator: true,
                    ..MenuItem::default()
                },
                MenuItem {
                    id: "y".to_owned(),
                    ..MenuItem::default()
                },
            ],
            ..Menu::default()
        };
        assert_eq!(block_on(ui.menu(&asker(), menu)), Ok(Some("y".to_owned())));
        assert_eq!(ui.asked().len(), 5);
        assert_eq!(pick_answer(PickKind::Open, None), Value::Null);
        assert_eq!(
            pick_answer(PickKind::OpenMany, Some(vec!["a".to_owned()])),
            json!(["a"])
        );
    }

    #[test]
    fn decorations_are_one_an_extension_a_target_and_go_with_it() {
        let d = Decorations::default();
        let row = Target::file("notes/a.md");
        let badge = |b: &str| Decoration {
            badge: Some(b.to_owned()),
            ..Decoration::default()
        };
        d.set("a", &row, Some(badge("1")));
        d.set("a", &row, Some(badge("2")));
        d.set("b", &row, Some(badge("x")));
        let on: Vec<_> = d.of(&row).into_iter().map(|x| x.from).collect();
        assert_eq!(on, ["a", "b"]);
        assert_eq!(d.of(&row)[0].decoration.badge.as_deref(), Some("2"));
        let before = d.revision();
        d.withdraw("a");
        assert!(d.revision() > before);
        d.set("b", &row, None);
        assert!(d.of(&row).is_empty());
        assert!(d.all().is_empty());
    }
}
