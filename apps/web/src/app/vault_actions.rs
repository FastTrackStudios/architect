//! The vault's actions in the browser (#495 H3): `VAULT_*`, what linked
//! extensions (Projects, Tasks, the Calendar…) read and change the vault's
//! notes with. `BASES_*` come from the vault query service (the Bases
//! extension's workstream, #272), not from here.
//!
//! - **Signed in**, each is the node's (`VaultActions` on the org lane,
//!   `apps/desktop/src/node/vault_actions.rs`): the same actions the
//!   desktop app runs, on the vault's folder there. The browser keeps no
//!   copy of the engine. A read-only action runs as a read, anything else
//!   as a write, which the lane's gate allows only a member who may write
//!   the vault. They act on the org's first vault, whose paths they name
//!   without its folder (`Projects/Plan.md`), as on a desk.
//! - **An extension's call** needs what its manifest asks for documents
//!   (`reads` for a read, `writes` for a change), on top of its
//!   `capabilities.actions` (#714's rule).
//! - **In the demo** there is no node: the vault's engine reads a folder
//!   and does not build for a browser. Each action answers that the demo
//!   has no vault engine and to sign in, which the extensions' views show.

use std::sync::Arc;

use architect::action::{
    ActionCaller, ActionError, ActionFuture, ActionInfo, ActionList, CallerAsyncHandler, Effect,
    Principal,
};
use architect_extensions::capabilities::DOCUMENTS;
use architect_extensions::host::Host;
use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_channel::oneshot;

/// Why the demo answers no vault action.
pub const DEMO_NO_VAULT: &str = "Sign in to see your data here: the demo runs without a node, \
     and the vault's actions (projects, tasks, events, bases) run on the node";

/// The vault actions the demo answers with [`DEMO_NO_VAULT`]: what the
/// Architect app's linked extensions call.
const DEMO_IDS: [(&str, Effect); 16] = [
    ("VAULT_LIST", Effect::ReadOnly),
    ("VAULT_READ", Effect::ReadOnly),
    ("VAULT_SEARCH", Effect::ReadOnly),
    ("VAULT_METADATA", Effect::ReadOnly),
    ("VAULT_RELOAD", Effect::ReadOnly),
    ("VAULT_CREATE", Effect::Mutating),
    ("VAULT_APPEND", Effect::Mutating),
    ("VAULT_SET_PROPERTY", Effect::Mutating),
    ("VAULT_REMOVE_PROPERTY", Effect::Mutating),
    ("VAULT_RENAME", Effect::Mutating),
    ("VAULT_DELETE", Effect::Destructive),
    ("BASES_LIST", Effect::ReadOnly),
    ("BASES_READ", Effect::ReadOnly),
    ("BASES_ROWS", Effect::ReadOnly),
    ("BASES_OPEN", Effect::ReadOnly),
    ("TAGS_LIST", Effect::ReadOnly),
];

/// One call for the node, run on the page's thread.
struct Call {
    id: String,
    args: String,
    read: bool,
    answer: oneshot::Sender<Result<String, String>>,
}

fn effect_of(name: &str) -> Effect {
    match name {
        "read_only" => Effect::ReadOnly,
        "destructive" => Effect::Destructive,
        _ => Effect::Mutating,
    }
}

fn info(
    id: &str,
    name: &str,
    description: &str,
    group: &str,
    effect: Effect,
    params: Option<String>,
) -> ActionInfo {
    ActionInfo {
        id: id.to_owned(),
        display_name: name.to_owned(),
        description: description.to_owned(),
        category: "Vault".to_owned(),
        group: group.to_owned(),
        shortcut: String::new(),
        effect,
        params_schema: params,
        result_schema: None,
        enabled: true,
        disabled_reason: None,
        checked: None,
    }
}

/// Whether `caller` may run an action of `effect` on the vault: an
/// extension needs `reads` (a read) or `writes` (a change) of documents;
/// anyone else is decided by the list as ever.
fn may(host: &Host, caller: &ActionCaller, id: &str, effect: Effect) -> Result<(), ActionError> {
    let Some(Principal::Extension { id: ext, .. }) = caller.principal() else {
        return Ok(());
    };
    let caps = host.capabilities(ext).unwrap_or_default();
    let (list, word) = if effect == Effect::ReadOnly {
        (&caps.reads, "reads")
    } else {
        (&caps.writes, "writes")
    };
    if list.iter().any(|c| c == DOCUMENTS) {
        return Ok(());
    }
    Err(ActionError::Denied {
        id: id.to_owned(),
        reason: format!("{ext} does not ask for {word} = [\"documents\"], which {id} needs"),
    })
}

