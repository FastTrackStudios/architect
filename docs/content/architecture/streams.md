+++
title = "Server push: streams end to end"
description = "PubSub fan-out, Entity events, #[subscribe] siblings, the client hooks — and the real-time-safety rules."
weight = 45
+++

architect's push story is one shape repeated at every layer: **the
client passes a channel sink into a subscribe RPC; the server fans
events out into every attached sink.** No second wire protocol — the
sink is a `vox::Tx<Event>`, the subscription is an ordinary service
method, and events ride the same socket (or
[in-process link](@/architecture/local.md)) as the CRUD calls.

Three layers, from server to screen:

1. **`PubSub<T>`** — the server-side hub (vox feature).
2. **Declarations** — the Entity `events` derive flag, or
   `#[subscribe]` on an `#[architect::rpc]` trait.
3. **`use_stream` / `use_store_stream`** — the client hooks
   (`atom` + `vox` features).

## 1. `PubSub<T>` — the fan-out hub

`architect::PubSub` is effect's `PubSub` + `SubscriptionRef` adapted to
wire subscribers: a cloneable hub of `vox::Tx` sinks. `publish(event)`
never waits — overflow is resolved per subscriber, by the **strategy**
chosen at construction:

| Constructor | On a full mailbox |
| --- | --- |
| `PubSub::sliding(cap)` | drop the **oldest** queued event — right default for state-shaped events (`Upserted(row)` supersedes older news) |
| `PubSub::dropping(cap)` | drop the **incoming** event (the backlog wins) |
| `PubSub::unbounded()` | never drop; the mailbox grows |

