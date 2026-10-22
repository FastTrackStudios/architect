+++
title = "Peers: documents between nodes"
description = "crdt::peer — pairwise document exchange with no hub, and per-document access the library checks."
weight = 58
+++

[`crdt::sync`](@/architecture/collaboration.md) is hub-and-spoke: replicas
attach to one canonical document on a host. That is half of local-first.
The other half (ADR 0006, "every node is a peer") is two nodes that each
hold a `DocRegistry` moving a document between themselves, with the
always-on host stopped or never involved. `crdt::peer`
(`features/crdt/crdt/src/peer.rs`) is that half. It was lifted from Task's
`vault-doc::peer` and `doc_peer.rs`, with the vault taken out.

## The pieces

| Item | What it is |
| --- | --- |
| `DocPeer` | The `#[vox::service]` lane a node serves to the nodes it admits. `list` returns every document the node holds, each with an opaque digest; `sync` is the ordinary `DocSync` session, addressed by a `DocAt { doc_id, name }` so the serving node can resolve (and register) a document it has never been asked for. |
| `DocPeerHost<C>` | Serves `DocPeer` over a `DocRegistry` and a `DocCatalog`. `DocPeerHost::new(registry, catalog, admit_peer)`: `admit_peer` receives the caller's iroh endpoint id. |
| `DocCatalog` | The app's index of documents: `held()` and `resolve(&DocAt)`. `OpenDocs(registry)` is the stock one (whatever the registry has open, digested by `vv_digest`), enough for tests and for apps whose doc ids are the address. |
| `SyncLane` | Something `exchange` can run a sync session through. `PeerLane { client, name }` is a dialled peer; `SelfLane::open(registry, principal)` is this node's own registry behind a gate that presents a fixed principal, so the registry's authorizer sees who is pulling. |
| `exchange(doc_id, local, remote)` | One document, both directions, through an in-memory replica. Local history goes up to the peer and the peer's comes down to the local registry, each through a normal client session, so each registry persists, relays to whoever has the document open, and applies its own access rules exactly as for a device's editor. Nothing about the pipe is remembered. Returns `Exchanged` or a `SessionEnd`. |
| `pull(local, peer, ours, per_doc)` | One sweep against one peer: `list`, skip documents whose digest matches `ours`, `exchange` the rest, each bounded by `per_doc`. Returns a `PullReport { listed, exchanged, pulled, foreign, failed }`. |

Both nodes sweep each other, so either pull alone converges two
documents. A pull carries both directions anyway, which is what lets an
edit reach a peer that cannot dial back (behind a NAT, asleep). Relayed
through a third node, an edit reaches a node it was never directly
exchanged with: A to B, then B to C, converges C with A while A is offline.

## Who may exchange

Two checks, both against the principal the router's `PermissionsGate`
resolved for the call (`architect::permissions_gate::caller()`):

1. **The node.** `DocPeerHost` serves only `Principal::Host` callers that
   its `admit_peer` check accepts (an org's own devices, by endpoint id).
   Everyone else, a signed-in user included, is refused on both `list`
   and `sync`. The lane is for nodes.
2. **The document.** `DocPeer::sync` attaches through the registry's
   `DocRegistry::with_authorizer`, which sees the same caller and decides
   per document. A refusal is `SyncError::UnknownDoc`, the same answer
   as a document that does not exist, so it leaks nothing about which
   ids are real.

Access is therefore checked by every node, in the library, per document.
Nothing trusts a machine because it answered.

A history that shares nothing with the served document (two nodes that
each created "the same" document separately) is refused with
`FOREIGN_LINEAGE` instead of merged, which would put every line in twice
(`is_foreign_lineage`, `check_lineage`).

## Wiring a node

The registry decides who may open what; the host decides which nodes may
dial; the gate in front of the router supplies the caller:

```rust,ignore
use crdt::peer::{DocPeerHost, OpenDocs, doc_peer_service_descriptor, DocPeerDispatcher};

let registry = DocRegistry::new(open_or_create)
    .with_authorizer(|who, doc_id| match who {
        Some(Principal::Host { endpoint }) => my_devices.contains(endpoint) && shared(doc_id),
        Some(Principal::Service { name }) => name == "self",
        _ => false,
    });

let host = DocPeerHost::new(registry.clone(), OpenDocs(registry.clone()), |endpoint| {
    my_devices.contains(endpoint)
});
let router = LayerRouter::new()
    .with(doc_peer_service_descriptor(), DocPeerDispatcher::new(host));
// Mount `router` behind a PermissionsGate (permit the DocPeer methods) on
// the node's EngineHost; peers dial it over iroh by endpoint id.
```

And the sweep a node runs against each peer it knows:

```rust,ignore
let local = SelfLane::open(registry.clone(), Principal::Service { name: "self".into() }).await?;
let ours = digests(&registry).await;            // doc id -> vv_digest, from OpenDocs::held
let report = crdt::peer::pull(&local, &peer_client, &ours, Duration::from_secs(10)).await?;
```

Peers find each other through iroh: n0 relays and DNS by default, or
`PeerLookup::N0AndLan` / `PeerLookup::Lan` (feature `iroh-lan`) to dial
by bare endpoint id on a LAN with the internet down
([HTTP and iroh](@/architecture/http.md)). A directory such as fts-auth
must never be the only way to find a peer.

## Tests

`features/crdt/crdt/tests/peer.rs` is the conformance suite and the
pattern to copy for an app's own peer tests (three in-process nodes,
each behind its own gate, dialled over `vox::memory_link_pair`):

| Test | Proves |
| --- | --- |
| `three_replicas_converge_without_a_hub` | A to B, B to C with A offline and no hub: C converges with A |
| `an_unadmitted_host_is_refused` | a `Principal::Host` that `admit_peer` rejects gets nothing |
| `the_registry_authorizer_sees_the_gates_caller` | per-document access sees the real caller, not `None` |
| `a_foreign_lineage_is_refused` | two unrelated documents with one id are not merged |
| `any_edit_order_converges` | proptest: random edit orders across replicas converge |

## Limits

- Fan-out beyond pairs is sweeps (or a host); there is no gossip.
- The digest is a version-vector digest: it skips unchanged documents,
  it is not an integrity check.
- `crdt::sync` (the hub) and `crdt::peer` (pairs) share one registry, so
  a node can be a host for its devices and a peer to another node at once.
