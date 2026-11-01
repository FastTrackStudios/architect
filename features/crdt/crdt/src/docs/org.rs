//! [`OrgDocs`]: one org's documents on a device, kept connected to the
//! org's server; [`OrgSyncs`]: one of them per org.
//!
//! Signed in, a device keeps a driver per org that owns something on it
//! — the person's own org, an org like Fast Track Audio for the projects
//! it owns — each with its own device index and its own [`Placement`].
//! The connection is a loop: dial the org's lane, run a session until
//! the connection closes, wait ([`Backoff`]), dial again. Every session
//! is a delta, so an outage costs nothing: offline, every doc edits
//! locally (each step durable in its file), and the next session sends
//! what the server lacks and merges what it has. Signed out, the dial
//! says [`Dialed::Stop`] and the loop ends; the files keep the docs.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use architect::Backoff;
use architect_orgs::{Org, OrgId, Orgs};
use tokio::sync::watch;

use super::DocsError;
use super::driver::{DocsDriver, DocsIndex, DocsLane};
use super::placement::Placement;

/// What dialing an org's lane found.
pub enum Dialed {
    /// The org's lane, connected (`/org/<slug>/vox` on Task's server).
    Lane(DocsLane),
    /// Not reached this time — offline, the server down, a handshake
    /// failing: tried again after a wait.
    Unreachable(String),
    /// Stop trying: signed out, or no longer in the org. The loop ends;
    /// [`OrgSyncs::ensure`] starts it again.
    Stop,
}

/// Where an org's connection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrgLink {
    /// Dialing.
    Connecting,
    /// A session is running.
    Live,
    /// Between two tries: everything edits locally meanwhile.
    Offline,
    /// The loop ended ([`Dialed::Stop`]) or never started.
    Stopped,
}

/// One org's documents on this device: its [`DocsDriver`] (the device's
/// index of the org's docs, and their sync) and its [`Placement`] (each
/// doc's file). Cheap to clone; clones share it.
///
/// Open and make docs through [`Self::driver`] (placing a doc's file
/// first with [`Placement::place`] when the app keeps it somewhere of its
/// own); [`Self::stay_connected`] keeps them in step with the org's
/// server.
#[derive(Clone)]
pub struct OrgDocs {
    org: Org,
    driver: DocsDriver,
    placement: Placement,
    link: Arc<watch::Sender<OrgLink>>,
}

impl OrgDocs {
    /// `org`'s documents, indexed by `index` and kept where `placement`
    /// says. Two processes on one device must not share an index (each
    /// writes it): name it for the process's role.
    pub fn new(org: Org, index: impl DocsIndex, placement: Placement) -> Self {
        let driver = DocsDriver::new(index, placement.clone());
        Self {
            org,
            driver,
            placement,
            link: Arc::new(watch::Sender::new(OrgLink::Stopped)),
        }
    }

    #[must_use]
    pub const fn org(&self) -> &Org {
        &self.org
    }

    #[must_use]
    pub const fn driver(&self) -> &DocsDriver {
        &self.driver
    }

    #[must_use]
    pub const fn placement(&self) -> &Placement {
        &self.placement
    }

    /// Where the connection is now.
    #[must_use]
    pub fn link(&self) -> OrgLink {
        *self.link.borrow()
    }

    /// Every change of [`Self::link`].
    #[must_use]
    pub fn watch_link(&self) -> watch::Receiver<OrgLink> {
        self.link.subscribe()
    }

    fn set_link(&self, link: OrgLink) {
        self.link.send_replace(link);
    }

    /// One session on `lane`, until its connection closes (any of the
    /// lane's clients' `Caller::closed`) or the session ends. What an app
    /// whose connection is already supervised (`use_app_supervised`)
    /// runs once per `Connection::generation`.
    ///
    /// # Errors
    ///
    /// The session's: a transport failure, or a session of this org
    /// already running.
    pub async fn run_on(&self, lane: &DocsLane) -> Result<(), DocsError> {
        self.set_link(OrgLink::Live);
        let ended = tokio::select! {
            res = self.driver.run(lane) => res,
            () = closed(lane) => Ok(()),
        };
        self.set_link(OrgLink::Offline);
        ended
    }

