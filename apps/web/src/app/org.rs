//! The signed-in account's org vaults, over the node (#400): the node's
//! org lane (`/org/{slug}/vox` on this origin) with the session as the
//! bearer (the WebSocket's `vox.bearer.<token>` subprotocol, never the
//! URL). Its `Notes` (`vault_proto::notes`) are listed into the app's
//! `MemorySource` (each vault a folder) and read in the background.
//!
//! The bearer is the issuer's short-lived access token (client
//! `architect`): the node asks the issuer's `whoami` and org list about
//! it, and verifies nothing itself. The lane is dialled again with each
//! renewed token ([`OrgVaults::redial`]); every open note's session moves
//! to the new lane.
//!
//! An open note is live: its body on the lane's `DocSync`
//! (`crdt::sync::SyncedDoc` on `vault_proto::collab_doc_id`), so typing
//! here reaches the node's document and file, and another's typing shows
//! here as it comes ([`LiveNote`]). A new note is written with
//! `Notes::write` before it opens. Renaming, moving and deleting notes,
//! and their tags and properties, are not sent to the node yet: the
//! navigator's layout is read-only here ([`NodeLayout`]) and says so.
//!
//! Kept in the tab's memory only: a reload lists and reads again (no
//! IndexedDB/OPFS copy yet). What fails is reported ([`OrgVaults::problems`]).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use architect::connect::{Endpoint, Pool};
use architect_navigator::markdown::note_from_markdown;
use architect_navigator::{MemorySource, Millis, NavigatorSource as _, NoteFile};
use crdt::CrdtDoc;
use crdt::sync::{DocSyncClient, SyncedDoc};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt as _;
use futures_util::future::{Either, select};
use vault_proto::notes::{NoteRef, NotesClient};
use wasm_bindgen_futures::spawn_local;

/// How long a dropped note session waits before it attaches again, ms.
const RETRY_MS: u32 = 2_000;

/// The org lane on this origin: `wss://<host>/org/<slug>/vox`.
#[must_use]
pub fn lane_url(slug: &str) -> Option<String> {
    let location = web_sys::window()?.location();
    let scheme = if location.protocol().ok()? == "https:" {
        "wss"
    } else {
        "ws"
    };
    Some(format!(
        "{scheme}://{}/org/{slug}/vox",
        location.host().ok()?
    ))
}

/// The lane's clients, as dialled with one token.
#[derive(Clone)]
struct Lane {
    notes: NotesClient,
    sync: DocSyncClient,
}

async fn dial(slug: &str, token: String) -> Result<Lane, String> {
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
    Ok(Lane { notes, sync })
}

struct Inner {
    slug: String,
    lane: RefCell<Lane>,
    /// Each open note's session, woken when the lane is dialled again.
    redialled: RefCell<Vec<UnboundedSender<()>>>,
    /// Whoever shows what failed.
    problems: RefCell<Vec<UnboundedSender<String>>>,
    /// The vaults, in the order listed.
    vaults: RefCell<Vec<String>>,
}

/// The org's vaults over its lane.
#[derive(Clone)]
pub struct OrgVaults(Rc<Inner>);

impl OrgVaults {
    /// Dials the lane as the person with `token`.
    ///
    /// # Errors
    ///
    /// The lane cannot be reached, or refuses the session.
    pub async fn open(slug: &str, token: String) -> Result<Self, String> {
        let lane = dial(slug, token).await?;
        Ok(Self(Rc::new(Inner {
            slug: slug.to_owned(),
            lane: RefCell::new(lane),
            redialled: RefCell::default(),
            problems: RefCell::default(),
            vaults: RefCell::default(),
        })))
    }

    /// Dials the lane again with a renewed `token`: what comes next goes
    /// over it, and every open note's session attaches again on it.
    ///
    /// # Errors
    ///
    /// The lane cannot be reached with it; the old lane stays.
    pub async fn redial(&self, token: String) -> Result<(), String> {
        let lane = dial(&self.0.slug, token).await?;
        *self.0.lane.borrow_mut() = lane;
        self.0
            .redialled
            .borrow_mut()
            .retain(|tx| tx.unbounded_send(()).is_ok());
        Ok(())
    }

