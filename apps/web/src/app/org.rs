//! The signed-in account's org vaults, over the node (#400): the node's
//! org lane (`/org/{slug}/vox` on this origin) with the session as the
//! bearer (the WebSocket's `vox.bearer.<token>` subprotocol, never the
//! URL). Its `Notes` (`vault_proto::notes`) are listed into the app's
//! `MemorySource` (each vault a folder), read in the background, and an
//! edit is written back when the typing rests.
//!
//! An open note is live: its body on the lane's `DocSync`
//! (`crdt::sync::SyncedDoc` on `vault_proto::collab_doc_id`), so typing
//! here reaches the node's document and file, and another's typing shows
//! here as it comes ([`LiveNote`]).
//!
//! Kept in the tab's memory only: a reload lists and reads again (no
//! IndexedDB/OPFS copy yet), and with the node out of reach nothing is
//! shown but why.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use std::sync::Arc;

use architect::connect::{Endpoint, Pool};
use crdt::CrdtDoc;
use crdt::sync::{DocSyncClient, SyncedDoc};
use futures_channel::mpsc::UnboundedReceiver;
use architect_navigator::markdown::note_from_markdown;
use architect_navigator::{MemorySource, Millis, NavigatorSource as _, NoteFile};
use vault_proto::notes::{NoteRef, NotesClient};
use wasm_bindgen_futures::spawn_local;

/// How long typing rests before it is written to the node, ms.
const WRITE_AFTER_MS: u32 = 600;

/// The org lane on this origin: `wss://<host>/org/<slug>/vox`.
#[must_use]
pub fn lane_url(slug: &str) -> Option<String> {
    let location = web_sys::window()?.location();
    let scheme = if location.protocol().ok()? == "https:" {
        "wss"
    } else {
        "ws"
    };
    Some(format!("{scheme}://{}/org/{slug}/vox", location.host().ok()?))
}

/// The org's vaults over its lane.
#[derive(Clone)]
pub struct OrgVaults {
    notes: NotesClient,
    sync: DocSyncClient,
    /// Each pending write's turn: the latest typing wins, written once it
    /// rests.
    pending: Rc<RefCell<HashMap<String, u64>>>,
}

impl OrgVaults {
    /// Dials the lane as the person with `token`.
    ///
    /// # Errors
    ///
    /// The lane cannot be reached, or refuses the session.
    pub async fn open(slug: &str, token: String) -> Result<Self, String> {
        let url = lane_url(slug).ok_or("no address for the org's lane")?;
        let at = Endpoint::url(url).with_bearer(token);
        let pool = Pool::new();
        let notes: NotesClient = pool
            .client(&at)
            .await
            .map_err(|e| format!("the org's lane: {e}"))?;
        let sync: DocSyncClient = pool
            .client(&at)
            .await
            .map_err(|e| format!("the org's lane: {e}"))?;
        Ok(Self {
            notes,
            sync,
            pending: Rc::default(),
        })
    }

    /// Every note of the org's vaults, as a vault of folders (one per
    /// vault), their texts read after (`fill`).
    ///
    /// # Errors
    ///
    /// The node refused the listing (not a member).
    pub async fn list(&self, name: &str, now: Millis) -> Result<(MemorySource, Vec<NoteRef>), String> {
        let notes = self.notes.list().await.map_err(|e| e.to_string())?;
        let mut source = MemorySource::new(name);
        let mut files = Vec::new();
        for n in &notes {
            source = source.with_folder(n.vault.clone());
            if let Some((folder, _)) = n.path.rsplit_once('/') {
                source = source.with_folder(format!("{}/{folder}", n.vault));
            }
            files.push(NoteFile::new(format!("{}/{}", n.vault, n.path), now, now));
        }
        Ok((source.with_files(files), notes))
    }

    /// Reads each note into `source`, one after another, so the navigator
    /// fills its previews, tags and tasks as they come.
    pub fn fill(&self, source: MemorySource, notes: Vec<NoteRef>, now: Millis) {
        let client = self.notes.clone();
        spawn_local(async move {
            for n in notes {
                let path = format!("{}/{}", n.vault, n.path);
                if source.read_text(&path).is_some() {
                    continue;
                }
                if let Ok(text) = client.read(n.vault, n.path).await {
                    source.set_text(note_from_markdown(&path, &text, now, now), text);
                }
            }
        });
    }

