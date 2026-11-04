//! Extensions' actions on the app's own list too
//! ([`Host::forward_actions_to`]).
//!
//! A host puts each extension's actions on its list (the extensions'
//! own: ⌘K reads it among the app's lists). An app whose control socket,
//! CLI and MCP serve another list — its own — forwards them there, as
//! they come and go: `actions run LAB_WASI_COUNTER_ADD` on the app's list
//! runs the extension's action on the host's, as whoever ran it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionCaller, ActionFuture, ActionList, CallerAsyncHandler};

use crate::host::{Event, Host};

/// The ids one forwarding put on its list.
#[derive(Clone)]
struct Forwarded {
    from: ActionList,
    to: ActionList,
    ours: Arc<Mutex<BTreeSet<String>>>,
}

impl Forwarded {
    fn put(&self, id: &str) {
        let mut ours = self.ours.lock().unwrap_or_else(PoisonError::into_inner);
        if ours.contains(id) || self.to.info(id).is_some() {
            // Ours already, or the app's own: left alone.
            return;
        }
        let Some(info) = self.from.info(id) else {
            return;
        };
        let (from, action) = (self.from.clone(), id.to_owned());
        let handler: CallerAsyncHandler = Arc::new(move |caller: &ActionCaller, args: &[u8]| {
            let (from, caller, action, args) =
                (from.clone(), caller.clone(), action.clone(), args.to_vec());
            let run: ActionFuture =
                Box::pin(async move { from.run_json_async_with(&caller, &action, &args).await });
            Ok(run)
        });
        if self.to.register_forwarded_with(&info, handler).is_ok() {
            ours.insert(id.to_owned());
        }
    }

    fn take(&self, id: &str) {
        let mut ours = self.ours.lock().unwrap_or_else(PoisonError::into_inner);
        if ours.remove(id) {
            self.to.unregister(id);
        }
    }
}

impl Host {
    /// Puts every loaded extension's actions on `to` as well (an app's
    /// own list: what its control socket, CLI and MCP serve), now and as
    /// extensions load, reload and unload. Each runs on this host's list
    /// as whoever ran it on `to` (`to`'s authorizer and confirmation
    /// first). An id `to` already has of its own is left alone.
    pub fn forward_actions_to(&self, to: &ActionList) {
        let forwarded = Forwarded {
            from: self.list().clone(),
            to: to.clone(),
            ours: Arc::default(),
        };
        // Which extension put which id: what an unload takes away.
        let by_extension: Arc<Mutex<std::collections::BTreeMap<String, Vec<String>>>> =
            Arc::default();
        {
            let (forwarded, by_extension) = (forwarded.clone(), Arc::clone(&by_extension));
            self.on_event(move |event| {
                let mut by = by_extension
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                match event {
                    Event::Loaded(change) => {
                        for id in &change.removed {
                            forwarded.take(id);
                        }
                        let ids: Vec<String> =
                            change.added.iter().chain(&change.kept).cloned().collect();
                        for id in &ids {
                            forwarded.put(id);
                        }
                        by.insert(change.id.clone(), ids);
                    }
                    Event::Unloaded { id } => {
                        for action in by.remove(id).unwrap_or_default() {
                            forwarded.take(&action);
                        }
                    }
                    Event::Failed { .. } => {}
                }
            });
        }
        // What is loaded already.
        let mut by = by_extension
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for loaded in self.loaded() {
            for id in &loaded.actions {
                forwarded.put(id);
            }
            by.entry(loaded.manifest.id.clone())
                .or_insert_with(|| loaded.actions.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use architect::action::{ActionCaller, ActionList};
    use futures::executor::block_on;

    use crate::catalog::Origin;
    use crate::host::{Host, SharedSections};
    use crate::package::Package;

    const MANIFEST: &str = r#"
id = "test.forwarded"
name = "Forwarded"
version = "0.1.0"
kind = "luau"
api = "0.1"

[[actions]]
id = "TEST_FORWARDED_ADD"
name = "Add"
params = '{"type":"object","properties":{"n":{"type":"integer"}}}'
script = "add.luau"
"#;

    fn run(list: &ActionList, id: &str, args: &str) -> Result<Vec<u8>, architect::action::ActionError> {
        block_on(list.run_json_async_with(&ActionCaller::app(), id, args.as_bytes()))
    }

    #[test]
    fn an_extensions_actions_run_on_the_apps_list_while_it_is_loaded() {
        let app = ActionList::new();
        let host = Host::new(ActionList::new(), SharedSections::default());
        let package = |body: &'static str| {
            Package::embedded(MANIFEST, &[("add.luau", body)]).unwrap()
        };
        // Loaded before the forwarding, and after.
        host.load(&package("return args.n + 1"), Origin::Builtin)
            .unwrap();
        host.forward_actions_to(&app);
        assert_eq!(run(&app, "TEST_FORWARDED_ADD", r#"{"n":1}"#).unwrap(), b"2");
        // A reload keeps it there, running the new copy.
        host.load(&package("return args.n + 10"), Origin::Builtin)
            .unwrap();
        assert_eq!(run(&app, "TEST_FORWARDED_ADD", r#"{"n":1}"#).unwrap(), b"11");
        // Unloaded, it is gone from both.
        assert!(host.unload("test.forwarded"));
        assert!(app.info("TEST_FORWARDED_ADD").is_none());
        assert!(run(&app, "TEST_FORWARDED_ADD", "{}").is_err());
        // Loaded again, back.
        host.load(&package("return args.n + 2"), Origin::Builtin)
            .unwrap();
        assert_eq!(run(&app, "TEST_FORWARDED_ADD", r#"{"n":1}"#).unwrap(), b"3");
    }

    static OWN: architect::action::ActionMeta = architect::action::ActionMeta {
        id: "TEST_FORWARDED_ADD",
        trait_name: "",
        method_name: "TEST_FORWARDED_ADD",
        display_name: "The app's own",
        description: "The app's own.",
        category: "Test",
        group: "Test",
        shortcut: "",
        toggleable: false,
        undo: false,
    };

    #[test]
    fn an_id_the_app_has_of_its_own_is_left_alone() {
        let app = ActionList::new();
        app.register_with(
            &OWN,
            architect::action::Effect::ReadOnly,
            |_: &ActionCaller, (): ()| Ok::<_, String>("the app's".to_owned()),
        )
        .unwrap();
        let host = Host::new(ActionList::new(), SharedSections::default());
        host.forward_actions_to(&app);
        host.load(
            &Package::embedded(MANIFEST, &[("add.luau", "return 1")]).unwrap(),
            Origin::Builtin,
        )
        .unwrap();
        assert_eq!(run(&app, "TEST_FORWARDED_ADD", "null").unwrap(), b"\"the app's\"");
        assert!(host.unload("test.forwarded"));
        assert!(app.info("TEST_FORWARDED_ADD").is_some(), "still the app's");
    }
}
