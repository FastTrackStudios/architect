//! Getting a code or a link to somebody.
//!
//! Two of the sign-in routes have to *send* something, and this crate
//! deliberately knows nothing about SMTP — it renders pages. So the
//! host supplies a sender, and the pages call it.
//!
//! A trait with two methods rather than a general mail interface: these
//! are the only two messages these pages send, and naming them means a
//! host cannot wire the wrong template into the wrong flow.
//!
//! # Neither method returns an error
//!
//! On purpose. A sign-in page must answer identically whether or not
//! the address exists, and whether or not delivery succeeded — anything
//! else tells a stranger which addresses have accounts here. Delivery
//! problems belong in the host's logs, not in the response. The
//! consequence is real and accepted: somebody whose mail bounces sees
//! "check your inbox" and waits.

use std::sync::Arc;

use async_trait::async_trait;

#[async_trait]
pub trait LoginMailer: Send + Sync + 'static {
    /// A one-click sign-in link.
    async fn send_magic_link(&self, to: &str, url: &str);
    /// A short code to type back in.
    async fn send_login_code(&self, to: &str, code: &str);
}

/// The default: writes the message to the log instead of sending it.
///
/// Mirrors what the server's own mailer does with no SMTP host
/// configured, and for the same reason — a local server should be
/// usable before anybody has configured mail, and an operator reading
/// the log *is* the transport. It logs the code and the link in full,
/// because a sign-in link nobody can read is no use at all.
///
/// It says so loudly on the way past, because this is exactly the
/// configuration nobody should reach production with.
pub struct LogOnlyMailer;

#[async_trait]
impl LoginMailer for LogOnlyMailer {
    async fn send_magic_link(&self, to: &str, url: &str) {
        tracing::warn!(
            target: "auth_ui::mail",
            %to,
            %url,
            "no mailer is configured — the sign-in link is only in this log"
        );
    }

    async fn send_login_code(&self, to: &str, code: &str) {
        tracing::warn!(
            target: "auth_ui::mail",
            %to,
            %code,
            "no mailer is configured — the sign-in code is only in this log"
        );
    }
}

/// The default sender, for a `UiState` that was not given one.
#[must_use]
pub fn log_only() -> Arc<dyn LoginMailer> {
    Arc::new(LogOnlyMailer)
}
