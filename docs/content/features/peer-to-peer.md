+++
title = "Peer to peer"
description = "Devices sync documents with each other directly over iroh, admitted per document; a share link opens one live document to a guest."
weight = 23
+++

## What it is and when to use it

Every node is a peer. Nodes that each hold a `DocRegistry` exchange documents
directly, with no hub: a pairwise sweep (`crdt::peer::pull`) skips documents
whose version digest already matches and syncs the rest both ways. Each node
checks access per document in the library. The caller is the iroh endpoint id
the handshake proved (a `Principal::Host`, admitted if it is one of the
person's devices from the auth directory), or a guest holding a share link's
key (`fts-doc:<endpoint>/<doc>?key=…`). Revoking a link ends its live sessions.

Use it so the owner's devices sync with the server stopped or never involved,
and to hand someone a live link to one document. Design:
[Peer to peer](@/architecture/peers.md) and
[Collaboration](@/architecture/collaboration.md).

## A minimal example

`crdt` with `vox` (native); `architect` with `iroh`, plus `iroh-lan` for LAN
lookup. After `features/crdt/crdt/tests/p2p.rs`:

```rust
use architect::iroh_link::{self, PeerLookup};
use crdt::p2p::{Devices, NodeAccess, Shares, node_router};
use crdt::peer::OpenDocs;
use crdt::registry::DocRegistry;

let endpoint = iroh_link::bind_endpoint_with(secret_key, PeerLookup::N0AndLan).await?;
let access = NodeAccess::new(Devices::new(), Shares::new());
let registry = DocRegistry::new(open_doc).with_authorizer({
    let access = access.clone();
    move |who, id| access.admits(who, id)
});
access.devices.replace(directory.peer_endpoints(this_device));   // from auth's DeviceDirectory

let (ep, reg, acc) = (endpoint.clone(), registry.clone(), access.clone());
tokio::spawn(async move {
    iroh_link::serve_peers(&ep, move |remote| {
        node_router(reg.clone(), OpenDocs(reg.clone()), acc.clone(), &remote.to_string())
    })
    .await
});

// Share one document live; the guest joins with the text
let link = access.shares.share(&endpoint.id().to_string(), doc_id).to_string();
```

The guest parses the text as a `DocLink`, dials `link.endpoint` with
`iroh_link::connect`, adds `LinkKey::new(&link)` as middleware on a
`DocSyncClient`, and runs a `SyncedDoc` over it. A device sweeping a peer
uses `SelfLane::open` for its own side and `pull(&local, &peer, &ours, timeout)`.

## Code tour

1. `crates/architect/src/iroh_link.rs` `PeerLookup`, `bind_endpoint_with`: an
   endpoint found through n0, the LAN, or both.
2. `crates/architect/src/iroh_link.rs` `serve_peers`: a handler per QUIC
   connection, built from the proved remote id.
3. `features/crdt/crdt/src/p2p.rs` `node_router`: document sync and the peer
   lane behind a `PermissionsGate` whose engine is `NodeAccess`.
4. `features/crdt/crdt/src/p2p.rs` `NodeAccess` (`caller`, `admits`),
   `Devices` (`replace`, `forget`), `Shares` (`share`, `revoke`, `revoke_doc`).
5. `features/crdt/crdt/src/link.rs` `DocLink`, `LinkKey`: the link text and the
   middleware sending its key as a bearer.
6. `features/crdt/crdt/src/peer.rs` `DocPeerHost`: `list` and `sync`, for
   admitted hosts only, after a lineage check.
7. `features/crdt/crdt/src/peer.rs` `SelfLane`, `exchange`, `pull`: the
   pairwise sweep.
8. `features/auth/auth-proto/src/device.rs` `DeviceDirectory`: the person's
   other devices' endpoint ids.

A device dials with `iroh_link::connect`. `serve_peers` hands the proved id to
`node_router`, and the gate resolves the caller with `NodeAccess::caller`: a
guest if a live link key came with the call, else a host. The gate refuses an
unregistered endpoint before any service runs; then the registry's authorizer
decides per document. In a sweep, `pull` lists the peer's documents, skips
equal digests, and for each other one `exchange` syncs local to remote to
local through an in-memory pipe, so history moves both ways.

## Tests to read

- `features/crdt/crdt/tests/peer.rs`: three in-process nodes converge without a
  hub; an unadmitted host is refused; the authorizer sees the gate's caller; a
  foreign lineage is refused; any edit order converges (proptest).
- `features/crdt/crdt/tests/p2p.rs`: real iroh endpoints: a link joins a live
  document and edits flow both ways, a revoked link ends; registered devices
  edit live with no server; a removed device is refused; devices on one
  network find each other (needs multicast).
- `crates/architect/tests/iroh_lan.rs` and `crates/architect/tests/iroh_link.rs`:
  dialling by bare id, redial after a restart.

## Status

Done in architect. Not yet: gossip (fan-out beyond pairs is sweeps);
read-only links; choosing between two foreign lineages needs a person.
`EngineHost`'s iroh lane puts the host's gate in front of each peer, the
caller the `Principal::Host` its handshake proved; a router built per peer
means calling `serve_peers` as above. The digest is a skip check, not an integrity
check.

- **Task** uses the pairwise lane (`task:apps/server/src/doc_peer.rs`, with
  `DocPeerHost`, `SelfLane` and `exchange`); it does not use `crdt::p2p`,
  `serve_peers` or share links yet.
- **Studio** runs hub-style sync over iroh with its own ticket format
  (`studio:apps/session/daw/src/collab.rs`); the peer lane and links are still
  to adopt.