    fn notes(&self) -> NotesClient {
        self.0.lane.borrow().notes.clone()
    }

    /// What fails from now on, one message each.
    #[must_use]
    pub fn problems(&self) -> UnboundedReceiver<String> {
        let (tx, rx) = unbounded();
        self.0.problems.borrow_mut().push(tx);
        rx
    }

    /// Says what failed to whoever shows it.
    pub fn report(&self, problem: impl Into<String>) {
        let problem = problem.into();
        self.0
            .problems
            .borrow_mut()
            .retain(|tx| tx.unbounded_send(problem.clone()).is_ok());
    }

    /// Every note of the org's vaults, as a vault of folders (one per
    /// vault), their texts read after (`fill`).
    ///
    /// # Errors
    ///
    /// The node refused the listing (not a member).
    pub async fn list(
        &self,
        name: &str,
        now: Millis,
    ) -> Result<(MemorySource, Vec<NoteRef>), String> {
        let notes = self.notes().list().await.map_err(|e| e.to_string())?;
        let mut source = MemorySource::new(name);
        let mut files = Vec::new();
        let mut vaults: Vec<String> = Vec::new();
        for n in &notes {
            if !vaults.contains(&n.vault) {
                vaults.push(n.vault.clone());
            }
            source = source.with_folder(n.vault.clone());
            if let Some((folder, _)) = n.path.rsplit_once('/') {
                source = source.with_folder(format!("{}/{folder}", n.vault));
            }
            files.push(NoteFile::new(format!("{}/{}", n.vault, n.path), now, now));
        }
        *self.0.vaults.borrow_mut() = vaults;
        Ok((source.with_files(files), notes))
    }

    /// The vaults, as listed.
    #[must_use]
    pub fn vaults(&self) -> Vec<String> {
        self.0.vaults.borrow().clone()
    }

    /// Reads each note into `source`, one after another, so the navigator
    /// fills its previews, tags and tasks as they come. Notes that cannot
    /// be read are counted and reported once.
    pub fn fill(&self, source: MemorySource, notes: Vec<NoteRef>, now: Millis) {
        let this = self.clone();
        spawn_local(async move {
            let mut failed = 0_usize;
            let mut first = None;
            for n in notes {
                let path = format!("{}/{}", n.vault, n.path);
                if source.read_text(&path).is_some() {
                    continue;
                }
                match this.notes().read(n.vault, n.path).await {
                    Ok(text) => {
                        source.set_text(note_from_markdown(&path, &text, now, now), text);
                    }
                    Err(e) => {
                        failed = failed.saturating_add(1);
                        first.get_or_insert_with(|| format!("{path}: {e}"));
                    }
                }
            }
            if let Some(first) = first {
                this.report(format!(
                    "{failed} note{} could not be read ({first})",
                    if failed == 1 { "" } else { "s" }
                ));
            }
        });
    }

    /// Reads one note now (opening it before `fill` reached it).
    ///
    /// # Errors
    ///
    /// Not a vault's note, or the node did not answer it.
    pub async fn read(&self, path: &str) -> Result<String, String> {
        let (vault, rest) = path
            .split_once('/')
            .ok_or_else(|| format!("{path} is in no vault"))?;
        self.notes()
            .read(vault.to_owned(), rest.to_owned())
            .await
            .map_err(|e| e.to_string())
    }

    /// Makes a new empty note on the node, at the first of `paths`
    /// (`<vault>/<path in it>`) that no note holds: one made by someone
    /// else since the listing is never written over (`Notes::create`
    /// refuses it, and the next name is tried). Done before it opens, so
    /// its live document starts from the file. The path made.
    ///
    /// # Errors
    ///
    /// Not inside a vault, or the node refused the write.
    pub async fn create(&self, paths: impl IntoIterator<Item = String>) -> Result<String, String> {
        crate::create_first(paths, |path| async move {
            let (vault, rest) = path
                .split_once('/')
                .filter(|(v, r)| !v.is_empty() && !r.is_empty())
                .ok_or_else(|| format!("{path} is in no vault"))?;
            match self
                .notes()
                .create(vault.to_owned(), rest.to_owned(), String::new())
                .await
            {
                Ok(()) => Ok(true),
                Err(architect::vox::VoxError::User(e))
                    if matches!(*e, vault_proto::notes::NotesError::Exists(_)) =>
                {
                    Ok(false)
                }
                Err(e) => Err(e.to_string()),
            }
        })
        .await
    }

