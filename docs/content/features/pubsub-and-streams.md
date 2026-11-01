+++
title = "PubSub and streams"
description = "Server push: PubSub hubs, #[subscribe] streams over vox and SSE, the client hooks, and the pump that delivers past a full sink."
weight = 15
+++

## What it is and when to use it

`PubSub<T>` is a server-side fan-out hub. `publish` never blocks the
publisher: each subscriber has a mailbox with an overflow strategy
(`sliding` drops the oldest, `dropping` drops the incoming, `unbounded`
keeps all), an optional replay of the last events, and an attach form that
sends a snapshot and then the changes after it. A `#[subscribe]` method on a
service turns a hub into a vox stream and an SSE route; on the client,
`use_stream` and `use_store_stream` follow it from Dioxus.

Use it for live signals (meters, playheads, "something changed"), never a
CRDT for those. Don't publish from an audio thread: hand events to
`architect::rt` there. Design: [Server push](@/architecture/streams.md).

## A minimal example

From `examples/greeter/src/lib.rs`. `PubSub` needs `vox`; the pump needs
`platform`, which `local` and `server-axum` already turn on.

```rust
#[architect::service]
pub trait Greeter {
    async fn greet(&self, name: String) -> Result<String, GreetError>;
    #[subscribe]
    fn greeted(&self) -> String;
}

struct Memory { greeted: PubSub<String>, /* .. */ }
// Memory { greeted: PubSub::sliding(64), .. }

impl GreeterStreamSource for Memory {
    fn greeted_hub(&self) -> &PubSub<String> { &self.greeted }
}

// In greet:
self.greeted.publish(greeting.clone());   // returns the live subscriber count
```

An in-process consumer takes `hub.subscribe(capacity)` (an
`async_channel::Receiver<T>`). Over HTTP, `GreeterHttpClient::greeted()`
returns an `EventStream` of SSE events. In Dioxus, `use_stream(subscribe,
on_event)` holds the subscribe call in flight and resubscribes after it ends.

## Code tour

1. `crates/architect/src/pubsub.rs` `PubSub` (`sliding`, `dropping`,
   `unbounded`, `with_replay`) and `EventSink`: a vox `Tx` or a local channel.
2. `crates/architect/src/pubsub.rs` `begin_attach`, `complete_attach`,
   `complete_attach_all`: snapshot, then changes, with nothing lost between.
3. `crates/architect/src/pubsub.rs` `publish`, `deliver`, `drain_ready`: each
   mailbox under its strategy, then `try_send` into the sink until it refuses.
4. `crates/architect/src/pubsub.rs` `pump`, `after_pumped`, `PumpRuntime`: the
   full-sink retry (below).
5. `crates/architect-rpc-derive/src/lib.rs` `emit_stream_block`: the
   `<Trait>StreamSource` trait (`<name>_hub`, optional `<name>_intro` and
   `<name>_attach`) and `<Trait>StreamService`.
6. `crates/architect/src/http.rs` `sse`: SSE over a local `EventSink`.
7. `crates/architect/src/stream.rs` `use_stream`, `use_store_stream`; and
   `crates/architect/src/rt.rs` `rt_channel`, `drain_into` for real-time producers.

A subscription (a vox call carrying a `Tx`, or `GET /greeter/greeted`)
reaches the generated host, which calls `begin_attach(sink)` and then
`complete_attach` with the source's intro, if any. Each `publish` puts the
event in every mailbox and drains what the sink takes. The vox call stays in
flight for the subscription's life; dropping it, or closing the HTTP stream,
closes the sink, and the subscriber is pruned at the next sweep.

### The full-sink retry

A sink has its own credit, so it can refuse an event even when the mailbox
has room. Then the refused event goes to that subscriber's pump: a detached
task, spawned on the publisher's tokio runtime (or the runtime seen at
`begin_attach`; on wasm through `platform::spawn`), holding a weak reference
to the hub. It awaits `sink.send(event)`, which is real back-pressure but only
on the pump, never on the publisher, then drains the mailbox again, and ends
when the sink takes everything without waiting, closes, or the hub is gone.
While it runs, new publishes queue in the mailbox under its strategy, so
order holds. With no runtime at all, the event goes back to the front of the
mailbox and leaves on the next `publish`.

## Tests to read

- Unit tests in `crates/architect/src/pubsub.rs` (features `vox` and
  `platform`): an event that meets a full sink arrives without another
  publish; publishes behind a pump keep their order; a sink that closes under
  its pump is pruned; a thread with no runtime publishes through the attached
  one; with no runtime a full sink waits for the next publish.
- `crates/architect/tests/http_face.rs`: subscriptions as SSE, with the hub and
  the attach forms; an argless subscription's intro over SSE; an entity's
  events.
- `examples/greeter/tests/round_trip.rs`: streams over HTTP.
- `features/files/tests/files.rs`: events behind the permissions gate reach
  only who may see them.

## Status

Done in architect; the pump is newer than the latest tag. Limits: `publish`
takes a mutex, so it is not real-time safe; SSE carries no typed errors; a
late subscriber misses events unless the source has an intro or replay;
pruning is lazy.

- **Task** uses `PubSub`, `#[subscribe]` and `use_stream` widely (for example
  `task:apps/server/src/live.rs`). Its tag has no pump: a full sink waits for
  the next publish.
- **Studio** uses `PubSub` (`studio:features/adapters/reaper/daw-reaper/src/event_hub.rs`),
  `#[subscribe]` and `architect::rt`. Moving to a tag with the pump adds a
  `Send + Sync` bound on event types.
