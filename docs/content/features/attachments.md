+++
title = "Attachments"
description = "architect-attachments: files attached to a document or a record, kept by hash, offline first, uploaded resumably and once, under a quota, previewed through renditions, readable by whoever may read the parent."
weight = 27
+++

## What it is and when to use it

`architect-attachments` hangs files off a **parent**: a co-edited document,
or a record of the app's (a task, a contact, an invoice). Use it for the
image pasted into a note, the PDF on a task, the clip on a review item: files
that belong to something, not files a person browses by name (those are
[Files](@/features/files.md)).

An attachment is a blob named by its sha256. A document lists its
attachments itself, in a Loro map, so attaching works offline and syncs with
the document. The bytes travel apart, by hash: pushed when the device is
online, fetched by another device when it opens the document. Access follows
the parent. The design is in [Documents](@/architecture/documents.md),
section "Attachments"; the API is in `skills/architect/references/files.md`.

## A minimal example

From `features/attachments/tests/attachments.rs`:

```rust
use architect_attachments::{AttachmentsClient, AttachmentsRemote, Parent, attach_local, doc};
use architect_attachments::server::AttachmentsServer;

// Server: behind the gate on the org's lane; access by the gate's engine.
let attachments = AttachmentsServer::new(&data, blobs.clone(), Arc::clone(&engine))
    .with_docs(docs)                                     // the server's copies of documents
    .with_renditions(files.rendition_engine().cloned()); // previews, shared with files
let router = router.with(attachments_service_rpc_service_descriptor(), serve(attachments.clone()));
let gate = gate.permit(attachments_service_rpc_service_descriptor(), &architect_attachments::PERMITS);

// A note's kind keeps its attachments as its blobs.
let kind = DocKind::new("app.note", "Note").blobs(doc::refs);

// Device A, offline: bytes into its store, the entry into the note.
let shot = attach_local(&store, note.loro(), "shot.png", "image/png", &png, now_us, None).await?;
// Back online: push what the server lacks.
AttachmentsClient::new(caller).push_doc(&store, note_id, note.loro()).await?;

// Device B: the note arrived by sync; its attachments by hash.
let fetcher = Fetcher::new(store_b)
    .with_remote(Arc::new(AttachmentsRemote::new(&client_b, Parent::doc(note_id))));
crdt::blobs::keep_offline(&kind, note_b.loro(), &fetcher).await?;
```

A record's attachments go through the server: `push` the bytes, then
`AttachmentsService::attach(Parent::record("task", id), attachment)`.

### In a browser

Feature `http` mounts `WebAttachments` beside the lane. A page mints a ticket
for the parent over the lane and builds plain URLs:

```rust
let web = WebAttachments::new(attachments.clone(), identity); // the gate's resolver
let app = architect::layers![architect_attachments::web::Service].provide_http(&web);

let ticket = client.service.ticket(Parent::doc(note_id)).await?;
let src = web::content_url("", &org, &Parent::doc(note_id), &shot.hash, Some(&ticket.token));
let poster = web::rendition_url("", &org, &parent, &clip.hash, RenditionKind::Poster, Some(&ticket.token));
```

## Code tour

1. `features/attachments/src/model.rs` `Parent` (`resource`, `parse`),
   `Attachment`, `Upload`, `AttachFault`: the model.
2. `features/attachments/src/doc.rs` `attach`, `detach`, `list`, `refs`: a
   document's own list, in its `attachments` map.
3. `features/attachments/src/service.rs` `AttachmentsService`, `PERMITS`:
   the wire. `begin` and `send` upload by hash; `attach`, `detach` and `list`
   handle a record's list; `size` and `read` serve the bytes; `renditions`,
   `ticket` and `usage` are the rest.
4. `features/attachments/src/client.rs` `AttachmentsClient` (`push`, `read`),
   `attachment_of`, `hash_of`: wasm-clean.
5. `features/attachments/src/blob.rs` `attach_local`,
   `AttachmentsClient::push_doc`, `AttachmentsRemote`: a device's side over
   `architect::blob`.
6. `features/attachments/src/server/mod.rs` `AttachmentsServer` (`new`,
   `with_docs`, `with_renditions`, `ticket_key`, `set_quota`, `held`,
   `release_unreferenced`), `DocSource`.
7. `features/attachments/src/server/acting.rs` `Acting`: the service for one
   caller in one org. Every call checks the parent through the engine, then
   reads or changes the org's ledger (`features/attachments/src/server/ledger.rs`).
8. `features/attachments/src/web.rs` `AttachmentsWeb`, `content_url`,
   `rendition_url`; `features/attachments/src/server/web.rs` `WebAttachments`.

A push calls `begin(parent, hash, size)`. The server checks `write` on the
parent and answers with how many bytes it has: all of them if the org's
store already holds the hash, otherwise what is staged. After that,
`send(parent, hash, offset, bytes)` appends chunks at the offset where the
upload stands. When the last chunk arrives, the server checks the bytes
against the hash as the blob store takes them, records them in the org's
ledger and asks for their renditions. A read checks `read` on the parent,
then that the parent points at the hash: the document's own map in the
server's copy, or the record's list.

## Tests to read

`features/attachments/tests/attachments.rs` runs over a real vox link,
through the gate and the composite engine (org roles plus spaces), with a
real socket for the byte face. The tests cover:

- A note attaches an image and a PDF offline. It syncs and pushes, and a
  second device fetches both by hash and collects what it no longer lists.
- A viewer reads but cannot attach. Knowing a hash reads nothing. A person
  from another org reads the note's attachments once a space shares the note,
  and stops on her next call after she is removed.
- An upload resumes and is refused at the wrong offset. The same bytes are
  sent once and counted once. The quota refuses before a byte is sent, and
  bytes that do not match their hash start again.
- A record keeps its list across a restart. A detached blob that nothing
  holds is released.
- A browser reads an attachment by range with a ticket for its parent and
  nothing else. A page someone attached is served as bytes, and a video's
  poster is made and served.

## Status

Done in architect. Not yet:

- a wasm store for the browser, so a web app can attach offline (it can attach
  online through `AttachmentsClient::push` and `doc::attach`)
- previews of images and PDFs: the files feature's renditions cover video
  and audio
- moving Task's attachments over (TF2)
