//! The app's home for a signed-in member, filling the page before a note
//! is open: the linked view the distribution names
//! (`architect_distribution::Distribution::home`), else a view an
//! extension's island declares (`SiteIsland::home`).
//!
//! It is an extension's view like any other. A linked one is the
//! distribution's own code in the app's document, and reads what it is
//! handed with `linked::use_embed` (its `props`); it shows in the demo
//! too, on the demo's vault. An island's is drawn in the frame host from
//! the island's web bundle, handed data and asking back. What it is
//! handed is the member's vaults as rows, one a note that has a `type`:
//! its path and its properties, never its text (`home` in its `note`
//! prop: `{person, orgs: [{slug, rows: [{path, open, type,
//! properties}]}], recent: [path]}`; `path` is the note's own in its
//! vault, `open` what the app opens it by). What it may ask is what any view may: the
//! playhead's asks, and `NOTE_OPEN {path}`, which opens the note in the
//! workspace as a press in the navigator would. A share link's guest has
//! no home: there is no vault to list.

use std::rc::Rc;

use architect_navigator::NavigatorSource as _;
use architect_navigator::source::PropertyValue;
use dioxus::prelude::*;
use serde_json::{Value, json};

use super::vault::{VaultWorkspace, WebVault};

/// Whether the home shows (over the workspace, which stays as it was).
pub static HOME: GlobalSignal<bool> = Signal::global(|| true);

/// The most rows handed to the view, and the most notes remembered as
/// recently opened.
const MAX_ROWS: usize = 4000;
const MAX_RECENT: usize = 12;

thread_local! {
    /// The notes opened from the home, newest first, for this page's life.
    static RECENT: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Note `path` was opened from a view: the newest of the notes the home
/// is told were opened recently.
pub fn remember(path: &str) {
    RECENT.with(|r| {
        let mut r = r.borrow_mut();
        r.retain(|p| p != path);
        r.insert(0, path.to_owned());
        r.truncate(MAX_RECENT);
    });
}

fn value(p: &PropertyValue) -> Value {
    match p {
        PropertyValue::Text(t) => json!(t),
        PropertyValue::Number(n) => json!(n),
        PropertyValue::Bool(b) => json!(b),
        PropertyValue::List(l) => json!(l),
    }
}

/// The vault's typed notes as rows: `{path, type, properties}`.
fn rows(vault: &WebVault) -> Vec<Value> {
    vault
        .source
        .files()
        .into_iter()
        .filter_map(|f| {
            let kind = f.properties.iter().find_map(|(k, v)| match v {
                PropertyValue::Text(t) if k == "type" && !t.trim().is_empty() => {
                    Some(t.trim().to_owned())
                }
                _ => None,
            })?;
            let properties: serde_json::Map<String, Value> = f
                .properties
                .iter()
                .map(|(k, v)| (k.clone(), value(v)))
                .collect();
            // An org's notes are listed under a folder a vault: the row's
            // path is the note's own in its vault (what a link in another
            // note says), and `open` is what the app opens it by.
            let own = match (&vault.org, f.path.split_once('/')) {
                (Some(_), Some((_, rest))) => rest,
                _ => f.path.as_str(),
            };
            Some(json!({ "path": own, "open": f.path, "type": kind, "properties": properties }))
        })
        .take(MAX_ROWS)
        .collect()
}

/// What draws the home.
#[derive(Clone, PartialEq)]
pub enum Home {
    /// The distribution's own linked view: the extension's id and the
    /// view's.
    Linked { extension: String, view: String },
    /// An island's view, in a frame.
    Island(crate::ext_host::Island, String),
}

impl Home {
    /// Whether it is the distribution's own linked view.
    #[must_use]
    pub const fn is_linked(&self) -> bool {
        matches!(self, Self::Linked { .. })
    }
}

/// What draws the home: the linked view the distribution names, when its
/// extension runs here as the app's own, else the first island that
/// declares a home view.
pub fn use_home() -> Memo<Option<Home>> {
    let listing = super::frames::use_listing();
    let ui = try_use_context::<architect_extensions::interface::ExtensionUi>();
    use_memo(move || {
        let linked = super::extensions::home_decl().filter(|(extension, view)| {
            ui.is_some_and(|ui| {
                let _ = ui.revision();
                ui.host().linked_view(extension, view).is_some()
            })
        });
        if let Some((extension, view)) = linked {
            return Some(Home::Linked { extension, view });
        }
        listing.read().as_ref().and_then(|l| {
            l.iter()
                .find_map(|i| i.home.clone().map(|view| Home::Island(i.clone(), view)))
        })
    })
}

/// The home, filling the page.
#[component]
pub fn HomePage(person: String, org: String) -> Element {
    let vault = use_context::<WebVault>();
    let VaultWorkspace(workspace) = use_context::<VaultWorkspace>();
    let home = use_home();
    // The rows as the vault has them now: read again every second and a
    // half while the home shows (the org's notes arrive after the page).
    let mut data = use_signal(|| Value::Null);
    {
        let vault = vault.clone();
        use_future(move || {
            let vault = vault.clone();
            let (person, org) = (person.clone(), org.clone());
            async move {
                loop {
                    let recent = RECENT.with(|r| r.borrow().clone());
                    let next = json!({ "home": {
                        "person": person,
                        "orgs": [{ "slug": org, "rows": rows(&vault) }],
                        "recent": recent,
                    } });
                    if *data.peek() != next {
                        data.set(next);
                    }
                    futures_timer::Delay::new(std::time::Duration::from_millis(1500)).await;
                }
            }
        });
    }
    // A row pressed: the note, in the workspace, and the home out of its way.
    {
        let vault = vault.clone();
        // A frame's ask arrives outside any scope: back into this one, or
        // the signals written below find no runtime and the app stops.
        let (runtime, scope) = (
            dioxus::core::Runtime::current(),
            dioxus::core::current_scope_id(),
        );
        use_hook(move || {
            super::frames::set_note_opener(Some(Rc::new(move |path: &str| {
                // By the app's path, or by the note's own in its vault.
                let found = super::vault::note_named(&vault, path);
                let known = found.is_some();
                if let Some(path) = found.as_deref() {
                    remember(path);
                    runtime.in_scope(scope, || {
                        super::vault::show(workspace, path);
                        *HOME.write() = false;
                    });
                }
                known
            })));
        });
    }
    match home() {
        None => rsx! {},
        Some(Home::Linked { extension, view }) => rsx! {
            div { "data-testid": "home-page",
                style: "flex: 1 1 0; min-height: 0; display: flex; flex-direction: column;",
                super::embed::LinkedView {
                    key: "{extension}/{view}",
                    extension, view,
                    claim: super::embed::HOME_CLAIM.to_owned(),
                    props: data(),
                    pane: true,
                }
            }
        },
        Some(Home::Island(island, view)) => rsx! {
            div { "data-testid": "home-page",
                style: "flex: 1 1 0; min-height: 0; display: flex; flex-direction: column;",
                super::embed::Claimed { key: "{island.folder}/{view}", island, view, note: data(), pane: true }
            }
        },
    }
}
