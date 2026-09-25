//! Signing a device in from another one (RFC 8628, the device authorization
//! grant): a device with no browser — a CLI, an app on a phone that should
//! not type a password — asks the auth server for a code, shows it, and
//! polls until its person approves the code from a browser where they are
//! signed in.
//!
//! This is the protocol, not a client: the bodies to post and what the
//! answers mean. Like [`crate::oidc`], it has no HTTP client of its own, so
//! it stays wasm-clean and every host posts with whatever it already has.
//! The routes are the auth server's (architect-auth's transport):
//! [`CODE_PATH`] starts a sign-in, [`TOKEN_PATH`] polls it.
//!
//! ```ignore
//! let started = http.post(format!("{issuer}{CODE_PATH}")).body(device::start_body("my-app")).send().await?;
//! let code = device::parse_start(&started.text().await?)?;
//! show(&code.user_code, &format!("{issuer}{}", code.verification_uri_complete));
//! let mut wait = code.interval();
//! let session = loop {
//!     sleep(wait).await;
//!     let answer = http.post(format!("{issuer}{TOKEN_PATH}")).body(device::poll_body(&code.device_code, "my-app")).send().await?;
//!     match device::parse_poll(answer.status().is_success(), &answer.text().await?)? {
//!         Poll::Approved(session) => break session,
//!         Poll::Pending => {}
//!         Poll::SlowDown => wait += device::SLOW_DOWN_STEP,
//!     }
//! };
//! store.save(&session)?;
//! ```

use std::time::Duration;

use crate::StoredSession;

/// Where a device sign-in starts, relative to the auth server.
pub const CODE_PATH: &str = "/auth/device/code";
/// Where it is polled.
pub const TOKEN_PATH: &str = "/auth/device/token";
/// How much longer to wait between polls each time the server says to
/// slow down (RFC 8628 §3.5).
pub const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);

/// A started sign-in: what to show, and what to poll with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    /// What the device polls with. A secret: never shown.
    pub device_code: String,
    /// What its person confirms, from a browser where they are signed in.
    pub user_code: String,
    /// Where they confirm it, relative to the auth server.
    pub verification_uri: String,
    /// The same with the code filled in: the link to open (or a QR code).
    pub verification_uri_complete: String,
    /// How long the code lasts.
    pub expires_in_seconds: u64,
    interval_seconds: u64,
}

impl DeviceCode {
    /// How long to wait between polls (at least a second).
    #[must_use]
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_seconds.max(1))
    }
}

/// What a poll said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// Approved: the device's session.
    Approved(StoredSession),
    /// Not yet approved: poll again after the interval.
    Pending,
    /// Polling too often: wait [`SLOW_DOWN_STEP`] longer from now on.
    SlowDown,
}

/// Why a device sign-in could not go on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    /// The person refused it.
    #[error("the sign-in was refused")]
    Denied,
    /// The code ran out before it was approved: start again.
    #[error("the code expired before it was approved")]
    Expired,
    /// The server said something this does not understand.
    #[error("the auth server answered what this cannot read: {0}")]
    Unreadable(String),
    /// The server refused for another reason; its words.
    #[error("device sign-in failed: {0}")]
    Failed(String),
}

/// The body that starts a sign-in for the client `client_id` (a name the
/// server shows its person — `task-cli`, `session-ios`).
#[must_use]
pub fn start_body(client_id: &str) -> String {
    serde_json::json!({ "client_id": client_id }).to_string()
}

/// The started sign-in, from [`CODE_PATH`]'s answer.
///
/// # Errors
///
/// [`DeviceError::Unreadable`] when the answer is not a started sign-in.
pub fn parse_start(body: &str) -> Result<DeviceCode, DeviceError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| DeviceError::Unreadable(body.to_owned()))?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| DeviceError::Unreadable(body.to_owned()))
    };
    let number = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_i64)
            .map_or(0, |n| u64::try_from(n).unwrap_or(0))
    };
    Ok(DeviceCode {
        device_code: text("device_code")?,
        user_code: text("user_code")?,
        verification_uri: text("verification_uri")?,
        verification_uri_complete: text("verification_uri_complete")?,
        expires_in_seconds: number("expires_in_seconds"),
        interval_seconds: number("interval_seconds"),
    })
}

