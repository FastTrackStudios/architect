//! Server-rendered account and organization pages for architect-auth.
//!
//! # Why these live in a crate and not in the server
//!
//! The pages that manage an organization — who is in it, what they may
//! do, which links let more people in — are the same pages for every
//! deployment. They were written once inside `apps/auth-server`, where
//! no other binary could reach them, so a second product wanting an
//! org switcher had to reimplement one. Here they mount anywhere:
//!
//! ```no_run
//! # use auth_ui::UiState;
//! # fn example<S: architect_auth::AuthStorage + Clone + Send + Sync + 'static>(
//! #     auth: architect_auth::ArchitectAuth<S>,
//! #     cookie: architect_auth::transport::AuthCookieConfig,
//! # ) -> axum::Router {
//! axum::Router::new().merge(auth_ui::router(UiState::new(auth, cookie)))
//! # }
//! ```
//!
//! # Server-rendered, and no script at all
//!
//! Rendered with `dioxus-ssr` into plain HTML, with ordinary `<form>`
//! posts — the same choice the sign-in screen makes, for the same
//! reason and one more. The reason: an account page reached by someone
//! locked out of an app must not itself depend on a WASM bundle
//! booting. The extra one: every action here is a POST with a
//! deterministic redirect, so the whole surface is testable by a
//! browser driver without waiting on hydration, and by `tower::oneshot`
//! without a browser at all.
//!
//! # What is deliberately not here
//!
//! Sign-in, sign-up and password reset stay with the server that owns
//! the mailer and the social providers. This crate is what a person
//! does *after* they are signed in.

pub mod accounts;
pub mod assets;
pub mod admin;
pub mod api_keys;
pub mod chrome;
pub mod consent;
pub mod device;
pub mod guest;
pub mod last_login;
pub mod login;
pub mod mailer;
pub mod multi_session;
pub mod orgs;
pub mod page;
pub mod passkey_script;
pub mod passkeys;
pub mod phone;
pub mod profile;
pub mod qr;
pub mod sessions;
pub mod settings;
pub mod two_factor;
pub mod views;
pub mod wallet;

use architect_auth::transport::AuthCookieConfig;
use architect_auth::{ArchitectAuth, AuthStorage};
use axum::Router;
use axum::routing::{get, post};

/// What the pages need from the host application.
///
/// Deliberately small: the engine and the cookie policy, and nothing
/// about mail, social providers or OIDC. A consumer that has an
/// `ArchitectAuth` can mount these pages, which is the point.
#[derive(Clone)]
pub struct UiState<S> {
    pub auth: ArchitectAuth<S>,
    pub cookie: AuthCookieConfig,
    /// Where "back to the app" goes. `/` unless set.
    pub home: String,
    /// The name an authenticator app shows next to a two-factor entry.
    /// Your product's name; "architect-auth" unless set.
    pub issuer: String,
    /// This server's public URL, for building links that arrive by
    /// mail. Must match what the engine was configured with, or a magic
    /// link callback is refused as untrusted.
    pub base_url: String,
    /// The domain a wallet signature is bound to. Must match what the
    /// engine was configured with, or every signature is refused.
    pub siwe_domain: String,
    /// How a sign-in code or link reaches somebody. Logs them instead
    /// of sending, unless the host supplies a real sender.
    pub mailer: std::sync::Arc<dyn mailer::LoginMailer>,
    /// How a code reaches a phone. Logs instead of sending, unless the
    /// host supplies a real sender.
    pub sms: std::sync::Arc<dyn mailer::SmsSender>,
}

impl<S> UiState<S> {
    #[must_use]
    pub fn new(auth: ArchitectAuth<S>, cookie: AuthCookieConfig) -> Self {
        Self {
            auth,
            cookie,
            home: "/".to_owned(),
            issuer: "architect-auth".to_owned(),
            mailer: mailer::log_only(),
            sms: mailer::log_only_sms(),
            base_url: "http://localhost:8080".to_owned(),
            siwe_domain: "localhost".to_owned(),
        }
    }

    /// This server's public URL, used to build mailed links.
    #[must_use]
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// The domain wallet signatures are bound to.
    #[must_use]
    pub fn siwe_domain(mut self, domain: impl Into<String>) -> Self {
        self.siwe_domain = domain.into();
        self
    }

    /// Supply a real sender for sign-in codes and links.
    #[must_use]
    pub fn mailer(mut self, mailer: std::sync::Arc<dyn mailer::LoginMailer>) -> Self {
        self.mailer = mailer;
        self
    }

    /// Supply a real sender for phone codes.
    #[must_use]
    pub fn sms(mut self, sms: std::sync::Arc<dyn mailer::SmsSender>) -> Self {
        self.sms = sms;
        self
    }

    /// The name an authenticator app shows for a two-factor entry.
    #[must_use]
    pub fn issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = issuer.into();
        self
    }

    #[must_use]
    pub fn home(mut self, home: impl Into<String>) -> Self {
        self.home = home.into();
        self
    }
}

