+++
title = "Composing the UI"
description = "How architect's Dioxus client is structured: feature crates own state + presentation, the shell owns transport + routing + composition, and pages compose by matching an AtomResult phase."
weight = 52
+++

[`pattern.md`](@/architecture/pattern.md) covers the server side — one
source struct, every backend surface. This is the client counterpart: how
the Dioxus UI is structured so features stay self-contained and the shell
just composes them.

There are exactly two layers, and a hard rule that keeps them honest:

> **Dioxus primitives for navigation; the `AtomResult` phase-match for
> state.** A page navigates with `Link` / `nav` like any Dioxus app, and
> reads data by `match`ing a phase. Nothing else.

## The two layers

| Layer | Crate | Owns |
| --- | --- | --- |
| **Feature** | `features/<f>/<f>-ui` | state (data + mutation hooks), presentation (dumb components), the data-wiring context |
| **Shell** | `examples/app/ui` (`app-ui`) | transport (where clients connect), the typed `Route`, and pages that compose features |

A feature knows nothing about routes; the shell knows nothing about vox
clients, stores, or fetches. They meet at two thin interfaces: the feature
exposes **hooks returning a phase** (`AtomResult`/`Async`) and **dumb
components**; the shell **matches the phase** and lays out the components.

## The state layer is derived

The CRUD-shaped client state comes from the entity itself. Add `store` to
the derive (next to `repo`) and the **whole layer is emitted** — gated on
the proto crate's `atom` + `vox` features, the same convention as
`server`:

```rust
#[derive(architect::Entity, facet::Facet, Clone, Debug, PartialEq)]
#[architect(table_name = "examples", repo, store)]
pub struct Example { … }
```

For `Example` this generates:

| Generated | What it is |
| --- | --- |
| `ExampleStore` + `provide_example_store` / `use_example_store` | the shared optimistic cache, typed `Store<Example, ExampleClientError>` |
| `ExampleClientError` | `ClientError<RepoError>` — the **typed** error channel |
| `use_example(id) → AtomResult<Example, ExampleClientError>` | cache-first single read (instant after a list visit, reflects optimistic edits) |
| `use_example_list() → AtomResult<Vec<(Id, Example)>, …>` | store-backed list, tracks the `"examples"` reactivity key |
| `ExampleMutations` / `use_example_mutations` | optimistic `create` / `update` / `delete` (+ rollback, notification report, key invalidation) |
| `Example::draft(&ExampleCreate)` | client-side placeholder row for optimistic inserts |
| `ExampleEvent` / `ExampleEvents` / `ExampleEvented<R>` / `use_example_events()` | (with `events`) **live data**: the server wraps its backend once and every write broadcasts; the client hook folds pushed changes into the store, so every store-rendered page updates live across clients |

The hooks read the app's **single** `Connection<vox::Caller>` from
context (provided once at the root — see below) and build their typed
clients as cheap views over the shared connection; the store reconciles
optimistic writes against the server exactly as described in
[optimistic state](@/architecture/optimistic.md).

## Anatomy of a feature crate

With the CRUD layer derived, the feature crate only holds what the derive
**can't** know:

```
features/example/example-ui/src/
  data.rs        use_examples (search-aware list), use_example_live (refreshable read)
  mutations.rs   ExampleActions — the service verb `duplicate`
  components/    dumb: ExampleCard, ExampleRow, ExampleList, SearchBar, forms/
```

### Hand-written data hooks

A custom hook is a thin binding of the same generic primitives the derive
uses — only the fetch is yours:

```rust
// search-aware list: empty query → repo list, otherwise service search
pub fn use_examples(query: Signal<String>) -> AtomResult<Vec<(Id<Uuid>, Example)>, ExampleClientError> {
    let conn = use_connection::<Caller>();           // the app's one connection
    let store = use_example_store();                 // the derived store
    let reactivity = try_use_reactivity();
    use_store_list(store, move || async move {
        if let Some(r) = reactivity { r.track(EXAMPLE_REACTIVITY_KEY); }
        let caller = ready_or_pending!(conn);        // Loading / Connect-error early-outs
        let q = query();
        // ExampleRepoClient::new(caller).list(…) or
        // ExampleServiceClient::new(caller).search(…) — typed via ClientError
    })
}
```

