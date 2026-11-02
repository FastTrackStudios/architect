+++
title = "Devices: who a node lets in"
description = "architect-devices — this device on the account, an org's enrolled devices, pairing a device by code or QR, revoking, and the one admission table every peer-to-peer lane checks."
weight = 58
+++

Every node (a phone, a laptop, an org's server, an engine) serves its lanes
over iroh, where the QUIC handshake proves the caller's endpoint id
(`Principal::Host`). Which endpoints a node lets in, and what each may do,
used to be decided four ways in Task (`task:apps/server/src/account_devices.rs`,
`task:apps/server/src/device_enrollment.rs`, `task:crates/ui/src/device_pairing.rs`,
files' `enroll_device`), and Studio was about to add a fifth for engine
pairing. `architect-devices` (`features/devices`) is the one way.

## One table

`features/devices/src/admission.rs` `Admission` is a node's table of the
devices it lets in, by endpoint id. Each `Peer`
(`features/devices/src/model.rs` `Peer`) carries its name, platform,
install id, how to reach it (`Route`: a relay, a direct address) and the
grants that let it in (`Grant`):

| Grant | How a device gets it | What it may do |
| --- | --- | --- |
| `Grant::Account` | one of the owner's devices, from the issuer's address book (the account pass) | whatever the owner may do on this node |
| `Grant::Org { org, role, by }` | a member enrols it in an org, with a role at most their own | what that role may do in that org, decided by the org's `ConfinedEngine` |
| `Grant::Paired { scopes, by }` | it redeemed a pairing offer made on a signed-in device | exactly the offer's `Scope`s (narrow only) |

Every lane checks the same table:

- **As the gate's engine.** `Admission` is a `PermissionEngine` for
  `Principal::Host`: an admitted device may do what its grants allow
  (`Peer::allows`), any endpoint may ask to pair (`PAIR_RESOURCE`), and
  nobody else anything. Compose it with the engine for people
  (`CompositeEngine`, first allow wins) on an `EngineHost`'s iroh lane.
- **For documents.** `crdt::p2p::NodeAccess` holds an `Admission`: an account
  device opens any document and sweeps (`DocPeer`), a paired device opens
  the documents its scopes name (`docs/<id>`), no sweeps.
- **For connections.** `Admission::ends(endpoint)` resolves once a device
  that was let in is let in no more. `features/devices/src/iroh.rs` `serve`
  (and `closer`, for `EngineHost::iroh_until`) closes the device's iroh
  connections on it, through `crates/architect/src/iroh_link.rs`
  `serve_peers_until`; `crdt::p2p` ends its live `DocSync` sessions on it.

So revoking a device (`Admission::revoke`, `revoke_org`, or an account pass
that no longer lists it) refuses its next call, ends its sessions and
closes its connections, on every lane.

The table is the node's to keep: `Admission::to_json` / `restore_json`
(and `observe`, called after every change) so a node starts offline with
the devices it knew.

## The account

`features/devices/src/account.rs` `pass` is one round of a device's side of
the issuer's address book (`features/auth/auth-proto/src/device.rs`
`DeviceService`), lifted from Task's `account_devices.rs`:

1. register this install (`ThisDevice`), offering an org of its own if the
   account shares none yet (`AccountNode::offer`): the first device's org is
   the one they all share;
2. take this device's leftover rows off the account (`stale_rows`: an id an
   older build used, a same-named row unrefreshed for `STALE_AFTER`);
3. hold the shared org (`AccountNode::hold`: Task's node adopts or joins it;
   `FixedEndpoint` is an app with one endpoint and no org, Studio's engine)
   and register the endpoint for it;
4. `Admission::set_account` with the others: each holds `Grant::Account`, a
   device off the list loses it, other grants stand.

`drive` runs it on start, whenever a reconnect watch changes and every few
minutes. The issuer only introduces devices; once they know each other they
sync with it stopped.

## An org's devices

On an org's host, `features/devices/src/server.rs` `EnrolmentHost` serves
`DeviceEnrolment` (`features/devices/src/service.rs`) on the org's lanes:
`enrol` (a member enrols a device with a role no higher than their own;
enrolling again changes it), `enrolments` (an owner or admin sees all, a
member their own) and `revoke` (the member who enrolled it, or an owner or
admin). The org is the lane's (`architect_orgs::server::current_org`), never
an argument.

`project_members` keeps `architect_orgs::server::Members` in step with the
table, so an enrolled device is a `Principal::Host` member of its orgs and
the org's `ConfinedEngine` gives it its role's rules there, exactly as for a
person with that role. Revoking takes it out of `Members`.

## Pairing

For a device that is not signed in, or is somebody else's
(`features/devices/src/pairing.rs`):

1. On a signed-in device a person chooses what to grant, and the node makes
   an offer: `Pairings::offer(scopes, now_ms)` gives a six-digit `code` and a
   `PairTicket` for a QR code or link. A ticket is
   `fts-pair:<endpoint>#<secret>`, or `PairTicket::url(base)`,
   `<base>?pair=<endpoint>#<secret>`: the secret rides the fragment, which a
   browser never sends.
2. The new device dials the offering node by endpoint id (the ticket's, or
   one picked from the devices nearby, `iroh_link::LanBrowser`) and calls
   `DevicePairing::pair` with the code or the secret
   (`PairProof`). `NodeAccess::pairing` mounts the service on the node's
   lane; `PairingHost` serves it on any other router.
3. The offer is used up and the endpoint the connection proved is admitted
   with `Grant::Paired { scopes }`. The answer (`Paired`) says who let it in
   and what it may do.

An offer lasts `OFFER_TTL_MS` and is single use. Scopes must be narrow
(`Scope::is_narrow`: a named resource, named actions). A six-digit code is
guessable in principle, so every wrong code or secret costs every open
offer one of its `CODE_TRIES`; an offer out of tries is withdrawn. No server
takes part.

## Tests

| Test | Proves |
| --- | --- |
| `features/devices/tests/peer_to_peer.rs` `two_account_devices_and_a_paired_one_edit_live_with_the_issuer_stopped` | two devices of one account meet through a real issuer, a third is paired by code, and all three edit one document live over loopback iroh with the issuer stopped; the paired one opens nothing its scopes do not name |
| `features/devices/tests/peer_to_peer.rs` `revoking_a_device_closes_its_lanes` | taking back a pairing, and taking a device off the account, ends its live session and closes its connection; it gets nothing after |
| `features/devices/tests/peer_to_peer.rs` `a_code_pairs_once_and_a_wrong_one_nothing` | a wrong code admits nothing, a stranger may only ask to pair, an offer is used once |
| `features/devices/tests/enrolment.rs` | a member enrols a device with a role the org's engine then enforces, never above their own; listing and revoking by owner or admin; non-members refused |

No test needs multicast: endpoints bind loopback and dial by address.

## Not yet

- One endpoint per device row: the issuer's `AuthDevice` holds one
  `endpoint_id`, so two apps on one device with endpoints of their own
  (Task's node, Studio's engine) cannot both register theirs yet.
- The devices page (this device, the account's others, enrolments, revoke)
  comes with the sign-in UI (architect#188).
- Learnt routes (direct addresses from a connection, LAN sightings) are
  recorded by `Admission::learn_route`, but nothing records them
  automatically; the route list and its preflight are remote access (AR1).
