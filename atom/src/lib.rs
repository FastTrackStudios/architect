//! `architect-atom` — optimistic, stale-while-revalidate client state for
//! Dioxus.
//!
//! This is the **client-side twin** of architect's server-side
//! Layer/Resource/Scope DI. architect borrows
//! [Effect](https://effect.website/)'s ideas *without the
//! `Effect<A,E,R>` monad* (see `docs/.../idioms.md` §6); `architect-atom`
//! does the same on the client, borrowing
//! [effect-atom](https://github.com/tim-smart/effect-atom)'s `Result` +
//! writable optimistic atoms — expressed as plain Dioxus `Signal`s.
//!
//! The crate is entity-agnostic and wasm-safe: its only dependency is
//! Dioxus core. Implement [`StoreEntity`] for your own types; nothing here
//! knows about vox, tokio, or any concrete entity.
//!
//! # The three primitives
//!
//! - [`AtomResult`] — async state that **keeps the previous value across a
//!   refetch** (stale-while-revalidate), with flat `Error` / `Defect`
//!   phases. Render a page by `match`ing it (`use architect::AtomResult::*`
//!   for unqualified arms) — the idiomatic, exhaustive equivalent of
//!   effect's `onWaiting/onError/onDefect/onSuccess`.
//! - [`use_async`] / [`Async`] — a single refreshable async value (the
//!   async atom + `useAtomRefresh`), folded into an [`AtomResult`].
//! - [`Store`] + [`Id`] + [`use_mutation`] / [`Mutation`] — a keyed
//!   optimistic entity cache (the `Atom.family` + writable optimistic
//!   atoms): mutations patch it instantly and either reconcile against the
//!   server's value or roll back on failure.
//!
//! ```ignore
//! use architect_atom::{use_store, use_mutation, Id, Store};
//!
//! let store: Store<Example> = use_context();           // provided at the app root
//! let create = use_mutation::<String>();
//!
//! // optimistic create: a temp row appears instantly, swapped for the
//! // server's row on success, removed on failure.
//! create.run(
//!     store,
//!     |s| s.insert_optimistic(Example::draft(name.clone(), desc.clone())),
//!     move || async move {
//!         clients.repo.create(ExampleCreate { name, description: desc })
//!             .await.map(Some).map_err(|e| format!("create: {e:?}"))
//!     },
//! );
//! ```

mod actions;
mod connection;
mod id;
mod mutation;
mod notify;
mod reactivity;
mod resource;
mod result;
mod store;

pub use actions::{
    ActionInvoker, ActionLike, group_actions, use_action_invoker, use_actions_grouped,
};
pub use connection::{
    Connection, ConnectionState, use_connect, use_connect_reactive, use_connection,
    use_connection_root,
};
pub use id::Id;
pub use mutation::{Mutation, use_mutation};
pub use notify::{
    Notice, NoticeLevel, Notifications, provide_notifications, try_use_notifications,
    use_notifications,
};
pub use reactivity::{Reactivity, provide_reactivity, try_use_reactivity, use_reactivity};
pub use resource::{Async, use_async};
pub use result::AtomResult;
pub use store::{
    MutationId, Store, StoreData, StoreEntity, use_store, use_store_entry, use_store_list,
};

// Re-exported so `#[derive(architect::Entity)]`-emitted client hooks can
// name dioxus items (`use_context`, …) through `::architect::dioxus::…`
// without the consumer crate adding its own dioxus dependency line.
pub use dioxus;
