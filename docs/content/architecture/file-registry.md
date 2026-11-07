+++
title = "File registry and copies: every file, where it lives, how many verified copies"
description = "The spec for the files feature's registry (#347): one local index of every file in every root a device may see, searchable offline, and signed, verified accounting of which holders store each blob, so a copy count is a fact."
weight = 73
+++

This page extends [Project files](@/architecture/project-files.md)
([#347](https://github.com/FastTrackStudios/architect/issues/347)). Cody's
ask: "we keep a registry of the entire file database, so all systems know
all the files they have access to and cache that locally so we can still
search files on other devices offline and know where they are. Then as stuff
is replicated, we should always know what devices files are stored on, so we
know with 100% certainty how many copies of the data exist."

**What is there already.** Each device holds the manifest of every root it
can see (`features/files/src/sync/manifest.rs` `Manifest`): every path, size
and version, merged offline. `FILES_SEARCH` (`Engine::search` in
`features/files/src/sync/assets.rs`) walks every manifest on every query. A
device knows what it holds itself (`Engine::holds`) and nothing about anyone
else's bytes.

**What this adds.**

1. **The registry:** one persisted index per device, built from those
   manifests, searched in milliseconds and offline.
2. **The ledger:** each holder's signed statements of which blobs it holds,
   in what state, last verified when; synced like the manifests.
3. **Verification:** scrubs and possession challenges, so a copy is counted
   only on fresh evidence.
4. **Policy:** a minimum count of verified copies, and of sites, per org and
   per root. What is under it is reported, and an unpin or evict that would
   go under it is refused.

## The scale it is built for

On starcommand, under `/mnt/starcommand/Task`: 11 TB, 7 orgs, about 200
roots. tombrooksmusic alone has 31 projects. A project is a folder of songs,
and each song is a session file, a hundred `.rpp-bak`s and an
`Audio Files/` of hundreds of takes
(`Crescendum Album/Crescendum/Audio Files/148-V LEAD-241011_2006.wav`, a few
hundred KB to a few hundred MB each). That is roughly a million live files.
The registry is sized and benchmarked at 1,000,000 entries across 7 orgs and
210 roots.

## 1. The registry

**One entry per live file** in every root this device may open:

| Field | From |
|---|---|
| org, root (place and id), section | the manifest's `meta` (`RootMeta`) |
| path, name | the live version's path |
| size, mtime | `FileVersion::size`, `FileVersion::at_us` |
| sha-256 | `FileVersion::content` |
| versions | how many versions the path has |
| asset properties | a library asset's note frontmatter (`assets_of`), on the asset's note and on its media |

Every hash any version names (not only live ones) maps to the roots and
orgs that name it. That map scopes the ledger (below) and serves lookups by
hash.

**Kept current by the passes.** A root's entry set is rebuilt only when its
manifest changed: each root carries the digest of its document's version
vector (the one the peer lane already lists). A pass compares digests, and
re-reads only the roots that moved. A manifest that arrives over a live
bridge is in the registry at the next pass (every `files_secs`).

**Persisted.** One file per root in the agent's data dir
(`<data>/files/registry/<doc id>`), written when the root changes. At start
the registry loads them without opening a document, so search works before
the first pass.

**Search,** fully offline:

- words: every word must occur in the entry's haystack, which is the
  lowercase path, root name, org and property values;
- a content hash: `sha:<hex>` (8 or more digits) or a bare 64-digit hex
  finds every path holding those bytes, across roots and orgs. A shorter
  bare hex is a word, because take names hold dates (`241011_2006`);
- filters: `org:<slug>`, `root:<name>`, `kind:<asset type>`.

A haystack is one contiguous lowercase string per entry, so a query is a
linear scan with a fast substring test. The benchmark says whether that is
enough at a million entries; a trigram index is the next step if it is not.

**Every result says where the file lives:** the holders with a copy, how
many copies are verified, and whether one is reachable now (this device, or
a holder the agent is connected to at the moment).

**Access-scoped.**

- A device's registry holds only roots whose manifests it may open. The
  manifests arrive over the node's document lane, which checks every attach
  against the registry's authorizer.
- The lane's `list` is filtered the same way, so a device is never even told
  the names of roots it may not open. This is a small change to
  `features/crdt/crdt/src/peer.rs` `DocPeerHost`: `list` keeps only the
  documents the caller may open.
- **Revoking access removes the entries.** The engine records which peers
  list each root it does not hold itself. When a peer that listed a root
  answers a `list` without it, and no other peer lists it, the device forgets
  it: its registry entries, its ledger entries, its place in the catalogue.
- A device that is offline when access is revoked keeps the entries until it
  next sweeps that peer. It cannot fetch bytes for them in the meantime,
  because `BlobTransferHost::allow` serves only hashes in manifests the
  caller may read.

## 2. The ledger: who holds what

A **holder** is anything that stores blobs:

| Kind | Who | Signs with |
|---|---|---|
| `device` | a laptop's or desk's agent | its iroh key |
| `node` | the headless node (`files_hold`) | its iroh key |
| `s3` | an S3 or Garage bucket used as a blob backend | the key of the device that manages it |

A holder's id is its iroh endpoint id: the hex of its ed25519 public key.
That id is also the key that verifies its records, so a record cannot be
forged by a relay. An S3 holder's id is `s3:<endpoint>/<bucket>`. Its record
names its manager, the device that writes to the bucket, and the manager
signs for it. The S3 face (`feat/347-s3`) uses the engine's external-holder
call for this.

Every holder has a **site** (`home`, `starcommand`, `backblaze`, …). The
config sets it, and it defaults to the device's name. "Off-site" means at a
different site.

**An attestation**: "holder X holds blob H, in state S, last verified at T".

- **Fields:** hash, size, state, `verified_at`, the blob's Merkle root
  (below), bytes held (for a partial), written at, signature.
- **States:**
  - `full`: held deliberately. A pinned working copy, a root held in place,
    or the node's version store.
  - `cached`: fetched on open into the blob cache, and evictable.
  - `partial`: a download in progress, with its bytes so far.
  - `evicting`: being removed.
- Only `full` and `cached` are ever counted.

**A challenge record**: "verifier V asked X for chunk i of H at T: passed or
failed". V signs it.

**Where they live.** One Loro document per holder per org, named on the peer
lane `ledger/<org>/<holder>`, beside the manifests and through the same
catalogue and bridges. The holder is the only writer of its documents:

- `holder`: its kind, name, site, and manager for an S3 holder;
- `copies`: hash → attestation;
- `checks`: holder and hash → the latest challenge it made.

A blob is attested in the ledger of every org whose roots name it. A device
in one org never learns the hashes held for another, and access to an org's
ledger follows access to the org's documents, as the manifests' does.

**Policy** is one document per org, `ledger/<org>/policy`: scope (`""` for
the org, or a root's place) → `{min_verified, min_sites}`. Writing it needs
the role that writes the org's files.

**Kept at the moment things happen.** The engine updates its own
attestations in the same call that changes what it holds:

- a pin writes the working copy, then attests each file `full`;
- a fetch (on open, a library's notes) attests `cached` when the bytes land,
  and `partial` while a `.part` grows;
- an evict attests `evicting`, removes the blob, then removes the
  attestation;
- an unpin removes the working copy's `full` attestations. What stays in the
  cache becomes `cached`;
- a delete applied to a working copy removes the attestation of the bytes no
  longer held;
- a scrub that finds bad bytes removes the attestation at once (below).

Each pass also reconciles the attestations with what the device actually
holds: its indexes, its store, its `.part`s. A change made outside the
engine is caught there.

The ledger documents a device keeps are listed in its data dir. After a
restart it reads them before it writes, so nothing is signed again.

## 3. Certainty: verification, not bookkeeping

An attestation is a claim. A copy is **counted** only when there is recent
evidence that its bytes exist and are right.

**Evidence** comes in two kinds.

1. **A scrub** is a full re-hash by the holder. It reads the whole file,
   checks its sha-256 against the hash it is filed under, and computes its
   Merkle root on the same read. If the bytes match, `verified_at` is now.
   If they do not:
   - the attestation is removed;
   - a blob in the store is moved to `<data>/files/corrupt/`, and the node
     fetches a good copy back from a peer;
   - a working file whose bytes changed without a new stat is reported as
     damaged.
   Fetching and committing are scrubs too: both hash the bytes they move.
2. **A challenge** proves possession without moving the file.
   - The verifier picks a random chunk index i of blob H and asks holder X to
     prove it.
   - X answers with the chunk's bytes and the Merkle path from that chunk to
     H's root.
   - The verifier checks the path against a Merkle root it trusts. In order:
     its own, from its own scrub; another holder's, from its attestation;
     X's own.
   - A chunk is 1 MiB, so a challenge moves 1 MiB plus about 1 KiB of path,
     whatever the file's size.
   - **No answer is no evidence.** A holder that is offline or times out
     records nothing, so its copy ages towards stale. A failure is recorded
     only when the proof does not check, or the holder says it lacks the
     bytes.
   - The holder keeps each blob's chunk hashes (32 bytes per MiB, 350 MB for
     11 TB on the node) so it answers without reading the whole file. When
     they are missing it recomputes them.
   - S3 has no code to answer, so its manager answers for it. The manager
     reads the chunk from the bucket by range and the path from the chunk
     hashes it keeps.

**The Merkle root** is defined as follows:

- leaves: `sha256(0x00 ‖ chunk)` over 1 MiB chunks;
- inner nodes: `sha256(0x01 ‖ left ‖ right)`, with an odd node carried up;
- the empty file: `sha256(0x00)`.

It sits beside the sha-256 because the sha-256 of a whole file cannot check
a single chunk.

**Confidence of one copy** (holder X, blob H), at time now, with window W:

| State | When |
|---|---|
| **verified** | the newest evidence (X's own scrub, or any valid passed challenge of X for H) is within W, and no failed challenge is newer |
| **failed** | a failed challenge is newer than the last passed evidence |
| **stale** | the newest evidence is older than W: "unverified since T" |
| **unknown** | no evidence, a signature that does not verify, a size that disagrees with the manifest, or the state is `partial` or `evicting` |

- **Only verified copies are counted,** and a device that cannot vouch for a
  copy says so.
- An offline holder's copies stay verified until their evidence ages past W.
  They show as unreachable now, then as "unverified since …". The bytes are
  not gone because a laptop is asleep, but after W nothing vouches for them.
- W defaults to 30 days. Scrubs are scheduled so each blob a holder keeps is
  re-hashed every W/2: the stalest first, within a byte budget per pass. At
  11 TB that is about 9 MB/s on the node.
- Challenges are cheap and run every pass: each device challenges a few of
  the stalest copies held by peers it is connected to.

**The threat model.** Holders are the org's own devices and buckets. What
the design guards against is loss, rot, a file deleted or changed outside
the engine, a cache evicted behind the ledger's back, and stale bookkeeping.
It does not guard against a device built to lie. Signatures stop a relay or
a third device from speaking for a holder. A holder that fakes its own scrub
is caught by the first challenge from a peer that trusts another root.

**The views.**

- **Per file:** N verified copies on [holders, their sites], plus the
  unverified ones and why, each with its verification time and how it was
  verified (scrub, or a challenge by whom), and whether it is reachable now.
  Asked by path or by hash.
- **Per root and per org:**
  - files and bytes;
  - bytes at fewer than K verified copies (K is the policy's minimum, 2 if
    none is set);
  - bytes held by only one holder;
  - bytes with no verified copy;
  - each holder's bytes held and bytes verified.

**Policy.**

- **The minimum** is `min_verified` (verified copies) and `min_sites`
  (distinct sites among them). "At least 2 verified, one off-site" is
  `{min_verified: 2, min_sites: 2}`. A root's policy overrides its org's.
- **It applies to live files.** History is kept by the node and is not
  counted.
- **Violations** are reported per root, with each file under policy and what
  it lacks.
- **An unpin or evict that would go under it is refused.** This works the
  way unpin already refuses unsynced files: it names the files that would
  drop under the minimum. The check counts the verified copies other than
  this device's. Bytes that stay in this device's cache after an unpin still
  count.
- **Scheduled replication** (optional, per holder): a holder with
  `replicate_budget` bytes to spare fetches the files under policy that it
  does not hold, fewest verified copies first. It attests them `full`.

## 4. In the app

- **A file's and an asset's page** have a Copies section: each holder, its
  kind and site, its state, the confidence with its time ("verified 3 h ago
  by scrub", "unverified since 2 Nov"), and whether it is reachable now.
- **Storage health** is a page in the Files extension, per org and root:
  - copy counts, bytes under policy, bytes on one holder;
  - the holders, with bytes held and verified;
  - the violations, and the policy, which can be edited.
- **Search results** (the navigator's Files search and `FILES_SEARCH`) show
  each hit's holders, its verified count, and whether it is reachable now.
  An offline result says where to get it.

## 5. Actions

On `Files` (`features/files/src/agent.rs`), so each is a `FILES_*` action, on
the agent's HTTP face and over the node's `/mcp`:

| Action | Does |
|---|---|
| `FILES_SEARCH` | the registry: words, hash, filters; each hit with its locations |
| `FILES_LOCATIONS` | a path's or a hash's copies, with confidence |
| `FILES_COPIES` | the copy report of an org or a root |
| `FILES_POLICY_SET` | set a root's or an org's minimum |
| `FILES_POLICY_CHECK` | the files under policy in a scope |
| `FILES_SCRUB` | scrub now (a scope, or everything due) and challenge peers |
| `FILES_EVICT` | drop a file's or a root's cached bytes, refused under policy |
| `FILES_HOLDERS` | every holder this device knows, with its reachability |

The CLI is the Action List's: `architect-desktop` runs any of them by name
(`run FILES_COPIES --arg scope=tombrooksmusic`).

## The smallest pieces

| Piece | Where | Lines |
|---|---|---|
| The model: holders, attestations, confidence, reports, policy | files model module (wasm-clean) | 250 |
| Merkle tree, proofs | sync module `verify` | 150 |
| The ledger documents, signing | sync module `ledger` | 400 |
| The registry: build, persist, search | sync module `registry` | 350 |
| Engine: holdings, scrub, challenges, evict, policy guard, reports | sync module `copies` | 450 |
| Agent methods | `Files`, the engine's impl | 120 |
| Node: identity, `Possession` on the peer lane, ledger names in the catalogue, forgetting revoked roots | `apps/desktop/src/node/files.rs`, `apps/desktop/src/node/catalog.rs` | 200 |
| `DocPeerHost::list` filtered by the caller | `features/crdt/crdt/src/peer.rs` | 20 |
| UI: copies on file and asset pages, storage health, search locations | `features/files/extension/src/lib.rs` | 300 |
| Tests | the files feature's and the node's | 500 |

**Shared engine calls this touches** (named for `feat/347-drive` and
`feat/347-s3`):

- `Engine::unpin` gains the policy guard;
- `Engine::search` answers from the registry;
- `Engine::pass_all` reconciles attestations, scrubs and challenges after
  its passes;
- a successful write to a working copy attests the copy.

Nothing else in the engine's existing calls changes. The drive needs no
change: its opens go through the fetcher, which attests. The S3 face uses
`Engine::attest_external` and implements the `Prover` trait for its bucket.

## What is left after this

- A trigram index if linear search is not fast enough at the scale above.
- Replication to S3 driven by policy (with `feat/347-s3`).
- Erasure coding and repair (today a damaged copy is refetched whole).
- A holder's capacity and free space in its record, so scheduled
  replication picks where to put bytes rather than being configured.
