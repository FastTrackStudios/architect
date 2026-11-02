+++
title = "Devices"
description = "This device on the account, an org's enrolled devices, pairing by code or QR, revoking — and the one admission table every peer-to-peer lane checks."
weight = 24
+++

## What it is and when to use it

A node lets a device in by the iroh endpoint id its connection proved. Three
things let one in, and `architect-devices` keeps all three in one table, the
node's `Admission`: the owner's other devices (the account pass over the
issuer's address book), devices an org's members enrolled with a role, and
devices paired by a six-digit code or a QR code or link. Every lane checks it:
a gate's engine, `crdt::p2p`'s document access, and the connection, which
closes when the device is revoked.

Use it for any node that serves peers: Task's node and server, Studio's
engine, a phone. Design: [Devices](@/architecture/devices.md).

## A minimal example

`architect-devices` with `account` and `iroh`; `crdt` with `vox`. After
`features/devices/tests/peer_to_peer.rs`:

```rust
use architect_devices::account::{self, FixedEndpoint, ThisDevice};
use architect_devices::{Admission, Pairings, Scope};
use crdt::p2p::{NodeAccess, Shares, node_router};

let devices = Admission::new();
let pairings = Pairings::new(devices.clone(), &endpoint.id().to_string(), "Cody's laptop");
let access = NodeAccess::new(devices.clone(), Shares::new()).pairing(pairings.clone());

// The account's other devices, on every reconnect.
account::pass(&devices_client, &token, &this_device, &FixedEndpoint(endpoint.id().to_string()), &devices).await?;

// Serve documents and pairing; a revoked device's connection closes.
architect_devices::serve(&endpoint, devices.clone(), move |remote| {
    node_router(registry.clone(), OpenDocs(registry.clone()), access.clone(), &remote.to_string())
});

// Pair somebody's iPad for one song: show `offer.code`, or `offer.ticket` as a QR code.
let offer = pairings.offer(vec![Scope::doc(song, &["read", "write"])], now_ms)?;
```

The iPad dials the laptop, calls `DevicePairingClient::pair` with
`PairProof::Code { code }` (or the ticket's `proof()`), and syncs the song.
`devices.revoke(&ipad_endpoint)` takes it back.

## Code tour

1. `features/devices/src/model.rs` `Peer`, `Grant`, `Scope`, `Route`: a device,
   what lets it in, what it may do, how to reach it.
2. `features/devices/src/admission.rs` `Admission`: the table (`set_account`,
   `admit`, `revoke`, `revoke_org`, `ends`, `to_json`), and the gate's
   `PermissionEngine` for `Principal::Host`.
3. `features/devices/src/account.rs` `pass`, `drive`, `AccountNode`: the
   account pass.
4. `features/devices/src/pairing.rs` `Pairings` (`offer`, `redeem`),
   `PairTicket`: codes and tickets.
5. `features/devices/src/service.rs` `DevicePairing`, `DeviceEnrolment`,
   `PairingHost`: the wire.
6. `features/devices/src/server.rs` `EnrolmentHost`, `project_members`: an
   org's enrolled devices as members its `ConfinedEngine` knows.
7. `features/devices/src/iroh.rs` `serve`, `closer`; `crates/architect/src/iroh_link.rs`
   `serve_peers_until`; `crates/architect/src/host.rs` `EngineHost::iroh_until`:
   closing a revoked device's connections.

## Tests to read

- `features/devices/tests/peer_to_peer.rs`: two devices of one account and a
  third paired by code edit one document live with the issuer stopped;
  revoking a device ends its session and closes its connection.
- `features/devices/tests/enrolment.rs`: per-org enrolment with roles.
- `features/crdt/crdt/tests/p2p.rs`: the document lanes over the same table.

## Status

Done in architect. Not yet: a device row holds one endpoint (two apps with
their own endpoints on one device cannot both register); the devices page
(architect#188); automatic route learning (remote access, AR1).

- **Task** still runs its own pass and enrolment
  (`task:apps/server/src/account_devices.rs`,
  `task:apps/server/src/device_enrollment.rs`,
  `task:crates/ui/src/device_pairing.rs`); task#404 moves it here.
- **Studio** plans engine pairing (studio#285, studio#370) onto this.
