//! The showcase's Recipe scaler (`examples/showcase/recipe-scaler`, #536,
//! #710 R7) in the browser: an extension written only in TypeScript, its
//! actions run in Web Workers (`architect_pages::web`, `WebPages::
//! register_actions`) and its page in another, the same files the desktop
//! app runs on `QuickJS`. `?page=recipes` opens its tab.
//!
//! The vault is a stand-in for the app's `VAULT_*` actions, in the tab's
//! memory: the showcase vault's two recipes and a note that is none.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use architect::action::{ActionError, ActionFuture, ActionInfo, ActionList, Effect};
use architect_extensions::Manifest;
use architect_extensions::web::WebHost;
use architect_pages::web::{PageFolder, WebPages};
use serde_json::{Value as Json, json};

/// Its manifest.
pub const MANIFEST: &str =
    include_str!("../../../../examples/showcase/recipe-scaler/extension.toml");

/// Its files, as text, by their paths in its folder.
pub const FILES: &[(&str, &str)] = &[
    (
        "actions/list.ts",
        include_str!("../../../../examples/showcase/recipe-scaler/actions/list.ts"),
    ),
    (
        "actions/scale.ts",
        include_str!("../../../../examples/showcase/recipe-scaler/actions/scale.ts"),
    ),
    (
        "actions/open.ts",
        include_str!("../../../../examples/showcase/recipe-scaler/actions/open.ts"),
    ),
    (
        "pages/scaler.tsx",
        include_str!("../../../../examples/showcase/recipe-scaler/pages/scaler.tsx"),
    ),
    (
        "pages/lib/recipe.ts",
        include_str!("../../../../examples/showcase/recipe-scaler/pages/lib/recipe.ts"),
    ),
];

/// Its page, from its folder.
#[must_use]
pub fn folder() -> PageFolder {
    PageFolder::new("pages/scaler.tsx", FILES.iter().copied())
}

type Notes = Rc<RefCell<BTreeMap<String, (Json, String)>>>;

thread_local! {
    /// The stand-in vault's notes: properties and body, by path.
    static NOTES: Notes = Rc::new(RefCell::new(BTreeMap::new()));
}

/// A note's properties and body, when there is one (`lab_note`).
#[must_use]
pub fn note(path: &str) -> Option<Json> {
    NOTES.with(|n| {
        n.borrow()
            .get(path)
            .map(|(p, b)| json!({ "properties": p, "body": b }))
    })
}

fn info(id: &str, effect: Effect) -> ActionInfo {
    ActionInfo {
        id: id.to_owned(),
        display_name: id.to_owned(),
        description: "The stand-in vault's.".to_owned(),
        category: "Vault".to_owned(),
        group: String::new(),
        shortcut: String::new(),
        effect,
        params_schema: Some(r#"{"type":"object","additionalProperties":true}"#.to_owned()),
        result_schema: None,
        enabled: true,
        disabled_reason: None,
        checked: None,
    }
}

fn answer(
    id: &'static str,
    f: fn(&Notes, &Json) -> Result<Json, String>,
) -> Arc<dyn Fn(&[u8]) -> Result<ActionFuture, ActionError> + Send + Sync> {
    Arc::new(move |args: &[u8]| {
        let args: Json = serde_json::from_slice(args).unwrap_or(Json::Null);
        let out = NOTES.with(|n| f(n, &args));
        let run: ActionFuture = Box::pin(async move {
            out.map(|v| v.to_string().into_bytes())
                .map_err(|message| ActionError::Failed {
                    id: id.to_owned(),
                    message,
                })
        });
        Ok(run)
    })
}

/// The stand-in vault on `list`, its recipes in it.
///
/// # Errors
///
/// An action id already on the list.
pub fn vault(list: &ActionList) -> Result<(), ActionError> {
    NOTES.with(|n| {
        let mut n = n.borrow_mut();
        n.insert(
            "Recipes/Pancakes.md".to_owned(),
            (
                json!({ "servings": 4, "ingredients": ["1 1/2 cup flour", "1 1/4 cup milk", "1 egg", "2 tbsp sugar", "3 tsp baking powder", "3 tbsp melted butter", "a pinch of salt"] }),
                String::new(),
            ),
        );
        n.insert(
            "Recipes/Chili.md".to_owned(),
            (
                json!({ "servings": 6, "ingredients": ["500 g minced beef", "2 cans kidney beans", "1 can chopped tomatoes", "1 onion", "2 cloves garlic", "2 tsp chili powder", "1 tsp cumin"] }),
                String::new(),
            ),
        );
        n.insert("Recipes/Notes.md".to_owned(), (json!({}), "Ideas.".to_owned()));
    });
    list.register_forwarded(
        &info("VAULT_LIST", Effect::ReadOnly),
        answer("VAULT_LIST", |n, a| {
            let folder = a.get("folder").and_then(Json::as_str).unwrap_or_default();
            Ok(Json::Array(
                n.borrow()
                    .keys()
                    .filter(|p| folder.is_empty() || p.starts_with(&format!("{folder}/")))
                    .map(|p| json!({ "path": p }))
                    .collect(),
            ))
        }),
    )?;
    list.register_forwarded(
        &info("VAULT_READ", Effect::ReadOnly),
        answer("VAULT_READ", |n, a| {
            let path = a.get("path").and_then(Json::as_str).unwrap_or_default();
            let (p, b) = n
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| format!("{path}: no such note"))?;
            let title = path
                .rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".md");
            Ok(json!({ "path": path, "title": title, "properties": p, "body": b }))
        }),
    )?;
    list.register_forwarded(
        &info("VAULT_CREATE", Effect::Mutating),
        answer("VAULT_CREATE", |n, a| {
            let path = a
                .get("path")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            if n.borrow().contains_key(&path) {
                return Err(format!("already exists: {path}"));
            }
            let properties: Json = a
                .get("properties")
                .and_then(Json::as_str)
                .and_then(|t| serde_json::from_str(t).ok())
                .unwrap_or(Json::Null);
            let body = a
                .get("body")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            n.borrow_mut().insert(path.clone(), (properties, body));
            Ok(json!({ "path": path }))
        }),
    )?;
    Ok(())
}

/// Its TypeScript actions on `list`, each run in a Web Worker as its
/// extension: the host its page opens with too.
///
/// # Errors
///
/// Its manifest does not read, or an action id is taken.
pub fn register(pages: &WebPages, list: &ActionList) -> Result<WebHost, String> {
    let manifest = Manifest::parse(MANIFEST).map_err(|e| e.to_string())?;
    let host = WebHost::new(manifest, list.clone());
    pages
        .register_actions(list, &folder(), &host)
        .map_err(|e| e.to_string())?;
    Ok(host)
}
