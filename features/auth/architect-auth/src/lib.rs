//! Public entry point for architect-auth.

pub use auth::*;
pub use auth_proto as proto;

#[cfg(feature = "db")]
pub mod db {
    pub use auth::backend_db::AuthSeaOrmStorage;
    pub use auth_db::*;
}

/// Client session kit — token storage + vox call decoration. A
/// re-export of the standalone `auth-client` crate; wasm builds can
/// also depend on `auth-client` directly (this umbrella pulls the
/// native-only engine).
#[cfg(feature = "client")]
pub mod client {
    pub use auth_client::*;

    use auth_proto::AuthSessionBundle;

    /// Build a [`StoredSession`] from the bundle a sign-up / sign-in /
    /// refresh returned — token plus user id, email, and expiry.
    pub fn stored_session(bundle: &AuthSessionBundle) -> StoredSession {
        let mut session = StoredSession::new(bundle.token.clone())
            .with_user_id(bundle.user.id.to_string())
            .with_expires_at_unix(bundle.session.expires_at.timestamp());
        if let Some(email) = &bundle.user.email {
            session = session.with_email(email.clone());
        }
        session
    }
}