/// The node's vault actions for `vault` on `app`, each run through `org`;
/// those `app` has already are left as they are.
pub fn install_org(app: &Arc<ActionList>, host: Host, org: super::org::OrgVaults, vault: String) {
    let (tx, mut rx) = unbounded::<Call>();
    let runner = org.clone();
    let on = vault.clone();
    wasm_bindgen_futures::spawn_local(async move {
        use futures_util::StreamExt as _;
        while let Some(call) = rx.next().await {
            let out = runner
                .run_vault_action(&on, &call.id, call.args, call.read)
                .await;
            let _ = call.answer.send(out);
        }
    });
    let app = Arc::clone(app);
    wasm_bindgen_futures::spawn_local(async move {
        let infos = match org.vault_actions(&vault).await {
            Ok(infos) => infos,
            Err(e) => {
                org.report(format!("The vault's actions are not here: {e}"));
                return;
            }
        };
        for a in infos {
            if app.info(&a.id).is_some() {
                continue;
            }
            let effect = effect_of(&a.effect);
            let meta = info(
                &a.id,
                &a.name,
                &a.description,
                &a.group,
                effect,
                a.params.clone(),
            );
            let (tx, host, id) = (tx.clone(), host.clone(), a.id.clone());
            let handler: CallerAsyncHandler =
                Arc::new(move |caller: &ActionCaller, args: &[u8]| {
                    may(&host, caller, &id, effect)?;
                    let (answer, heard) = oneshot::channel();
                    tx.unbounded_send(Call {
                        id: id.clone(),
                        args: if args.is_empty() {
                            "{}".to_owned()
                        } else {
                            String::from_utf8_lossy(args).into_owned()
                        },
                        read: effect == Effect::ReadOnly,
                        answer,
                    })
                    .map_err(|_| failed(&id, "the org's vaults are gone"))?;
                    let id = id.clone();
                    let run: ActionFuture = Box::pin(async move {
                        match heard.await {
                            Ok(Ok(out)) => Ok(out.into_bytes()),
                            Ok(Err(e)) => Err(failed(&id, &e)),
                            Err(_) => Err(failed(&id, "the org's vaults are gone")),
                        }
                    });
                    Ok(run)
                });
            let _ = app.register_forwarded_with(&meta, handler);
        }
    });
}

/// The demo's answer to each vault action: sign in ([`DEMO_NO_VAULT`]).
pub fn install_demo(app: &ActionList) {
    for (id, effect) in DEMO_IDS {
        if app.info(id).is_some() {
            continue;
        }
        // Any arguments: each answers the same.
        let meta = info(
            id,
            id,
            DEMO_NO_VAULT,
            "Vault",
            effect,
            Some(r#"{"type":"object","additionalProperties":true}"#.to_owned()),
        );
        let handler: CallerAsyncHandler = Arc::new(move |_: &ActionCaller, _: &[u8]| {
            let run: ActionFuture = Box::pin(async move { Err(failed(id, DEMO_NO_VAULT)) });
            Ok(run)
        });
        let _ = app.register_forwarded_with(&meta, handler);
    }
}

fn failed(id: &str, message: &str) -> ActionError {
    ActionError::Failed {
        id: id.to_owned(),
        message: message.to_owned(),
    }
}

/// Opens a vault path an extension names (`HostRequest::Open`): `root`
/// (the org's vault folder, none in the demo) and it, in the workspace.
pub fn open_paths_in(
    host: &Host,
    root: Option<String>,
) -> futures_channel::mpsc::UnboundedReceiver<String> {
    let (tx, rx): (UnboundedSender<String>, _) = unbounded();
    host.set_path_opener(move |path| {
        let full = root
            .as_ref()
            .map_or_else(|| path.to_owned(), |r| format!("{r}/{path}"));
        tx.unbounded_send(full)
            .map_err(|_| "the window is gone".to_owned())?;
        Ok(serde_json::json!({ "opened": path }))
    });
    rx
}
