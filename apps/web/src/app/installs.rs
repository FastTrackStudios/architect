//! Installed extensions in the web app (#495): what the showcase vault's
//! **Install all** installs, kept in this browser, run by the same host.
//!
//! - **Where installs are kept**: the host's install path
//!   (`architect_extensions::install::Installs`) and its grants
//!   (`architect_extensions::consent::Grants`) on a
//!   `architect_extensions::disk::MemDisk`, each change kept in this
//!   browser's `localStorage` ([`disk`]); read back when the page loads,
//!   so what was installed and granted runs again with no prompt. A
//!   private window or cleared site data starts empty.
//! - **What is offered**: the registry the distribution carries
//!   (`architect_distribution::showcase`), the desk's and the phone's too:
//!   no network.
//! - **What a vault needs**: its `.architect/extensions.toml`, over the
//!   vault, with the same component as on a desk
//!   (`architect_extensions::page::MissingExtensions`: one consent prompt,
//!   what cannot run here and why) ([`VaultNeeds`]).
//! - **Their pages**: a view with a TSX page runs in a Web Worker on the
//!   browser's engine (`architect_pages::web::WebPages`), drawn in the
//!   app's document under its pane, every host call the app's host
//!   service for the extension (`WebHost::through`) ([`view_pages`]).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use architect::action::ActionList;
use architect_extensions::browse::Browse;
use architect_extensions::disk::MemDisk;
use architect_extensions::extensions::Extensions;
use architect_extensions::interface::{PageAsk, ViewPages};
use architect_extensions::project::Requirements;
use architect_extensions::web::WebHost;
use architect_pages::web::{PageFolder, WebPage, WebPages};
use dioxus::prelude::*;

/// Where the host keeps installs, and its grants, on the [`disk`].
pub const INSTALLS: &str = "/architect/extensions";
pub const GRANTS: &str = "/architect/grants.json";

/// The vault's needs file, as on a desk (`vault_needs::NEEDS`).
const NEEDS: &str = ".architect/extensions.toml";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn key(scope: &str) -> String {
    format!("architect.extensions.{scope}")
}

/// The installs' files kept under `scope` in this browser: each path's
/// bytes as base64, in one JSON object.
fn load(scope: &str) -> BTreeMap<String, Vec<u8>> {
    use base64::Engine as _;
    let Some(text) = storage().and_then(|s| s.get_item(&key(scope)).ok().flatten()) else {
        return BTreeMap::new();
    };
    let kept: BTreeMap<String, String> = serde_json::from_str(&text).unwrap_or_default();
    kept.into_iter()
        .filter_map(|(path, b64)| {
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .ok()
                .map(|bytes| (path, bytes))
        })
        .collect()
}

fn keep(scope: &str, files: &BTreeMap<String, Vec<u8>>) {
    use base64::Engine as _;
    let Some(store) = storage() else { return };
    let kept: BTreeMap<&String, String> = files
        .iter()
        .map(|(p, b)| (p, base64::engine::general_purpose::STANDARD.encode(b)))
        .collect();
    let Ok(text) = serde_json::to_string(&kept) else {
        return;
    };
    if store.set_item(&key(scope), &text).is_err() {
        web_sys::console::warn_1(
            &"extensions: this browser's storage refused the installs (full, or private)".into(),
        );
    }
}

/// Where installs and grants are kept: under `scope` in this browser's
/// storage (an org's slug, the showcase's), or for this tab only (`None`:
/// the demo, which keeps nothing past the tab, as it says).
#[must_use]
pub fn disk(scope: Option<String>) -> MemDisk {
    match scope {
        Some(scope) => {
            MemDisk::with_files(load(&scope)).on_change(move |files| keep(&scope, files))
        }
        None => MemDisk::new(),
    }
}

/// What the page-view host needs: the app's pages, the extensions, and
/// the host's list.
#[derive(Clone)]
pub struct PageHost {
    pub pages: WebPages,
    pub extensions: Extensions,
    pub list: Arc<ActionList>,
}

/// [`ViewPages`] for the app: a view whose manifest names a page, of an
/// extension whose files are text here (installed in this browser, or
/// linked in), drawn by [`WebPageView`].
#[must_use]
pub fn view_pages() -> ViewPages {
    ViewPages(Rc::new(|ask: &PageAsk| {
        let page = ask.page.clone()?;
        let extension = ask.extension.clone();
        let view = ask.view.clone();
        Some(rsx! {
            WebPageView { key: "{extension}/{view}", extension, page }
        })
    }))
}