    /// Stay connected, for as long as the returned future runs or until
    /// `dial` says [`Dialed::Stop`]: dial, run a session until the
    /// connection closes, wait under a [`Backoff`] (exponential, full
    /// jitter, 250 ms to 10 s, reset by every connection), dial again.
    ///
    /// Logs a line per change, never per try: one when a live connection
    /// is lost, one for the first failed try of an outage, one when it
    /// is back.
    pub async fn stay_connected<D, F>(&self, dial: D)
    where
        D: Fn(Org) -> F,
        F: Future<Output = Dialed>,
    {
        self.stay_connected_with(dial, Backoff::default()).await;
    }

    /// [`Self::stay_connected`] under `backoff`.
    pub async fn stay_connected_with<D, F>(&self, dial: D, mut backoff: Backoff)
    where
        D: Fn(Org) -> F,
        F: Future<Output = Dialed>,
    {
        let slug = self.org.slug.clone();
        let mut outage = false;
        loop {
            self.set_link(OrgLink::Connecting);
            match dial(self.org.clone()).await {
                Dialed::Lane(lane) => {
                    backoff.reset();
                    tracing::info!(sync.org = %slug, sync.outcome = "connected", "org sync: connected");
                    outage = false;
                    let ended = self.run_on(&lane).await;
                    let why = ended.err().map(|e| e.to_string()).unwrap_or_default();
                    tracing::warn!(sync.org = %slug, sync.outcome = "dropped", error = %why, "org sync: the connection was lost");
                }
                Dialed::Unreachable(why) => {
                    if outage {
                        tracing::debug!(sync.org = %slug, sync.outcome = "unreachable", error = %why, "org sync: still not reached");
                    } else {
                        tracing::warn!(sync.org = %slug, sync.outcome = "unreachable", error = %why, "org sync: the server was not reached");
                        outage = true;
                    }
                }
                Dialed::Stop => {
                    tracing::info!(sync.org = %slug, sync.outcome = "stopped", "org sync: stopped");
                    self.set_link(OrgLink::Stopped);
                    return;
                }
            }
            self.set_link(OrgLink::Offline);
            tokio::time::sleep(backoff.next_delay()).await;
        }
    }
}

/// Resolves when any of the lane's connections closes.
async fn closed(lane: &DocsLane) {
    tokio::select! {
        () = lane.docs.caller.closed() => {}
        () = lane.changes.caller.closed() => {}
        () = lane.sync.caller.closed() => {}
    }
}

/// A boxed future, as [`OrgSyncs`]' callbacks are kept.
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

type Open = dyn Fn(Org) -> BoxFuture<Result<OrgDocs, DocsError>> + Send + Sync;
type Dial = dyn Fn(Org) -> BoxFuture<Dialed> + Send + Sync;

/// An org's driver and its connection loop.
struct Running {
    docs: OrgDocs,
    connection: tokio::task::JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.connection.abort();
    }
}

/// One [`OrgDocs`] per org, each kept connected for as long as it is
/// held here. Cheap to clone; clones share the drivers.
///
/// `open` makes an org's driver the first time it is asked for (its
/// index and its placement are the app's: a Turso index beside the
/// app's other files, documents under the org's own directory); `dial`
/// reaches the org's lane, as often as the loop needs it — with the
/// session token as it is at that moment, since a sign-in elsewhere may
/// have renewed it.
#[derive(Clone)]
pub struct OrgSyncs {
    open: Arc<Open>,
    dial: Arc<Dial>,
    orgs: Arc<tokio::sync::Mutex<HashMap<OrgId, Running>>>,
}

