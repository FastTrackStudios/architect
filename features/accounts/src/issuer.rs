//! The issuer over its HTTP face (feature `http`): auth-proto's generated
//! `AuthServiceHttpClient` and `DeviceServiceHttpClient`, the same routes
//! the deployed server answers (`POST /auth/sign-in/email`,
//! `/auth/device/code`, `/auth/devices/list`, …). HTTP rather than the
//! issuer's vox lane: an app and the issuer need not run one vox version.
//!
//! The requests run on a runtime of their own (two threads,
//! `accounts-net`), whoever awaits them: the window's executor is not tokio.

use std::sync::OnceLock;
use std::time::Duration;

use architect::ClientError;
use auth_client::StoredSession;
use auth_proto::{
    AuthFlowError, AuthServiceHttpClient, AuthSessionBundle, DeviceRegistration,
    DeviceServiceHttpClient, SignInEmailPassword,
};

use crate::model::{Account, Device};

/// FTS's issuer.
pub const DEFAULT_ISSUER: &str = "https://auth.fasttrackstudio.app";

/// Names another issuer (a self-hosted one, a local `apps/auth-server`).
pub const ISSUER_ENV: &str = "ARCHITECT_AUTH_ISSUER";

/// [`ISSUER_ENV`]'s issuer when set, else [`DEFAULT_ISSUER`].
#[must_use]
pub fn issuer_from_env() -> String {
    std::env::var(ISSUER_ENV)
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_ISSUER.to_owned())
}

/// What the issuer said no with, or why it could not be asked.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IssuerError {
    #[error("that email and password don't match an account")]
    InvalidCredentials,
    #[error("the session ended: sign in again")]
    SessionExpired,
    #[error("this account has two-factor sign-in: continue in the browser instead")]
    TwoFactor,
    #[error("the account's email isn't verified yet: check your inbox")]
    Unverified,
    #[error("the sign-in was refused")]
    Refused,
    #[error("the browser code expired: start again")]
    Expired,
    #[error("the issuer refused: {0}")]
    Other(String),
    #[error("couldn't reach {issuer}: {detail}")]
    Unreachable { issuer: String, detail: String },
}

impl IssuerError {
    /// The session is gone for good: the account needs signing in again.
    #[must_use]
    pub const fn ends_session(&self) -> bool {
        matches!(self, Self::SessionExpired | Self::InvalidCredentials)
    }
}

/// An account signed in: who, and its session.
#[derive(Clone, Debug)]
pub struct SignedIn {
    pub account: Account,
    pub session: StoredSession,
}

/// A browser sign-in started: the code its person approves, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserCode {
    /// What the app polls with. A secret: never shown.
    pub device_code: String,
    /// What its person sees on the approval page.
    pub user_code: String,
    /// The approval page, the code filled in.
    pub url: String,
    pub expires_in: Duration,
    pub interval: Duration,
}

/// A poll's answer.
#[derive(Clone, Debug)]
pub enum Polled {
    /// Not approved yet.
    Waiting,
    /// Asked too often: wait longer.
    SlowDown,
    Approved(Box<SignedIn>),
}

/// One issuer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issuer {
    origin: String,
}

fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("accounts-net")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// Runs `work` on the accounts' runtime, in the background.
pub fn spawn(work: impl Future<Output = ()> + Send + 'static) {
    if let Ok(rt) = runtime() {
        drop(rt.spawn(work));
    }
}

impl Issuer {
    /// The issuer at `origin` (`https://auth.fasttrackstudio.app`).
    #[must_use]
    pub fn new(origin: impl Into<String>) -> Self {
        Self {
            origin: origin.into().trim().trim_end_matches('/').to_owned(),
        }
    }