/// Mount every page this crate provides.
///
/// One router rather than several so a consumer cannot accidentally
/// mount the org pages without the invitation pages that org pages
/// link to.
pub fn router<S>(state: UiState<S>) -> Router
where
    S: AuthStorage + Clone + Send + Sync + 'static,
{
    Router::new()
        // ── What the pages are drawn with ─────────────────────────
        .route("/auth/assets/{file}", get(assets::serve))
        // ── The person ────────────────────────────────────────────
        .route(
            "/account/profile",
            get(profile::page::<S>).post(profile::save::<S>),
        )
        .route("/account/sign-out", post(profile::sign_out::<S>))
        .route("/oauth2/consent", post(consent::decide::<S>))
        .route("/login/guest", post(guest::start::<S>))
        .route("/account/upgrade", post(guest::upgrade::<S>))
        .route(
            "/account/phone",
            get(phone::page::<S>).post(phone::send_code::<S>),
        )
        .route("/account/phone/verify", post(phone::verify::<S>))
        .route(
            "/account/switch",
            get(accounts::page::<S>).post(accounts::switch::<S>),
        )
        .route("/account/switch/leave", post(accounts::leave::<S>))
        .route("/account/switch/leave-all", post(accounts::leave_all::<S>))
        // ── A device that has no browser ──────────────────────────
        // `/auth/device` is not a choice: the engine puts it in the
        // `verification_uri` printed on the other device's screen.
        .route(
            "/auth/device",
            get(device::page::<S>).post(device::look_up::<S>),
        )
        .route("/auth/device/approve", post(device::approve::<S>))
        .route("/auth/device/deny", post(device::deny::<S>))
        // ── Ways in that are not a password ───────────────────────
        .route(
            "/login/code",
            get(login::code_page::<S>).post(login::send_code::<S>),
        )
        .route("/login/code/verify", post(login::verify_code::<S>))
        .route(
            "/login/link",
            get(login::link_page::<S>).post(login::send_link::<S>),
        )
        .route("/login/magic", get(login::magic_callback::<S>))
        .route("/account/email", post(profile::change_email::<S>))
        .route("/account/password", post(profile::change_password::<S>))
        .route("/account/two-factor", get(two_factor::page::<S>))
        .route("/account/two-factor/enroll", post(two_factor::enroll::<S>))
        .route(
            "/account/two-factor/confirm",
            post(two_factor::confirm::<S>),
        )
        .route(
            "/account/two-factor/disable",
            post(two_factor::disable::<S>),
        )
        // The step between a password and a session. Not under
        // `/account`, because at this moment there is no usable session
        // for an account page to render from.
        .route(
            "/login/two-factor",
            get(two_factor::challenge::<S>).post(two_factor::verify::<S>),
        )
        .route(
            "/account/api-keys",
            get(api_keys::page::<S>).post(api_keys::create::<S>),
        )
        .route("/account/api-keys/revoke", post(api_keys::revoke::<S>))
        .route("/account/api-keys/delete", post(api_keys::delete::<S>))
        .route("/account/passkeys", get(passkeys::page::<S>))
        .route(
            "/account/passkeys/begin",
            post(passkeys::begin_registration::<S>),
        )
        .route(
            "/account/passkeys/complete",
            post(passkeys::complete_registration::<S>),
        )
        .route("/account/passkeys/delete", post(passkeys::delete::<S>))
        .route("/login/wallet/begin", post(wallet::begin::<S>))
        .route("/login/wallet/complete", post(wallet::complete::<S>))
        .route("/login/passkey/begin", post(passkeys::begin_sign_in::<S>))
        .route(
            "/login/passkey/complete",
            post(passkeys::complete_sign_in::<S>),
        )
        .route("/account/sessions", get(sessions::page::<S>))
        .route("/account/sessions/revoke", post(sessions::revoke::<S>))
        .route(
            "/account/sessions/revoke-others",
            post(sessions::revoke_others::<S>),
        )
        // ── Organizations ─────────────────────────────────────────
        .route("/orgs", get(orgs::index::<S>).post(orgs::create::<S>))
        .route("/orgs/{id}", get(orgs::show::<S>).post(orgs::update::<S>))
        .route("/orgs/{id}/delete", post(orgs::delete::<S>))
        .route("/orgs/{id}/leave", post(orgs::leave::<S>))
        .route("/orgs/{id}/members/role", post(orgs::set_role::<S>))
        .route("/orgs/{id}/members/remove", post(orgs::remove_member::<S>))
        .route("/orgs/{id}/teams", post(orgs::create_team::<S>))
        .route("/orgs/{id}/teams/delete", post(orgs::delete_team::<S>))
        .route("/orgs/{id}/teams/members", post(orgs::add_team_member::<S>))
        .route(
            "/orgs/{id}/teams/members/remove",
            post(orgs::remove_team_member::<S>),
        )
        .route("/orgs/{id}/invitations", post(orgs::invite::<S>))
        .route(
            "/orgs/{id}/invitations/cancel",
            post(orgs::cancel_invitation::<S>),
        )
        .route("/orgs/{id}/links", post(orgs::create_link::<S>))
        .route("/orgs/{id}/links/revoke", post(orgs::revoke_link::<S>))
        // ── Operator ──────────────────────────────────
        .route("/admin/users", get(admin::users::<S>))
        .route("/admin/users/role", post(admin::set_role::<S>))
        .route("/admin/users/ban", post(admin::ban::<S>))
        .route("/admin/users/unban", post(admin::unban::<S>))
        .route("/admin/users/delete", post(admin::delete::<S>))
        .route("/admin/users/impersonate", post(admin::impersonate::<S>))
        .route("/admin/stop-impersonating", post(admin::stop::<S>))
        // ── Coming in ─────────────────────────────────────────────
        // Unauthenticated on purpose: somebody following one of these
        // has no session yet, and must be able to see what they are
        // being asked to join before signing up for it.
        .route("/invite/{id}", get(orgs::invitation_page::<S>))
        .route("/invite/{id}/accept", post(orgs::accept_invitation::<S>))
        .route("/invite/{id}/reject", post(orgs::reject_invitation::<S>))
        .route("/join", get(orgs::join_page::<S>).post(orgs::join::<S>))
        .with_state(state)
}
