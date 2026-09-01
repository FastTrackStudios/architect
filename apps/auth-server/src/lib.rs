//! **The FastTrackStudio identity server** — `architect-auth` as a
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
//!   and an axum [`Router`](axum::Router) you can mount inside a larger
//!   app; and
//! * a **binary** (`auth-server`) — configuration from the environment,
//!   migrations on boot, listener, health probes.
//!
//! # Surfaces
//!
//! | Path | Purpose |
//! |------|---------|
//! | `/vox` | vox RPC over WebSocket — the native path for the Rust apps |
//! | `/.well-known/openid-configuration` | OIDC discovery |
//! | `/oauth2/{authorize,token,userinfo}` | OIDC provider |
//! | `/auth/jwt/jwks` | key set (see the caveat below) |
//! | `/auth/{sign-up,sign-in,session,refresh,sign-out}` | session JSON |
//! | `/healthz`, `/readyz` | probes |
//!
//! # Known limitations
//!
//! **JWT signing is symmetric.** The engine hardcodes `HS256`, so there
//! is no public key to publish and `/auth/jwt/jwks` returns an empty key
//! set by design — see [`http`]. First-party relying parties can verify
//! through `/oauth2/userinfo`; a genuine third-party RP cannot verify an
//! id_token offline until the engine grows RS256/ES256 support. That is
//! a change in `auth/src/flows.rs`, not here.
//!
//! **The HTTP surface is a subset.** `architect-auth` describes ~150
//! routes; this server mounts the OIDC provider and the core session
//! API. The rest remain available over vox and through `ArchitectAuth`
//! directly. Mounting them generically is blocked on the command structs
//! deriving no serde.

pub mod config;
pub mod http;
pub mod server;
pub mod ui;

pub use config::{ConfigError, ServerConfig};
pub use server::{AuthServer, VOX_SUBPROTOCOL, app_router, build, build_engine, serve};