/// The body of a poll for `device_code`, from `user_agent` (what the
/// person's session list calls this device).
#[must_use]
pub fn poll_body(device_code: &str, user_agent: &str) -> String {
    serde_json::json!({ "device_code": device_code, "user_agent": user_agent }).to_string()
}

/// What a poll's answer means: `ok` is whether it was a success status.
///
/// # Errors
///
/// [`DeviceError::Denied`] or [`DeviceError::Expired`] end the sign-in;
/// anything else the server said is [`DeviceError::Failed`], and an answer
/// that is not JSON is [`DeviceError::Unreadable`].
pub fn parse_poll(ok: bool, body: &str) -> Result<Poll, DeviceError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| DeviceError::Unreadable(body.to_owned()))?;
    if ok {
        let token = value
            .get("token")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| DeviceError::Unreadable(body.to_owned()))?;
        let user = value.get("user");
        let field = |key: &str| {
            user.and_then(|u| u.get(key))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        let mut session = StoredSession::new(token);
        if let Some(id) = field("id") {
            session = session.with_user_id(id);
        }
        if let Some(email) = field("email") {
            session = session.with_email(email);
        }
        return Ok(Poll::Approved(session));
    }
    // The error's stable code (architect-auth's `AuthFlowError::code`), not
    // its message — except slow_down, which rides `invalid_input`.
    match value.get("code").and_then(serde_json::Value::as_str) {
        Some("verification_required") => Ok(Poll::Pending),
        Some("invalid_input") if body.contains("slow_down") => Ok(Poll::SlowDown),
        Some("permission_denied") => Err(DeviceError::Denied),
        Some("invalid_credentials") => Err(DeviceError::Expired),
        _ => Err(DeviceError::Failed(body.to_owned())),
    }
}

/// The account server a Task server trusts for sign-in — the
/// `central_auth` issuer in its `/.well-known/task-server.json` — without a
/// trailing slash.
#[must_use]
pub fn issuer_from_well_known(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value
        .get("central_auth")
        .and_then(serde_json::Value::as_str)
        .map(|issuer| issuer.trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_started_sign_in_reads_its_code_and_links() {
        let code = parse_start(
            r#"{"device_code":"dev-secret","user_code":"DPCHMESG","verification_uri":"/auth/device",
                "verification_uri_complete":"/auth/device?user_code=DPCHMESG","expires_in_seconds":900,
                "interval_seconds":5}"#,
        )
        .unwrap();
        assert_eq!(code.user_code, "DPCHMESG");
        assert_eq!(
            code.verification_uri_complete,
            "/auth/device?user_code=DPCHMESG"
        );
        assert_eq!(code.interval(), Duration::from_secs(5));
        assert!(
            parse_start(r#"{"user_code":"X"}"#).is_err(),
            "no device code"
        );
    }

    #[test]
    fn a_poll_is_pending_slowed_approved_or_over() {
        let pending = r#"{"code":"verification_required","message":"not yet"}"#;
        assert_eq!(parse_poll(false, pending), Ok(Poll::Pending));
        let slow = r#"{"code":"invalid_input","message":"slow_down"}"#;
        assert_eq!(parse_poll(false, slow), Ok(Poll::SlowDown));
        assert_eq!(
            parse_poll(false, r#"{"code":"permission_denied"}"#),
            Err(DeviceError::Denied)
        );
        assert_eq!(
            parse_poll(false, r#"{"code":"invalid_credentials"}"#),
            Err(DeviceError::Expired)
        );
        let approved = r#"{"user":{"id":"91b9","email":"a@b.c"},"session":{},"token":"jwt"}"#;
        let Ok(Poll::Approved(session)) = parse_poll(true, approved) else {
            panic!("approved");
        };
        assert_eq!(
            session,
            StoredSession::new("jwt")
                .with_user_id("91b9")
                .with_email("a@b.c")
        );
    }

    #[test]
    fn the_issuer_comes_from_the_servers_well_known_document() {
        assert_eq!(
            issuer_from_well_known(r#"{"central_auth":"https://auth.example.app/"}"#).as_deref(),
            Some("https://auth.example.app")
        );
        assert_eq!(issuer_from_well_known(r#"{"name":"x"}"#), None);
    }
}