(effect's fourth strategy — back-pressure that suspends the publisher —
is deliberately absent: blocking the writer is exactly what a host
integration can't afford.)

Two more knobs:

- **Replay** — `.with_replay(n)` hands every new subscriber the last
  `n` published events before live traffic. `sliding(cap).with_replay(1)`
  is the cheap snapshot for state-shaped streams.
- **Snapshot-then-changes** — the buffered attach gives a subscribe
  implementation effect-`SubscriptionRef` semantics (*current state
  first, then every change*) without holding the hub's lock across an
  async snapshot read:

```rust,ignore
async fn subscribe(&self, sink: Tx<ExampleEvent>) -> Result<(), RepoError> {
    let pending = self.hub.begin_attach(sink);   // mailbox collects from here
    let rows = self.inner.list(everything()).await?;     // no lock held
    self.hub.complete_attach(pending, Some(ExampleEvent::Snapshot(rows.items)));
    Ok(())   // on a failed read: self.hub.abort_attach(pending)
}
```

Changes that landed during the read are delivered *after* the snapshot
— possibly already contained in it. That's why event payloads should
carry **full state** (idempotent re-application), never diffs.

## 2a. Entity events — the `events` derive flag

For CRUD entities, don't write any of the above. Add `events` to the
derive (`#[architect(table_name = …, repo, store, events)]`) and the
[Entity derive](@/architecture/pattern.md) emits the whole story:

```rust,ignore
enum ExampleEvent { Snapshot(Vec<Example>), Upserted(Example), Deleted(Uuid) }
#[vox::service] trait ExampleEvents { async fn subscribe(&self, sink: Tx<ExampleEvent>) … }
struct ExampleEvented<R> { … }    // publish-through repo wrapper + subscribe host
fn use_example_events()           // (with `store`) client hook — see §3
```

`ExampleEvented::new(repo)` wraps any `ExampleRepo` backend: every
successful `create`/`update`/`delete` passes through and broadcasts
`Upserted(row)` / `Deleted(id)`; `subscribe` does the buffered-attach
snapshot above, so subscribing alone fully hydrates a client store.
Its `Services` bundle mounts CRUD **and** the event feed, so
`ExampleEvented::new(backend).into_router()` serves both.

Two rules, learned the hard way:

- **Wrap once per process.** The hub lives inside the wrapper;
  per-connection routers must share the one instance (the example
  server wraps at startup and builds a router per WebSocket connection
  over it). Wrapping twice splits the hub — half your subscribers go
  deaf.
- **`hub()` is public** for changes that don't come through the repo —
  host pollers, CRDT merge hooks — publish into the same hub and every
  subscriber sees them.

## 2b. `#[subscribe]` on rpc traits

Hand-written services declare streams inline. A method marked
`#[subscribe]` is a **declaration, not a callable method** — it names
an event type and is stripped from the trait
(`examples/layered-services/src/main.rs`, the ticker):

```rust,ignore
#[derive(Clone, Debug, PartialEq, facet::Facet)]
pub struct TickEvent { pub value: i64 }

#[architect::rpc]
pub trait Ticker {
    fn tick(&self) -> i64;

    /// Every counter change, as it happens.
    #[subscribe]
    fn ticks(&self) -> TickEvent;
}
```

The macro emits a vox **stream sibling** next to the base service:

- `TickerStream` — `async fn ticks(&self, sink: Tx<TickEvent>)`, with
  `TickerStreamClient` / `TickerStreamDispatcher` from vox.
- `TickerStreamSource` — the backend contract. The **no-arg hub form**
  asks for `fn ticks_hub(&self) -> &PubSub<TickEvent>`: the backend
  owns the hub, publishes into it on every state change, and the
  emitted host attaches each subscriber sink.
- `stream_serve` / `stream_layer` / `TickerStreamService` — mount
  verbs, parallel to the base trait's `serve`/`layer`/`Service`. The
  sibling is one more token in the bundle:

```rust,ignore
impl Services for LiveBackend {
    fn layers() -> impl Layer<LiveBackend> {
        layers![CounterService, TickerService, TickerStreamService]
    }
}

impl Ticker for LiveBackend {
    fn tick(&self) -> i64 {
        let mut g = self.counter.lock().expect("counter poisoned");
        *g += 1;
        self.ticks.publish(TickEvent { value: *g });   // publish-on-write
        *g
    }
}

impl TickerStreamSource for LiveBackend {
    fn ticks_hub(&self) -> &architect::PubSub<TickEvent> { &self.ticks }
}
```

A declaration **with filter params** —
`#[subscribe] fn events(&self, filter: F) -> E` — switches the backend
contract to the **attach form**: `fn events_attach(&self, filter: F,
sink: Tx<E>)`. Filtering, per-filter hubs, and snapshot-then-changes
(`begin_attach`/`complete_attach`) stay backend-owned; the emitted host
just forwards.

## 3. The client side

`architect::use_stream` (Dioxus; `atom` + `vox`) subscribes a component
to a stream for its lifetime. It rides `use_resource`, so the
subscription **re-establishes itself** when any signal the subscribe
future reads changes — read the connection state inside it and a
reconnect resubscribes automatically:

```rust,ignore
use_stream(
    move |sink| async move {
        match conn.state() {
            ConnectionState::Ready(c) => c.subscribe(sink).await.is_ok(),
            _ => false,                      // not up yet — retried on Ready
        }
    },
    |ev: TickEvent| { /* fold into state */ },
);
```

`use_store_stream(store, subscribe, apply)` aims the same machinery at
the optimistic [`Store`](@/architecture/optimistic.md): `apply` is a
plain `fn` matching the event enum onto `Store::put` /
`Store::remove_real` / `Store::hydrate`. Every page already rendering
from the store goes **live** — rows appear/update/vanish as other
clients change them — with zero page changes, and optimistic local
writes overlay incoming server truth exactly as they do over fetches.

With the Entity `events` + `store` flags you don't even write that:
the derive emits `use_example_events()`, folded into the one-call
`provide_example()` (store + live events) the app root mounts — that
single line in `examples/app/ui/src/lib.rs` is the whole client-side
wiring.

Subscriptions work identically over the in-process transport — the
layered-services example drives `TickerStreamClient` through
`LocalServer::establish` and asserts publish-on-write delivery, no
socket involved.

## Real-time safety: never publish from an audio thread

**`PubSub::publish` takes a mutex** (and clones the event per
subscriber). That's fine from a main thread or a tokio worker. It is
**never acceptable from a real-time thread** — an audio callback
blocking on a lock held by a lower-priority thread is a priority
inversion, and the missed deadline is an audible dropout. The same ban
covers allocation, channel sends that can park, and syscalls. This
isn't theoretical: FastTrackStudio's DAW publishes from REAPER audio
callbacks, which is exactly why `architect::rt` exists.

The `rt` feature is the sanctioned escape hatch — a **wait-free SPSC
ring** (`rtrb`) that splits the publish in two:

- **`RtProducer::push`** — wait-free, allocation-free, lock-free. Call
  it from the audio callback. A full ring **drops the event and bumps a
  counter** (`dropped()`), never blocks.
- **`RtConsumer::drain` / `drain_into`** — called from a normal thread
  (the same UI-rate tick that already drives pollers), pulling
  everything queued and handing it on — `drain_into(&hub)` publishes
  straight into the `PubSub` feeding your `#[subscribe]` subscribers.

```rust,ignore
// setup (non-RT): ring sized for ~2 ticks of worst-case event rate
let (mut rt_tx, mut rt_rx) = architect::rt::rt_channel::<PeakEvent>(1024);

// audio callback (RT): wait-free push, drop-on-full
rt_tx.push(PeakEvent { l, r });

// UI-rate tick (non-RT): fan out to subscribers
rt_rx.drain_into(&backend.peaks_hub());
```

For **latest-value streams** — meters, transport position — where only
the freshest sample matters, drain with `RtConsumer::latest()` and
publish the single survivor: a full ring then costs staleness, not
correctness.

Rules of thumb:

| Thread | May call |
| --- | --- |
| audio / real-time | `RtProducer::push`, `dropped()`, `is_abandoned()` — nothing else in this stack |
| normal (UI tick, tokio) | `RtConsumer::drain` / `latest` / `drain_into`, `PubSub::publish`, everything |

Size the ring for the worst-case burst between two drains and watch
`dropped()` in development — a climbing counter means the ring is too
small or the drain stalled.