The reusable primitives (in `architect`, behind the `atom` feature):

- **`use_store_entry(store, id, parse, fetch)`** — cache-first read: `Success`
  straight from the store if cached, else a fallback fetch folded back in.
- **`use_store_list(store, fetch)`** — list fetch → store hydrate →
  `entries_result()` (rows as the value, fetch as the phase).
- **`use_async(loader)`** — a single refreshable resource (no store);
  `.state()` is the phase, `.refresh()` re-runs it.
- **`Store` + `use_mutation`** — the optimistic keyed cache + writes. See
  [optimistic state](@/architecture/optimistic.md).
- **`use_interval(period)`** — a ticking signal; read it in a loader to
  re-fetch on a cadence (live-ish reads until vox subscriptions land).
  Needs the `platform` feature too.

### Typed errors end-to-end

Hooks fail with `ClientError<E>` — the service's **typed** error
(`App(RepoError::NotFound)`) kept distinct from infrastructure failures
(`Connect`, `Transport { retryable, .. }`). vox's `VoxError` folds in via
`ClientError::from`, so nothing is stringified along the way and a page
can write a real arm:

```rust
Error { error: ClientError::App(RepoError::NotFound), .. } => rsx! { NotFoundCard {} },
Error { error, .. } if error.is_infrastructure() => rsx! { Reconnecting {} },
Error { error, .. } => rsx! { StatusError { message: error.to_string() } },
```

### Mutation hooks

Optimistic writes are a small `Copy` handle the shell calls and then
navigates with plain Dioxus:

```rust
let mutations = use_example_mutations();   // derived
// in a handler:  mutations.delete(uuid);  nav.push(Route::Home {});
```

Two app-wide registries (both provided at the root, both optional) hook in
automatically:

- **`Notifications`** — a mutation that rolls back *after* its page
  unmounted (the navigate-away pattern) reports the failure to the queue;
  the shell renders it once (`NotificationTray`).
- **`Reactivity`** — settled mutations invalidate their entity's key
  (`use_mutation().invalidating(&["examples"])`, wired by the derive), so
  derived server data (search results, counts) re-fetches.

### Dumb components

Pure presentation — props in, no fetch, no navigation (`ExampleCard`,
`ExampleRow`, the forms). They're testable in isolation (the SSR component
tests render them directly) and reusable.

## The shell: pages that match a phase

`AtomResult` is a flat enum, so a page does
`use architect::AtomResult::*` and `match`es the phase. The universal
non-success arms are shared, feature-agnostic components (`Spinner`,
`StatusError`); only the success arm is feature-specific:

```rust
use architect::AtomResult::{Defect, Error, Initial, Loading, Reloading, Success};

#[component]
pub fn ExampleDetail(id: String) -> Element {
    rsx! {
        match use_example(id) {
            Initial | Loading                     => rsx! { Spinner {} },
            Success(example) | Reloading(example)  => rsx! { DetailContent { example } },
            // typed: this arm is impossible with stringified errors
            Error { error: ClientError::App(RepoError::NotFound), .. } => rsx! { NotFoundCard {} },
            Error  { error,  .. }                 => rsx! { StatusError { message: error.to_string() } },
            Defect { defect, .. }                 => rsx! { StatusError { message: format!("defect: {defect}") } },
        }
        Link { class: "back", to: Route::Home {}, "← back" }
    }
}

// success content: dumb component + normal-Dioxus navigation + feature mutations
#[component]
fn DetailContent(example: Example) -> Element {
    let nav = use_navigator();
    let mutations = use_example_mutations();
    let uuid = example.id;
    rsx! {
        ExampleCard { example }
        div { class: "actions",
            Link { class: "btn", to: Route::EditExample { id: uuid.to_string() }, "Edit" }
            button {
                onclick: move |_| { mutations.delete(uuid); nav.push(Route::Home {}); },
                "Delete"
            }
        }
    }
}
```