    /// Reads one note now (opening it before `fill` reached it).
    pub async fn read(&self, path: &str) -> Option<String> {
        let (vault, rest) = path.split_once('/')?;
        self.notes
            .read(vault.to_owned(), rest.to_owned())
            .await
            .ok()
    }

    /// Writes `text` as the note at `path` once the typing rests: the
    /// latest of a run of edits, once.
    pub fn write_later(&self, path: &str, text: String) {
        let Some((vault, rest)) = path.split_once('/') else {
            return;
        };
        let (vault, rest) = (vault.to_owned(), rest.to_owned());
        let turn = {
            let mut pending = self.pending.borrow_mut();
            let turn = pending.get(path).copied().unwrap_or(0).wrapping_add(1);
            pending.insert(path.to_owned(), turn);
            turn
        };
        let (client, pending, key) = (self.notes.clone(), Rc::clone(&self.pending), path.to_owned());
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(WRITE_AFTER_MS).await;
            if pending.borrow().get(&key).copied() != Some(turn) {
                return;
            }
            pending.borrow_mut().remove(&key);
            let _ = client.write(vault, rest, text).await;
        });
    }
}

impl OrgVaults {
    /// The note at `path` kept live with the node while the returned
    /// [`LiveNote`] is kept; each change that reaches it (the first catch-up
    /// too) wakes the receiver.
    #[must_use]
    pub fn live(&self, path: &str) -> Option<(LiveNote, UnboundedReceiver<()>)> {
        let (vault, rest) = path.split_once('/')?;
        let replica = CrdtDoc::ephemeral();
        let (tx, rx) = futures_channel::mpsc::unbounded::<()>();
        let attached = tx.clone();
        let mut synced = SyncedDoc::new(vault_proto::collab_doc_id(vault, rest), replica.clone())
            .on_import(Arc::new(move |_: &[u8]| {
                let _ = tx.unbounded_send(());
            }))
            .on_attached(Arc::new(move || {
                let _ = attached.unbounded_send(());
            }));
        let (stop, stopped) = futures_channel::oneshot::channel::<()>();
        let client = self.sync.clone();
        spawn_local(async move {
            let run = std::pin::pin!(synced.run(&client));
            let _ = futures_util::future::select(run, stopped).await;
        });
        Some((
            LiveNote {
                replica,
                _stop: stop,
            },
            rx,
        ))
    }
}

/// An open note's body, live with the node; dropped, its session ends.
pub struct LiveNote {
    replica: CrdtDoc,
    _stop: futures_channel::oneshot::Sender<()>,
}

impl LiveNote {
    /// The body as it is now (the markdown after the frontmatter).
    #[must_use]
    pub fn body(&self) -> String {
        self.replica
            .loro()
            .get_text(vault_proto::COLLAB_BODY_CONTAINER)
            .to_string()
    }

    /// Makes the body read `new`: the span that changed replaced, one
    /// commit, sent to the node.
    pub fn set_body(&self, new: &str) {
        let text = self.replica.loro().get_text(vault_proto::COLLAB_BODY_CONTAINER);
        let old: Vec<char> = text.to_string().chars().collect();
        let new: Vec<char> = new.chars().collect();
        let start = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let tail = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(old.len().min(new.len()).saturating_sub(start))
            .take_while(|(a, b)| a == b)
            .count();
        let removed = old.len().saturating_sub(start).saturating_sub(tail);
        let added: String = new
            .get(start..new.len().saturating_sub(tail))
            .unwrap_or_default()
            .iter()
            .collect();
        if removed == 0 && added.is_empty() {
            return;
        }
        if removed > 0 {
            let _ = text.delete(start, removed);
        }
        if !added.is_empty() {
            let _ = text.insert(start, &added);
        }
        self.replica.loro().commit();
    }
}
