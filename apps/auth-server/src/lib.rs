//! **The `FastTrackStudio` identity server** — `architect-auth` as a
//! deployable service instead of an embedded library.
//!
//! `architect-auth` is a complete auth engine (password, OAuth, passkey,
//! 2FA, magic link, API keys, organizations, teams, and a working OIDC
//! provider), but it ships as a library with no way to *run* it: the
//! only wire surface is a vox RPC trait a host has to mount, and the
//! HTTP side is route metadata with no router behind it. Every app that
//! wanted auth therefore embedded its own copy and its own user store —
//! which is exactly why one account cannot span Task, Session, Signal,
//! Keyflow and Ignition.
//!
//! This crate closes that gap. It is both:
//!
//! * a **library** — [`server::build`] hands back a configured engine
//!   and an [`EngineHost`](architect::host::EngineHost) you can turn into
//!   an axum [`Router`](axum::Router) and mount inside a larger app; and
//! * a **binary** (`auth-server`) — configuration from the environment,
//!   migrations on boot, listener, health probes.
//!
//! # Surfaces
//!
//! The services are declared once, as `#[architect::service]` traits in
//! `auth-proto`, and every wire below is generated from them — nothing in
//! this crate hand-writes a route that mirrors an RPC method:
//!
//! | Path | Purpose |
//! |------|---------|
//! | `/vox` | vox RPC over WebSocket — the native path for the Rust apps |
//! | iroh (`AUTH_IROH_KEY_FILE`) | the same vox router, peer to peer |
//! | `POST /auth/<method>` | `AuthService` as HTTP+JSON — sign-up, sign-in, session, refresh, sign-out, profile |
//! | `POST /organization/<method>` | `OrganizationService` as HTTP+JSON — orgs, members, invitations, roles |
//! | `/.well-known/openid-configuration` | OIDC discovery |
//! | `/oauth2/{authorize,token,userinfo}` | OIDC provider |
//! | `/auth/jwt/jwks` | key set (see the caveat below) |
//! | `/auth/social/{github,google,tone3000}/{start,callback}` | social sign-in and account linking |
//! | `/auth/accounts`, `/auth/accounts/{provider}/unlink` | linked accounts |
//! | `/oauth2/linked-token` | a linked GitHub token for a relying party (see [`social`]) |
//! | `/login`, `/sign-up`, `/account`, … | hosted pages |
//! | `/health`, `/healthz`, `/readyz` | probes |
//!
//! Clients are generated too: `auth_proto::AuthServiceClient` (vox) and
//! `auth_proto::AuthServiceHttpClient` (HTTP), and the same pair for
//! organizations. `apps/auth-server/tests/surfaces.rs` drives one suite
//! through every transport.
//!
//! # Known limitations
//!
//! **JWT signing is symmetric.** The engine hardcodes `HS256`, so there
//! is no public key to publish and `/auth/jwt/jwks` returns an empty key
//! set by design — see [`http`]. First-party relying parties can verify
//! through `/oauth2/userinfo`; a genuine third-party RP cannot verify an
//! `id_token` offline until the engine grows RS256/ES256 support. That is
//! a change in `auth/src/flows.rs`, not here.
//!
//! **The wire surface is what the traits declare.** `architect-auth`'s
//! engine speaks ~150 commands; the two `#[architect::service]` traits
//! expose the session and organization lifecycles. Exposing more is a
//! trait method away — both faces and both clients follow.

pub mod config;
pub mod dev;
pub mod mail;
pub mod oauth;
pub mod server;
pub mod social;
pub mod ui;

pub use config::{ConfigError, ServerConfig, SocialConfig, SocialProviderConfig};
pub use server::{
    AuthServer, VOX_SUBPROTOCOL, app_router, app_router_with_social, build, build_engine,
    http_router, serve, services, vox_router,
};