Note what *isn't* there: no `on_edit`/`on_done` event-handler props, no
fetch logic, no store access in the page itself — navigation is plain
`Link`/`nav`, and the data is one `match`.

The shell also owns:

- **Transport** (`transport.rs`) — `Transport::Remote(url)` vs
  `Transport::Local(server)`, with one connect function per typed client
  (wrapped in [`schedule::retry`](@/architecture/scheduling.md)).
- **Root provision** — two lines plus one per feature. `use_app`
  provides the registries (notifications + reactivity) and establishes
  the app's **single** connection; every feature's typed clients are
  `Client::new(caller)` views over the shared `vox::Caller`, so adding a
  feature adds zero connections. The derived `provide_<entity>()` bundles
  the store and its live event subscription:

  ```rust
  let t = transport.clone();
  use_app(move || async move { transport::connect(&t).await });  // Connection<Caller>
  provide_example();                                             // store + live events
  ```
- **The route table** (`Route`) and the `pages/` that map to it, plus the
  `NotificationTray` rendered once in the layout.

## More than one feature on a page

This is the payoff. A page calls **each feature's hook** and matches each
phase **independently** — every feature loads, errors, and renders on its
own, with the same shared arms:

```rust
#[component]
fn Dashboard(id: String) -> Element {
    let example = use_example(id.clone());   // example-ui → AtomResult<Example>
    let invoice = use_invoice(id);           // billing-ui → AtomResult<Invoice>
    rsx! {
        section {
            match example {
                Success(ex) | Reloading(ex) => rsx! { ExampleCard { example: ex } },
                Initial | Loading           => rsx! { Spinner {} },
                Error { error, .. }         => rsx! { StatusError { message: error.to_string() } },
                Defect { defect, .. }       => rsx! { StatusError { message: format!("defect: {defect}") } },
            }
        }
        section {
            match invoice {
                Success(inv) | Reloading(inv) => rsx! { InvoiceCard { invoice: inv } },
                Initial | Loading             => rsx! { Spinner {} },
                Error { error, .. }           => rsx! { StatusError { message: error.to_string() } },
                Defect { defect, .. }         => rsx! { StatusError { message: format!("defect: {defect}") } },
            }
        }
    }
}
```

- **No waterfall** — the example section renders while billing is still
  loading; each section has its own spinner/error. A single combined "load
  everything first" would blank the whole page until the slowest call
  returns.
- **Same shape everywhere** — `Spinner`/`StatusError` are entity-agnostic,
  so every section reads identically. The only feature-specific bits are
  the hook and the dumb success component.
- **Each feature is self-contained** — `billing-ui` ships `use_invoice` +
  `InvoiceCard` + its own context/store; the shell provides both features'
  contexts at the `App` root and lays out the sections.

When a page genuinely needs **all-or-nothing**, combine the phases instead
of nesting sections — match a tuple or `.value()`-zip them — but
independent sections are the default and usually what you want.

## The rules

1. **Navigation is Dioxus.** `Link { to: Route::… }` / `nav.push` — never an
   `on_*` event-handler prop just to forward a route, and never a raw
   `<a href>` (the typed `Route` lets the compiler catch dead links;
   [idioms §4](@/architecture/idioms.md)).
2. **State is a phase match.** Read a feature hook, `match` the
   `AtomResult` (or `Async::state()`). Loading/error/defect are shared
   chrome; success is feature content.
3. **Features own state + presentation; the shell owns routing +
   composition.** The data wiring (clients, store, fetch) never leaks into
   a page; routes never leak into a feature.
4. **Writes are optimistic.** Mutations patch the store instantly and
   reconcile/roll back ([optimistic state](@/architecture/optimistic.md)).

The worked reference is the example app:
`examples/app/features/example/example-ui/` (feature) +
`examples/app/ui/src/pages/` (shell).
