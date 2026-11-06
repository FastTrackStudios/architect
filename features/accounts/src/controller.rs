//! [`Accounts`]: the controller an app and its actions drive (feature
//! `http`). Thread-safe, so an action off the window's thread reads and
//! changes it directly; the UI hears each change ([`Accounts::on_change`])
//! and draws from [`Accounts::snapshot`] and the sheet's state.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use architect_orgs::Orgs;
use auth_client::StoredSession;

use crate::issuer::{self, Issuer, IssuerError, Polled, SignedIn};
use crate::model::{Account, AccountsFile, Device};
use crate::store::{AccountStore, ActiveSession};

/// A session that ends within this long is renewed at start.
const RENEW_WITHIN_SECS: i64 = 24 * 60 * 60;

/// What the sheet shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    /// Signing in: in the browser, or with an email and password.
    SignIn,
    /// The accounts on this device, and the active one's devices.
    Accounts,
}

/// The last thing a flow said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    /// Under way: what.
    Busy(String),
    /// Went wrong: why, for the person.
    Failed(String),
    /// Done: what happened.
    Done(String),
}

/// A browser sign-in waiting for its person's approval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserWait {
    /// The code the approval page shows.
    pub user_code: String,
    /// The approval page.
    pub url: String,
    /// Whether the browser opened it (else the person opens `url`).
    pub opened: bool,
    attempt: u64,
}

#[derive(Default)]
struct State {
    sheet: Option<Sheet>,
    status: Status,
    browser: Option<BrowserWait>,
    devices: Option<Result<Vec<Device>, String>>,
    /// Why the orgs shown are the cached list: the last fetch's failure.
    orgs_stale: Option<String>,
    attempt: u64,
}

type Listener = Box<dyn Fn() + Send + Sync>;

struct Inner {
    store: Arc<AccountStore>,
    issuer: Issuer,
    client_id: String,
    open_browser: bool,
    state: Mutex<State>,
    /// Held while a session is renewed, and by every call that presents
    /// one: a renewal ends the old token at the issuer, so nothing may use
    /// it meanwhile.
    sessions: tokio::sync::Mutex<()>,
    rev: AtomicU64,
    listeners: Mutex<Vec<Listener>>,
}

/// The accounts on this device and the flows that change them.
#[derive(Clone)]
pub struct Accounts(Arc<Inner>);

