//! Outgoing mail.
//!
//! `architect-auth` is deliberately delivery-agnostic: every flow that
//! needs to reach a person — email verification, password reset, the email
//! OTP — mints a token and *returns* it to the caller. The engine never
//! sends anything and has no mailer trait. That is the right split: a
//! library has no business owning an SMTP connection, and an embedder that
//! already has a mail pipeline should use it.
//!
//! It does mean a deployment without this module has flows that cannot
//! complete. Before it existed the issuer could create a password-reset
//! token and then drop it on the floor, which is why there was no
//! forgot-password route at all: there was nowhere for the token to go.
//!
//! Configuration is optional. With no SMTP host set the mailer runs in
//! **log mode** — it records what it would have sent, at INFO, including
//! the link — so local development and a first deploy work without
//! credentials. That is a development convenience and a production
//! hazard: a log-mode server in production silently fails to deliver
//! anything while looking healthy, so `is_live()` is surfaced at startup.

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

/// Everything needed to send, or the absence of it.
#[derive(Clone, Debug)]
pub struct MailConfig {
    /// SMTP host. `None` puts the mailer in log mode.
    pub host: Option<String>,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Envelope and header `From`. Must be an address the provider has
    /// been told this server may send as, or everything bounces.
    pub from: String,
    /// The externally visible origin, used to build the links in mail.
    /// Must be the public URL — a link to the pod address helps nobody.
    pub base_url: String,
}

#[derive(Clone)]
pub struct Mailer {
    config: MailConfig,
    transport: Option<AsyncSmtpTransport<Tokio1Executor>>,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("smtp transport could not be built: {0}")]
    Transport(#[from] lettre::transport::smtp::Error),
    #[error("message could not be built: {0}")]
    Build(#[from] lettre::error::Error),
    #[error("address is not valid: {0}")]
    Address(#[from] lettre::address::AddressError),
}

impl Mailer {
    pub fn new(config: MailConfig) -> Result<Self, MailError> {
        let transport = match config.host.as_deref() {
            None => None,
            Some(host) => {
                // STARTTLS on the submission port, which is what every
                // transactional provider expects; `relay` refuses to fall
                // back to plaintext, so a downgrade fails loudly rather
                // than shipping credentials in the clear.
                let mut builder =
                    AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?.port(config.port);
                if let (Some(user), Some(pass)) = (&config.username, &config.password) {
                    builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
                }
                Some(builder.build())
            }
        };
        Ok(Self { config, transport })
    }

    /// Whether mail will actually leave the building.
    pub fn is_live(&self) -> bool {
        self.transport.is_some()
    }

    /// Absolute link into this server, for use in mail.
    pub fn link(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url.trim_end_matches('/'), path)
    }

    pub async fn send(&self, to: &str, subject: &str, body: String) -> Result<(), MailError> {
        let Some(transport) = &self.transport else {
            // Deliberately logs the whole body: in log mode the operator
            // IS the mail transport, and a verification link they cannot
            // read is useless.
            tracing::info!(
                target: "auth_server::mail",
                to, subject, body, "mail not sent (no SMTP host configured)"
            );
            return Ok(());
        };

        let message = Message::builder()
            .from(self.config.from.parse()?)
            .to(to.parse()?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)?;

        match transport.send(message).await {
            Ok(_) => {
                tracing::info!(target: "auth_server::mail", to, subject, "sent");
                Ok(())
            }
            Err(err) => {
                // Never fatal to the caller. A sign-up that succeeded must
                // not be reported as failed because the mail provider had
                // a bad minute — the account exists, and the person can
                // ask for another link.
                tracing::error!(target: "auth_server::mail", to, subject, %err, "send failed");
                Err(err.into())
            }
        }
    }

    pub async fn send_email_verification(&self, to: &str, user_id: uuid::Uuid, token: &str) {
        let link = self.link(&format!(
            "/verify-email?user_id={user_id}&token={}",
            urlencode(token)
        ));
        let body = format!(
            "Confirm your FastTrackStudio account by opening this link:\n\n\
             {link}\n\n\
             It is valid for a limited time. If you did not create an account, \
             you can ignore this message — nothing will happen until the link \
             is opened.\n"
        );
        let _ = self
            .send(to, "Confirm your FastTrackStudio account", body)
            .await;
    }

    pub async fn send_password_reset(&self, to: &str, token: &str) {
        let link = self.link(&format!(
            "/reset-password?email={}&token={}",
            urlencode(to),
            urlencode(token)
        ));
        let body = format!(
            "Someone asked to reset the password for this FastTrackStudio \
             account. Open this link to choose a new one:\n\n\
             {link}\n\n\
             If that was not you, ignore this message. Your password has not \
             been changed, and nothing happens until the link is opened.\n"
        );
        let _ = self
            .send(to, "Reset your FastTrackStudio password", body)
            .await;
    }
}

/// Percent-encode a value going into a query parameter. Same reasoning as
/// `ui::encode_query_value`, which this deliberately mirrors: a token that
/// happens to contain `&` or `+` would otherwise silently truncate the
/// link and strand someone mid-reset.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> MailConfig {
        MailConfig {
            host: None,
            port: 587,
            username: None,
            password: None,
            from: "noreply@example.com".into(),
            base_url: "https://auth.example.com/".into(),
        }
    }

    #[test]
    fn without_a_host_the_mailer_is_not_live() {
        let mailer = Mailer::new(config()).expect("build");
        assert!(!mailer.is_live());
    }

    #[test]
    fn links_are_absolute_and_do_not_double_the_slash() {
        let mailer = Mailer::new(config()).expect("build");
        assert_eq!(
            mailer.link("/verify-email?x=1"),
            "https://auth.example.com/verify-email?x=1"
        );
    }

    /// A token is opaque and may contain anything. If it reaches the link
    /// unencoded, the query truncates at the first `&` and the person is
    /// sent to a reset page missing the token that authorises it.
    #[test]
    fn tokens_are_encoded_into_the_link() {
        assert_eq!(urlencode("a&b=c+d"), "a%26b%3Dc%2Bd");
        assert_eq!(urlencode("plain-token_123"), "plain-token_123");
    }
}