    /// The note at `path` kept live with the node while the returned
    /// [`LiveNote`] is kept; what happens to it comes on the receiver.
    /// The session attaches again after a drop, and on each re-dial.
    #[must_use]
    pub fn live(&self, path: &str) -> Option<(LiveNote, UnboundedReceiver<Heard>)> {
        let (vault, rest) = path.split_once('/')?;
        let replica = CrdtDoc::ephemeral();
        let (tx, rx) = unbounded::<Heard>();
        let (imported, attached) = (tx.clone(), tx.clone());
        let mut synced = SyncedDoc::new(vault_proto::collab_doc_id(vault, rest), replica.clone())
            .on_import(Arc::new(move |_: &[u8]| {
                let _ = imported.unbounded_send(Heard::Changed);
            }))
            .on_attached(Arc::new(move || {
                let _ = attached.unbounded_send(Heard::Attached);
            }));
        let outbox = synced.outbox();
        let (stop, mut stopped) = futures_channel::oneshot::channel::<()>();
        let (redial_tx, mut redialled) = unbounded::<()>();
        self.0.redialled.borrow_mut().push(redial_tx);
        let this = self.clone();
        spawn_local(async move {
            loop {
                let client = this.0.lane.borrow().sync.clone();
                let ended = {
                    let run = std::pin::pin!(synced.run(&client));
                    let interrupted = std::pin::pin!(select(&mut stopped, redialled.next()));
                    match select(run, interrupted).await {
                        Either::Left((ended, _)) => Some(ended),
                        // Closed: the note is gone.
                        Either::Right((Either::Left(_), _)) => return,
                        // A new lane: attach on it now.
                        Either::Right((Either::Right(_), _)) => None,
                    }
                };
                if let Some(ended) = ended {
                    let why = match ended {
                        Ok(()) => "the node ended the session".to_owned(),
                        Err(e) => e.to_string(),
                    };
                    if tx.unbounded_send(Heard::Lost(why)).is_err() {
                        return;
                    }
                    let wait = gloo_timers::future::TimeoutFuture::new(RETRY_MS);
                    let interrupted = select(&mut stopped, redialled.next());
                    if let Either::Right((Either::Left(_), _)) = select(wait, interrupted).await {
                        return;
                    }
                }
            }
        });
        Some((
            LiveNote {
                replica,
                outbox,
                _stop: stop,
            },
            rx,
        ))
    }
}

/// What an open note's session says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heard {
    /// Caught up with the node: the document holds what the node holds
    /// (the first time, the note may be edited).
    Attached,
    /// Something arrived from the node.
    Changed,
    /// The session ended (it attaches again shortly).
    Lost(String),
}

/// An open note's body, live with the node; dropped, its session ends.
pub struct LiveNote {
    replica: CrdtDoc,
    /// The session's outbox: what reaches the replica by import (a merge,
    /// `crate::live_text::merge_at`) is sent up through it.
    outbox: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    _stop: futures_channel::oneshot::Sender<()>,
}

impl LiveNote {
    /// The replica's Loro document (`crate::live_text::LiveText` edits it).
    #[must_use]
    pub fn doc(&self) -> crdt::loro::LoroDoc {
        self.replica.loro().clone()
    }

    /// Sends `update` to the node: a merged edit, which reached the
    /// replica by import, so no local-update subscription sent it.
    pub fn relay(&self, update: Vec<u8>) {
        let _ = self.outbox.send(update);
    }
}

pub use crate::read_only::NodeLayout;