impl std::fmt::Debug for Accounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Accounts")
            .field("issuer", &self.0.issuer)
            .field("store", &self.0.store)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Accounts {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Accounts {
    /// The accounts in `store`, signing in at `issuer` as `client_id` (what
    /// the issuer's approval page names the app: `architect-desktop`).
    #[must_use]
    pub fn new(store: Arc<AccountStore>, issuer: Issuer, client_id: impl Into<String>) -> Self {
        Self(Arc::new(Inner {
            store,
            issuer,
            client_id: client_id.into(),
            open_browser: true,
            state: Mutex::new(State::default()),
            sessions: tokio::sync::Mutex::new(()),
            rev: AtomicU64::new(0),
            listeners: Mutex::new(Vec::new()),
        }))
    }

    /// The same, never opening a browser itself: the sheet shows the link
    /// (a test, a session with no browser).
    #[must_use]
    pub fn without_browser(self) -> Self {
        match Arc::try_unwrap(self.0) {
            Ok(mut inner) => {
                inner.open_browser = false;
                Self(Arc::new(inner))
            }
            Err(shared) => Self(shared),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The store underneath.
    #[must_use]
    pub fn store(&self) -> &Arc<AccountStore> {
        &self.0.store
    }

    /// The issuer it signs in at.
    #[must_use]
    pub fn issuer(&self) -> &Issuer {
        &self.0.issuer
    }

    /// The active account's session, following switches: for an org list
    /// or a vox client.
    #[must_use]
    pub fn active_session(&self) -> ActiveSession {
        ActiveSession(self.0.store.clone())
    }

    /// Bumped on every change.
    #[must_use]
    pub fn rev(&self) -> u64 {
        self.0.rev.load(Ordering::SeqCst)
    }

    /// Calls `listener` (from any thread) after every change.
    pub fn on_change(&self, listener: impl Fn() + Send + Sync + 'static) {
        self.0
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Box::new(listener));
    }

    fn changed(&self) {
        self.0.rev.fetch_add(1, Ordering::SeqCst);
        for listener in self
            .0
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            listener();
        }
    }

    fn set_status(&self, status: Status) {
        self.state().status = status;
        self.changed();
    }

    /// The accounts and the active one.
    #[must_use]
    pub fn snapshot(&self) -> AccountsFile {
        self.0.store.snapshot()
    }

    /// The active account.
    #[must_use]
    pub fn active(&self) -> Option<Account> {
        self.0.store.active()
    }

    /// What the sheet shows, `None` when it is closed.
    #[must_use]
    pub fn sheet(&self) -> Option<Sheet> {
        self.state().sheet
    }

    /// The last thing a flow said.
    #[must_use]
    pub fn status(&self) -> Status {
        self.state().status.clone()
    }

    /// The browser sign-in waiting, if one is.
    #[must_use]
    pub fn browser(&self) -> Option<BrowserWait> {
        self.state().browser.clone()
    }

    /// The active account's devices as last fetched.
    #[must_use]
    pub fn devices(&self) -> Option<Result<Vec<Device>, String>> {
        self.state().devices.clone()
    }

    /// Opens the sheet on `page` (the accounts page fetches the devices).
    pub fn open_sheet(&self, page: Sheet) {
        {
            let mut state = self.state();
            state.sheet = Some(page);
            state.status = Status::Idle;
        }
        self.changed();
        if page == Sheet::Accounts {
            let me = self.clone();
            issuer::spawn(async move {
                let _ = me.refresh_devices().await;
            });
        }
    }

    /// The devices shown follow the active account: fetched again while the
    /// accounts page is open.
    fn devices_follow(&self) {
        let open = self.state().sheet == Some(Sheet::Accounts);
        if open && self.active().is_some() {
            let me = self.clone();
            issuer::spawn(async move {
                let _ = me.refresh_devices().await;
            });
        }
    }

    /// Closes the sheet; a browser sign-in waiting stops.
    pub fn close_sheet(&self) {
        {
            let mut state = self.state();
            state.sheet = None;
            state.browser = None;
            state.status = Status::Idle;
        }
        self.changed();
    }

    /// Stops a browser sign-in waiting, the sheet staying open.
    pub fn cancel_browser(&self) {
        {
            let mut state = self.state();
            state.browser = None;
            state.status = Status::Idle;
        }
        self.changed();
    }

    fn adopt(&self, signed: SignedIn) -> Result<Account, String> {
        let SignedIn { account, session } = signed;
        self.0
            .store
            .sign_in(account.clone(), &session)
            .map_err(|e| e.to_string())?;
        {
            let mut state = self.state();
            state.sheet = None;
            state.browser = None;
            state.devices = None;
            state.status = Status::Done(format!("Signed in as {}", account.label()));
        }
        self.changed();
        // This device on the account's list; best effort.
        let issuer = self.0.issuer.clone();
        issuer::spawn(async move {
            let _ = issuer.register_this_device(&session).await;
        });
        self.orgs_follow();
        Ok(account)
    }

    /// Signs in with an email and password; the account becomes active.
    ///
    /// # Errors
    ///
    /// Why not, for the person (also the sheet's status).
    pub async fn sign_in_password(&self, email: &str, password: &str) -> Result<Account, String> {
        if email.trim().is_empty() || password.is_empty() {
            let why = "Type your email and password.".to_owned();
            self.set_status(Status::Failed(why.clone()));
            return Err(why);
        }
        self.set_status(Status::Busy("Signing in…".to_owned()));
        match self.0.issuer.sign_in_password(email, password).await {
            Ok(signed) => self.adopt(signed).inspect_err(|e| {
                self.set_status(Status::Failed(e.clone()));
            }),
            Err(e) => {
                let why = sentence(&e);
                self.set_status(Status::Failed(why.clone()));
                Err(why)
            }
        }
    }

    /// Starts a browser sign-in and waits for it in the background: the
    /// approval page opens (or its link shows), and the account becomes
    /// active once its person approves.
    pub fn sign_in_browser(&self) {
        let attempt = {
            let mut state = self.state();
            state.attempt = state.attempt.wrapping_add(1);
            state.status = Status::Busy("Asking for a code…".to_owned());
            state.attempt
        };
        self.changed();
        let me = self.clone();
        issuer::spawn(async move { me.browser_flow(attempt).await });
    }

    fn still(&self, attempt: u64) -> bool {
        let state = self.state();
        state.attempt == attempt && state.browser.as_ref().is_some_and(|b| b.attempt == attempt)
    }

    async fn browser_flow(&self, attempt: u64) {
        let code = match self.0.issuer.start_browser(&self.0.client_id).await {
            Ok(code) => code,
            Err(e) => return self.set_status(Status::Failed(sentence(&e))),
        };
        let opened = self.0.open_browser && webbrowser::open(&code.url).is_ok();
        {
            let mut state = self.state();
            if state.attempt != attempt {
                return;
            }
            state.browser = Some(BrowserWait {
                user_code: code.user_code.clone(),
                url: code.url.clone(),
                opened,
                attempt,
            });
            state.status = Status::Busy("Waiting for you to approve it in the browser…".to_owned());
        }
        self.changed();
        let deadline = Instant::now()
            .checked_add(code.expires_in.max(Duration::from_secs(60)))
            .unwrap_or_else(Instant::now);
        let mut wait = code.interval;
        loop {
            tokio::time::sleep(wait).await;
            if !self.still(attempt) {
                return;
            }
            if Instant::now() > deadline {
                self.state().browser = None;
                return self.set_status(Status::Failed(sentence(&IssuerError::Expired)));
            }
            match self.0.issuer.poll_browser(&code.device_code).await {
                Ok(Polled::SlowDown) => {
                    wait = wait.saturating_add(auth_client::device::SLOW_DOWN_STEP);
                }
                Ok(Polled::Approved(signed)) => {
                    if self.still(attempt) {
                        let _ = self.adopt(*signed);
                    }
                    return;
                }
                // Not yet, or a blip: the next poll asks again.
                Ok(Polled::Waiting) | Err(IssuerError::Unreachable { .. }) => {}
                Err(e) => {
                    self.state().browser = None;
                    return self.set_status(Status::Failed(sentence(&e)));
                }
            }
        }
    }

    /// Makes `who` (a key, id or email) the active account; its session is
    /// checked with the issuer in the background.
    ///
    /// # Errors
    ///
    /// No such account.
    pub fn switch(&self, who: &str) -> Result<Account, String> {
        let account = self.0.store.switch(who).map_err(|e| e.to_string())?;
        {
            let mut state = self.state();
            state.devices = None;
            state.orgs_stale = None;
            state.status = Status::Done(format!("Switched to {}", account.label()));
        }
        self.changed();
        self.devices_follow();
        let me = self.clone();
        let key = account.key();
        issuer::spawn(async move { me.check(&key).await });
        Ok(account)
    }

    /// Checks `key`'s session with the issuer: renews one about to end,
    /// updates the profile, and marks an account the issuer refused as
    /// needing to sign in again. Unreachable is fine (offline).
    async fn check(&self, key: &str) {
        let held = self.0.sessions.lock().await;
        let Ok(Some(session)) = self.0.store.session(key) else {
            return;
        };
        let now = unix_now();
        let answer = if session.expires_within(now, RENEW_WITHIN_SECS) {
            self.0.issuer.refresh(&session.token).await
        } else {
            self.0.issuer.session(&session.token).await
        };
        match answer {
            Ok(signed) => {
                if signed.session.token != session.token {
                    let _ = self.0.store.update_session(key, &signed.session);
                }
                let _ = self.0.store.refresh_profile(&signed.account);
                self.changed();
                drop(held);
                if self.active().is_some_and(|a| a.key() == key) {
                    let _ = self.refresh_orgs().await;
                }
            }
            Err(e) if e.ends_session() => {
                let _ = self.0.store.expire(key);
                self.changed();
            }
            Err(_) => {}
        }
    }

    /// At start: checks the active account's session.
    pub fn start(&self) {
        if let Some(account) = self.active() {
            let me = self.clone();
            issuer::spawn(async move { me.check(&account.key()).await });
        }
    }

    /// Signs `who` (the active account when `None`) out of this device: its
    /// session ended at the issuer, then gone from the keychain.
    ///
    /// # Errors
    ///
    /// No such account; the keychain's.
    pub async fn sign_out(&self, who: Option<&str>) -> Result<Account, String> {
        let snapshot = self.snapshot();
        let account = who
            .map_or_else(
                || snapshot.active().cloned(),
                |who| snapshot.find(who).cloned(),
            )
            .ok_or_else(|| {
                who.map_or_else(
                    || "no account is signed in".to_owned(),
                    |w| format!("no account {w}"),
                )
            })?;
        let key = account.key();
        let held = self.0.sessions.lock().await;
        let told = match self.0.store.session(&key) {
            Ok(Some(session)) => self.0.issuer.sign_out(&session.token).await.is_ok(),
            _ => true,
        };
        self.0.store.remove(&key).map_err(|e| e.to_string())?;
        drop(held);
        {
            let mut state = self.state();
            state.devices = None;
            state.status = Status::Done(if told {
                format!("Signed out of {}", account.label())
            } else {
                format!(
                    "Signed out of {} on this device; {} couldn't be told, so the session ends when it expires",
                    account.label(),
                    self.0.issuer.origin()
                )
            });
        }
        self.changed();
        self.devices_follow();
        Ok(account)
    }

    /// Runs `call` with the active account's session, no renewal under way.
    async fn with_active<T, F, Fut>(&self, call: F) -> Result<T, String>
    where
        F: FnOnce(StoredSession) -> Fut,
        Fut: Future<Output = Result<T, IssuerError>>,
    {
        let _held = self.0.sessions.lock().await;
        let session = self
            .0
            .store
            .active_session()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no account is signed in".to_owned())?;
        call(session).await.map_err(|e| sentence(&e))
    }

    /// Fetches the active account's devices.
    ///
    /// # Errors
    ///
    /// Signed out; the issuer's.
    pub async fn refresh_devices(&self) -> Result<Vec<Device>, String> {
        let issuer = self.0.issuer.clone();
        let answer = self
            .with_active(|session| {
                let issuer = issuer.clone();
                async move { issuer.devices(&session).await }
            })
            .await;
        self.state().devices = Some(answer.clone());
        self.changed();
        answer
    }

    /// The active account's orgs and the one it is in now, as last
    /// fetched; signed out, only the person's own on this device.
    #[must_use]
    pub fn orgs(&self) -> Orgs {
        self.0.store.orgs()
    }

    /// Why the orgs shown are the cached list, if the last fetch failed.
    #[must_use]
    pub fn orgs_stale(&self) -> Option<String> {
        self.state().orgs_stale.clone()
    }

    /// Fetches the active account's orgs again in the background.
    fn orgs_follow(&self) {
        let me = self.clone();
        issuer::spawn(async move {
            let _ = me.refresh_orgs().await;
        });
    }

    /// Fetches the active account's orgs from the issuer and keeps them;
    /// the org it is in stays current unless it was taken away. A failure
    /// keeps the cached list ([`Self::orgs_stale`] says why).
    ///
    /// # Errors
    ///
    /// Signed out; the issuer's.
    pub async fn refresh_orgs(&self) -> Result<Orgs, String> {
        let Some(key) = self.active().map(|a| a.key()) else {
            return Err("no account is signed in".to_owned());
        };
        let issuer = self.0.issuer.clone();
        let answer = self
            .with_active(|session| {
                let issuer = issuer.clone();
                async move { issuer.organizations(&session).await }
            })
            .await;
        let kept = answer.and_then(|fresh| {
            self.0
                .store
                .set_orgs(&key, fresh)
                .map_err(|e| e.to_string())
        });
        self.state().orgs_stale = kept.as_ref().err().cloned();
        self.changed();
        kept
    }

    /// Moves the active account to org `who` (its id, slug or name, or
    /// `all`); kept for the account, and told to the issuer in the
    /// background as the session's active org.
    ///
    /// # Errors
    ///
    /// Signed out; no such org.
    pub fn switch_org(&self, who: &str) -> Result<Orgs, String> {
        let orgs = self.0.store.switch_org(who).map_err(|e| e.to_string())?;
        self.changed();
        if let Some(org) = orgs.current().map(|o| o.id.0.clone()) {
            let me = self.clone();
            issuer::spawn(async move {
                let issuer = me.0.issuer.clone();
                let _ = me
                    .with_active(
                        |session| async move { issuer.set_active_org(&session, &org).await },
                    )
                    .await;
            });
        }
        Ok(orgs)
    }

    /// Takes device `id` off the active account.
    ///
    /// # Errors
    ///
    /// Signed out; the issuer's.
    pub async fn forget_device(&self, id: &str) -> Result<(), String> {
        let issuer = self.0.issuer.clone();
        self.with_active(|session| {
            let (issuer, id) = (issuer.clone(), id.to_owned());
            async move { issuer.forget_device(&session, &id).await }
        })
        .await?;
        let _ = self.refresh_devices().await;
        Ok(())
    }
}

/// An issuer error as the sheet says it.
fn sentence(e: &IssuerError) -> String {
    let s = e.to_string();
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        format!("{}{}.", c.to_uppercase(), chars.as_str())
    })
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