/// Extension `extension`'s page `page`, in a Web Worker, drawn under this
/// element; closed when it goes.
#[component]
fn WebPageView(extension: String, page: String) -> Element {
    let host = use_context::<PageHost>();
    let id = use_hook(|| {
        thread_local! { static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
        format!(
            "ext-page-{}",
            NEXT.with(|n| {
                n.set(n.get() + 1);
                n.get()
            })
        )
    });
    let open = use_hook(|| Rc::new(RefCell::new(None::<WebPage>)));
    let mut problem = use_signal(|| None::<String>);
    {
        let (id, open, extension, page) = (
            id.clone(),
            Rc::clone(&open),
            extension.clone(),
            page.clone(),
        );
        use_effect(move || {
            if open.borrow().is_some() {
                return;
            }
            let Some(into) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id(&id))
            else {
                return;
            };
            let Some(chosen) = host
                .extensions
                .chosen()
                .into_iter()
                .find(|c| c.runs.id == extension)
            else {
                problem.set(Some(format!("{extension} is not here")));
                return;
            };
            let package = chosen.runs.package;
            let Some(files) = package.texts() else {
                problem.set(Some(format!(
                    "{extension}'s files cannot be read in a browser"
                )));
                return;
            };
            let folder = PageFolder {
                entry: page.clone(),
                files: files.into_iter().collect(),
            };
            let web = WebHost::new(package.manifest.clone(), (*host.list).clone())
                .through(host.extensions.host().api(&extension));
            *open.borrow_mut() = Some(host.pages.open(&folder, web, &into));
        });
    }
    rsx! {
        if let Some(why) = problem() {
            div { style: "padding: 24px; font-size: 13px; color: var(--muted-foreground);", "{why}" }
        }
        div {
            id: "{id}",
            "data-extension-page": "{extension}",
            style: "flex: 1; min-height: 0; overflow: auto; display: flex; flex-direction: column;",
        }
    }
}

/// The open vault's needs file: the source's copy, else (the built-in demo
/// and showcase vaults, whose source keeps only notes as text) the
/// build's.
fn needs_text(vault: &super::vault::WebVault) -> Option<String> {
    use architect_navigator::NavigatorSource as _;
    vault.source.read_text(NEEDS).or_else(|| {
        vault.org.is_none().then(|| {
            crate::demo::files()
                .into_iter()
                .find(|f| f.path == NEEDS)
                .and_then(|f| std::str::from_utf8(f.bytes).ok().map(str::to_owned))
        })?
    })
}

/// What the open vault needs (its `.architect/extensions.toml`), with
/// **Install all** through one consent prompt and what cannot run here,
/// each with why: the desk's banner, over the web app's vault. Nothing
/// when it needs nothing it lacks, or after "Not now".
#[component]
pub fn VaultNeeds() -> Element {
    let browse = try_use_context::<Browse>();
    let vault = use_context::<super::vault::WebVault>();
    let mut round = use_signal(|| 0_u32);
    let mut dismissed = use_signal(|| false);
    let requirements = use_hook(|| needs_text(&vault).map(|text| Requirements::from_toml(&text)));
    let Some(browse) = browse else {
        return rsx! {};
    };
    let requirements = match requirements {
        None => return rsx! {},
        Some(Err(why)) => {
            return rsx! {
                div { style: "padding: 8px 12px 0; font-size: 12px; color: var(--destructive);", "{NEEDS}: {why}" }
            };
        }
        Some(Ok(r)) => r,
    };
    let _ = round();
    let plan = browse.extensions().open(&requirements);
    if dismissed() || (plan.install.is_empty() && plan.offer.is_empty() && plan.missing.is_empty())
    {
        return rsx! {};
    }
    rsx! {
        document::Style { {architect_ui::UTILITIES_CSS} }
        div { style: "padding: 8px 12px 0; display: flex; flex-direction: column; gap: 6px; flex-shrink: 0; max-height: 440px; overflow-y: auto;",
            architect_extensions::page::MissingExtensions {
                browse: browse.clone(),
                plan,
                what: "vault".to_owned(),
                // It says what the install did; the plan is made again.
                on_done: move |_: String| round += 1,
                on_dismiss: move |()| dismissed.set(true),
            }
        }
    }
}