impl OrgSyncs {
    pub fn new<O, OF, D, DF>(open: O, dial: D) -> Self
    where
        O: Fn(Org) -> OF + Send + Sync + 'static,
        OF: Future<Output = Result<OrgDocs, DocsError>> + Send + 'static,
        D: Fn(Org) -> DF + Send + Sync + 'static,
        DF: Future<Output = Dialed> + Send + 'static,
    {
        Self {
            open: Arc::new(move |org| Box::pin(open(org))),
            dial: Arc::new(move |org| Box::pin(dial(org))),
            orgs: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    /// `org`'s driver: made now and connected, or the one already made
    /// (connected again if its loop had stopped). Two first asks at once
    /// make one driver.
    ///
    /// # Errors
    ///
    /// `open`'s, when the driver is made now.
    // The map's lock is held across `open`: that is what makes two first
    // asks make one driver.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn ensure(&self, org: &Org) -> Result<OrgDocs, DocsError> {
        let mut orgs = self.orgs.lock().await;
        if let Some(running) = orgs.get_mut(&org.id) {
            if running.connection.is_finished() {
                running.connection = self.connect(&running.docs);
            }
            return Ok(running.docs.clone());
        }
        let docs = (self.open)(org.clone()).await?;
        tracing::info!(sync.org = %org.slug, "org sync: on");
        let connection = self.connect(&docs);
        orgs.insert(
            org.id.clone(),
            Running {
                docs: docs.clone(),
                connection,
            },
        );
        Ok(docs)
    }

    fn connect(&self, docs: &OrgDocs) -> tokio::task::JoinHandle<()> {
        let docs = docs.clone();
        let dial = self.dial.clone();
        tokio::spawn(async move { docs.stay_connected(|org| dial(org)).await })
    }

    /// `org`'s driver, if one is held.
    pub async fn get(&self, org: &OrgId) -> Option<OrgDocs> {
        self.orgs.lock().await.get(org).map(|r| r.docs.clone())
    }

    /// The driver of the org the window is in ([`Orgs::current`]), made
    /// if need be; `None` in All, where nothing is written.
    ///
    /// # Errors
    ///
    /// [`Self::ensure`]'s.
    pub async fn current(&self, orgs: &Orgs) -> Result<Option<OrgDocs>, DocsError> {
        match orgs.current() {
            Some(org) => self.ensure(org).await.map(Some),
            None => Ok(None),
        }
    }

    /// Follow the person's orgs as they are now (a fresh list from the
    /// identity server): an org they are no longer in stops syncing and
    /// its driver is let go (its files stay), and the current org's is
    /// made. Returns the current org's driver.
    ///
    /// # Errors
    ///
    /// [`Self::ensure`]'s.
    pub async fn follow(&self, orgs: &Orgs) -> Result<Option<OrgDocs>, DocsError> {
        let mut held = self.orgs.lock().await;
        let left: Vec<OrgId> = held
            .keys()
            .filter(|id| orgs.get(id).is_none())
            .cloned()
            .collect();
        let gone: Vec<Running> = left.iter().filter_map(|id| held.remove(id)).collect();
        drop(held);
        for running in &gone {
            tracing::info!(sync.org = %running.docs.org.slug, sync.outcome = "left", "org sync: no longer in the org — stopped");
            running.docs.set_link(OrgLink::Stopped);
        }
        drop(gone);
        self.current(orgs).await
    }

    /// Stop `org`'s sync and let its driver go. Whether one was held.
    pub async fn stop(&self, org: &OrgId) -> bool {
        let running = self.orgs.lock().await.remove(org);
        running.is_some_and(|r| {
            r.docs.set_link(OrgLink::Stopped);
            true
        })
    }

    /// The orgs a driver is held for.
    pub async fn orgs(&self) -> Vec<OrgId> {
        self.orgs.lock().await.keys().cloned().collect()
    }
}