    /// Its origin.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    fn auth(&self) -> AuthServiceHttpClient {
        AuthServiceHttpClient::at(self.origin.clone())
    }

    fn devices_client(&self) -> DeviceServiceHttpClient {
        DeviceServiceHttpClient::at(self.origin.clone())
    }

    fn error(&self, e: ClientError<AuthFlowError>) -> IssuerError {
        match e {
            ClientError::App(AuthFlowError::InvalidCredentials) => IssuerError::InvalidCredentials,
            ClientError::App(AuthFlowError::SessionExpired) => IssuerError::SessionExpired,
            ClientError::App(AuthFlowError::TwoFactorRequired) => IssuerError::TwoFactor,
            ClientError::App(AuthFlowError::VerificationRequired) => IssuerError::Unverified,
            ClientError::App(AuthFlowError::PermissionDenied) => IssuerError::Refused,
            ClientError::App(AuthFlowError::InvalidInput(m) | AuthFlowError::Internal(m)) => {
                IssuerError::Other(m)
            }
            ClientError::Connect(detail) | ClientError::Transport { detail, .. } => {
                IssuerError::Unreachable {
                    issuer: self.origin.clone(),
                    detail,
                }
            }
        }
    }

    fn signed_in(&self, bundle: AuthSessionBundle) -> SignedIn {
        let user = bundle.user;
        let id = user.id.to_string();
        let mut session = StoredSession::new(bundle.token)
            .with_user_id(id.clone())
            .with_expires_at_unix(bundle.session.expires_at.timestamp());
        if let Some(email) = &user.email {
            session = session.with_email(email.clone());
        }
        SignedIn {
            account: Account {
                id,
                issuer: self.origin.clone(),
                email: user.email,
                name: user.name.or(user.display_username),
                image: user.image,
                needs_sign_in: false,
            },
            session,
        }
    }

    async fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, IssuerError>> + Send + 'static,
    ) -> Result<T, IssuerError> {
        let unreachable = |detail: String| IssuerError::Unreachable {
            issuer: self.origin.clone(),
            detail,
        };
        runtime()
            .map_err(|e| unreachable(format!("no runtime: {e}")))?
            .spawn(work)
            .await
            .map_err(|e| unreachable(format!("the request stopped: {e}")))?
    }

    /// Signs in with an email and password.
    ///
    /// # Errors
    ///
    /// [`IssuerError::InvalidCredentials`], [`IssuerError::TwoFactor`], …
    pub async fn sign_in_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<SignedIn, IssuerError> {
        let (me, client) = (self.clone(), self.auth());
        let input = SignInEmailPassword {
            email: email.trim().to_owned(),
            password: password.to_owned(),
            ip_address: None,
            user_agent: Some(user_agent()),
        };
        self.run(async move {
            let bundle = client
                .sign_in_email_password(input)
                .await
                .map_err(|e| me.error(e))?;
            Ok(me.signed_in(bundle))
        })
        .await
    }

    /// Starts a browser sign-in (the device authorization grant): its
    /// person approves the code on the issuer's page, signed in there with
    /// whatever the issuer offers.
    ///
    /// # Errors
    ///
    /// The issuer's, or [`IssuerError::Unreachable`].
    pub async fn start_browser(&self, client_id: &str) -> Result<BrowserCode, IssuerError> {
        let (me, client, id) = (self.clone(), self.auth(), client_id.to_owned());
        self.run(async move {
            let started = client
                .start_device_sign_in(id)
                .await
                .map_err(|e| me.error(e))?;
            let url = if started.verification_uri_complete.starts_with('/') {
                format!("{}{}", me.origin, started.verification_uri_complete)
            } else {
                started.verification_uri_complete
            };
            let secs = |n: i64| Duration::from_secs(u64::try_from(n).unwrap_or(0));
            Ok(BrowserCode {
                device_code: started.device_code,
                user_code: started.user_code,
                url,
                expires_in: secs(started.expires_in_seconds),
                interval: secs(started.interval_seconds).max(Duration::from_secs(1)),
            })
        })
        .await
    }

    /// Asks whether the browser sign-in `device_code` is approved.
    ///
    /// # Errors
    ///
    /// [`IssuerError::Refused`] (its person said no), [`IssuerError::Expired`].
    pub async fn poll_browser(&self, device_code: &str) -> Result<Polled, IssuerError> {
        let (me, client, code) = (self.clone(), self.auth(), device_code.to_owned());
        self.run(async move {
            match client.poll_device_sign_in(code, Some(user_agent())).await {
                Ok(bundle) => Ok(Polled::Approved(Box::new(me.signed_in(bundle)))),
                Err(ClientError::App(AuthFlowError::VerificationRequired)) => Ok(Polled::Waiting),
                Err(ClientError::App(AuthFlowError::InvalidInput(m))) if m == "slow_down" => {
                    Ok(Polled::SlowDown)
                }
                Err(ClientError::App(AuthFlowError::InvalidCredentials)) => {
                    Err(IssuerError::Expired)
                }
                Err(e) => Err(me.error(e)),
            }
        })
        .await
    }

    /// Who `token` is, and its session now (its expiry).
    ///
    /// # Errors
    ///
    /// [`IssuerError::SessionExpired`] when it is no longer valid.
    pub async fn session(&self, token: &str) -> Result<SignedIn, IssuerError> {
        let (me, client, token) = (self.clone(), self.auth(), token.to_owned());
        self.run(async move {
            let bundle = client
                .current_session(token)
                .await
                .map_err(|e| me.error(e))?;
            Ok(me.signed_in(bundle))
        })
        .await
    }

    /// A fresh session for `token` (the old one ends).
    ///
    /// # Errors
    ///
    /// [`IssuerError::SessionExpired`] when it is no longer valid.
    pub async fn refresh(&self, token: &str) -> Result<SignedIn, IssuerError> {
        let (me, client, token) = (self.clone(), self.auth(), token.to_owned());
        self.run(async move {
            let bundle = client
                .refresh_session(token)
                .await
                .map_err(|e| me.error(e))?;
            Ok(me.signed_in(bundle))
        })
        .await
    }

    /// Ends `token`'s session at the issuer. Idempotent there.
    ///
    /// # Errors
    ///
    /// [`IssuerError::Unreachable`].
    pub async fn sign_out(&self, token: &str) -> Result<(), IssuerError> {
        let (me, client, token) = (self.clone(), self.auth(), token.to_owned());
        self.run(async move { client.sign_out(token).await.map_err(|e| me.error(e)) })
            .await
    }

    /// Tells the issuer this device is signed in as `session`'s account,
    /// so the account's devices list it. Nothing is sent when it is listed.
    ///
    /// # Errors
    ///
    /// The issuer's.
    pub async fn register_this_device(&self, session: &StoredSession) -> Result<(), IssuerError> {
        let Some(id) = this_device(session) else {
            return Ok(());
        };
        let (me, client, token) = (self.clone(), self.devices_client(), session.token.clone());
        self.run(async move {
            let registration = DeviceRegistration {
                device_id: id,
                name: auth_client::this_device::name(),
                platform: auth_client::this_device::platform().to_owned(),
                endpoint_id: None,
                relay_url: None,
                offer_org: None,
            };
            let listed = client
                .list_devices(token.clone())
                .await
                .map_err(|e| me.error(e))?;
            if registration.is_listed(&listed.devices) {
                return Ok(());
            }
            client
                .register_device(token, registration)
                .await
                .map(drop)
                .map_err(|e| me.error(e))
        })
        .await
    }

    /// The account's devices, this one marked.
    ///
    /// # Errors
    ///
    /// The issuer's.
    pub async fn devices(&self, session: &StoredSession) -> Result<Vec<Device>, IssuerError> {
        let here = this_device(session);
        let (me, client, token) = (self.clone(), self.devices_client(), session.token.clone());
        self.run(async move {
            let directory = client.list_devices(token).await.map_err(|e| me.error(e))?;
            Ok(directory
                .devices
                .into_iter()
                .map(|d| Device {
                    id: d.device_id.to_string(),
                    this_one: Some(d.device_id) == here,
                    name: d.name,
                    platform: d.platform,
                    last_seen: d.last_seen_at.to_rfc3339(),
                })
                .collect())
        })
        .await
    }

    /// Takes the device `id` off the account.
    ///
    /// # Errors
    ///
    /// [`IssuerError::Other`] for an id that is not a device's; the issuer's.
    pub async fn forget_device(
        &self,
        session: &StoredSession,
        id: &str,
    ) -> Result<(), IssuerError> {
        let device: uuid::Uuid = id
            .trim()
            .parse()
            .map_err(|_| IssuerError::Other(format!("no device {id}")))?;
        let (me, client, token) = (self.clone(), self.devices_client(), session.token.clone());
        self.run(async move {
            client
                .forget_device(token, device)
                .await
                .map_err(|e| me.error(e))
        })
        .await
    }
}

/// This device's id at the issuer for `session`'s account.
fn this_device(session: &StoredSession) -> Option<uuid::Uuid> {
    let account = session.user_id.as_deref()?;
    let install = auth_client::this_device::install_id()?;
    Some(auth_client::this_device::device_id(install, account))
}

/// What the issuer's session list calls this sign-in.
fn user_agent() -> String {
    format!(
        "Architect on {} ({})",
        auth_client::this_device::name(),
        auth_client::this_device::platform()
    )
}
