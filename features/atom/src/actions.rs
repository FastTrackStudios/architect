//! Reactive listing + invocation for `architect::action`-style command
//! registries (REAPER named commands, CLI actions, menu items).
//!
//! Entity-agnostic like the rest of this crate: [`ActionLike`] is a trait,
//! not a concrete struct, so `architect-atom` doesn't depend on
//! `architect` (or any host's action-metadata type) directly. Implement
//! it for your own type — e.g. a downstream crate implements it for
//! `architect::action::ActionMeta` — the same shape as [`StoreEntity`]
//! (`crate::StoreEntity`).
//!
//! `architect::action::ActionMeta`'s `id`/`display_name`/`category`/
//! `group` fields are all `&'static str`, so this trait's methods return
//! borrowed `&str` rather than owned `String` — cheap for the common
//! case (a `'static` metadata const) without forcing an allocation.

use std::collections::BTreeMap;

use dioxus::prelude::*;

/// Minimal view of one action a menu/CLI/palette can list and invoke.
pub trait ActionLike: Clone + PartialEq + 'static {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn category(&self) -> &str;
    fn group(&self) -> &str;
}

/// Group `actions` by category, then by group within each category —
/// the same shape a CLI/menu builder needs (see
/// `architect-action-derive`'s own `clap::Command` construction). Pure
/// and unit-testable without a Dioxus runtime, per this crate's
/// `Store`/`StoreData` convention — [`use_actions_grouped`] is the thin
/// reactive wrapper.
pub fn group_actions<A: ActionLike>(actions: &[A]) -> BTreeMap<String, BTreeMap<String, Vec<A>>> {
    let mut tree: BTreeMap<String, BTreeMap<String, Vec<A>>> = BTreeMap::new();
    for action in actions {
        tree.entry(action.category().to_string())
            .or_default()
            .entry(action.group().to_string())
            .or_default()
            .push(action.clone());
    }
    tree
}

/// Reactive, category-then-group grouped view of a `'static` action
/// list (e.g. the macro-emitted `<Trait>Actions::all()`). A `Memo` over
/// a fixed input, not a fetch-backed [`crate::Store`] — actions are
/// declared at compile time and don't change at runtime, so there's no
/// async phase to track.
pub fn use_actions_grouped<A: ActionLike>(
    all: &'static [A],
) -> Memo<BTreeMap<String, BTreeMap<String, Vec<A>>>> {
    use_memo(move || group_actions(all))
}

/// Wraps a plain invoke callback with a reactive "which action id was
/// last invoked" signal, for lightweight UI feedback (e.g. flashing a
/// checkmark on the clicked menu item) without pulling in the full
/// [`crate::Async`]/[`crate::Mutation`] machinery.
///
/// v1 actions (`fn name(&self)`, no return value) are synchronous
/// fire-and-forget by design — there's no request/response or
/// rollback state to manage, so `Mutation` (built for exactly that)
/// would be the wrong tool here. For richer feedback (toasts, errors),
/// compose [`crate::Notifications`] directly inside `invoke` instead
/// of adding a second status-tracking primitive to this module.
#[derive(Clone, Copy)]
pub struct ActionInvoker<A: ActionLike> {
    last_invoked_id: Signal<Option<String>>,
    _marker: std::marker::PhantomData<A>,
}

impl<A: ActionLike> ActionInvoker<A> {
    /// The id of the most recently invoked action, if any. Doesn't
    /// clear itself — components that want a transient flash should
    /// pair this with their own timer (e.g. clear after N ms), the
    /// same way a caller would debounce any other signal.
    pub fn last_invoked_id(&self) -> Option<String> {
        self.last_invoked_id.read().clone()
    }
}

/// Build an [`ActionInvoker`] around `invoke`, which actually runs the
/// action (e.g. a `TracksClient`/CLI dispatch call, or a direct
/// in-process `Arc<dyn Trait>` method call).
pub fn use_action_invoker<A: ActionLike>(
    invoke: impl Fn(&A) + 'static,
) -> (ActionInvoker<A>, Callback<A>) {
    let mut last_invoked_id = use_signal(|| None);
    let invoker = ActionInvoker {
        last_invoked_id,
        _marker: std::marker::PhantomData,
    };
    let run = use_callback(move |action: A| {
        invoke(&action);
        last_invoked_id.set(Some(action.id().to_string()));
    });
    (invoker, run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Debug)]
    struct FakeAction {
        id: &'static str,
        display_name: &'static str,
        category: &'static str,
        group: &'static str,
    }

    impl ActionLike for FakeAction {
        fn id(&self) -> &str {
            self.id
        }
        fn display_name(&self) -> &str {
            self.display_name
        }
        fn category(&self) -> &str {
            self.category
        }
        fn group(&self) -> &str {
            self.group
        }
    }

    #[test]
    fn groups_by_category_then_group() {
        let actions = vec![
            FakeAction {
                id: "SESSION_BUILD_SETLIST",
                display_name: "Build Setlist",
                category: "Setlist",
                group: "Build",
            },
            FakeAction {
                id: "SESSION_LOAD_DEMO_SETLIST",
                display_name: "Load Demo Setlist",
                category: "Setlist",
                group: "Demo",
            },
            FakeAction {
                id: "SESSION_DUMP_RULER_STATE",
                display_name: "Dump Ruler State",
                category: "Debug",
                group: "",
            },
        ];

        let tree = group_actions(&actions);

        assert_eq!(tree.len(), 2, "two categories: Setlist, Debug");
        let setlist = &tree["Setlist"];
        assert_eq!(setlist.len(), 2, "two groups within Setlist: Build, Demo");
        assert_eq!(setlist["Build"][0].id, "SESSION_BUILD_SETLIST");
        assert_eq!(setlist["Demo"][0].id, "SESSION_LOAD_DEMO_SETLIST");
        assert_eq!(tree["Debug"][""][0].id, "SESSION_DUMP_RULER_STATE");
    }

    #[test]
    fn empty_input_produces_empty_tree() {
        let actions: Vec<FakeAction> = vec![];
        assert!(group_actions(&actions).is_empty());
    }
}
