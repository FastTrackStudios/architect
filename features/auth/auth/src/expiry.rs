//! Non-panicking expiry arithmetic.
//!
//! Every short-lived credential in `flows` — a session, a password-reset
//! token, an OTP, an OAuth state, a passkey challenge — is stamped with
//! `now + ttl`. Written as `Utc::now() + Duration::seconds(ttl)`, that
//! addition **panics** on overflow, and the ttl is not always a
//! constant: `session_ttl_seconds` and `oidc.code_ttl_seconds` come from
//! config, and `expires_in_seconds` arrives over the wire.
//!
//! A configuration typo therefore had a path to a panic inside a request
//! handler — two, in fact: `Duration::seconds` panics on an
//! out-of-range count *before* the addition ever happens. These helpers
//! saturate at the representable bounds instead, which for an *expiry*
//! is exactly the right degenerate answer: a saturated deadline is
//! "effectively never", and a saturated cutoff is "effectively
//! always".

use chrono::{DateTime, Duration, Utc};

/// `now + secs`, saturating at [`DateTime::<Utc>::MAX_UTC`] rather than
/// panicking.
#[must_use]
pub fn expires_in(secs: i64) -> DateTime<Utc> {
    after(Utc::now(), secs)
}

/// `base + secs`, saturating at [`DateTime::<Utc>::MAX_UTC`].
#[must_use]
pub fn after(base: DateTime<Utc>, secs: i64) -> DateTime<Utc> {
    // `try_seconds` first: `Duration::seconds` is itself a panicking
    // constructor for counts beyond `TimeDelta`'s range.
    Duration::try_seconds(secs)
        .and_then(|d| base.checked_add_signed(d))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// Seconds in a day, for the callers that think in days.
const DAY: i64 = 60 * 60 * 24;

/// `now + days`, saturating — including the days-to-seconds conversion.
///
/// Invitation and invite-link expiries are chosen in days, and by a
/// person typing into a form. `days * 86_400` overflows long before
/// `i64::MAX` days does, so the multiply saturates too.
#[must_use]
pub fn in_days(days: i64) -> DateTime<Utc> {
    expires_in(days.saturating_mul(DAY))
}

/// `now - secs`, saturating at [`DateTime::<Utc>::MIN_UTC`]. The cutoff
/// half of the same idea: "everything older than N seconds ago".
#[must_use]
pub fn ago(secs: i64) -> DateTime<Utc> {
    before(Utc::now(), secs)
}

/// `base - secs`, saturating at [`DateTime::<Utc>::MIN_UTC`].
#[must_use]
pub fn before(base: DateTime<Utc>, secs: i64) -> DateTime<Utc> {
    Duration::try_seconds(secs)
        .and_then(|d| base.checked_sub_signed(d))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{after, before, expires_in};
    use chrono::{DateTime, Utc};

    #[test]
    fn ordinary_ttl_is_exact() {
        let base = DateTime::from_timestamp(1_000_000, 0).unwrap();
        assert_eq!(after(base, 60).timestamp(), 1_000_060);
        assert_eq!(before(base, 60).timestamp(), 999_940);
    }

    #[test]
    fn absurd_ttl_saturates_instead_of_panicking() {
        // This is the case that used to abort the request handler.
        assert_eq!(expires_in(i64::MAX), DateTime::<Utc>::MAX_UTC);
        assert_eq!(after(DateTime::<Utc>::MAX_UTC, 1), DateTime::<Utc>::MAX_UTC);
        assert_eq!(
            before(DateTime::<Utc>::MIN_UTC, 1),
            DateTime::<Utc>::MIN_UTC
        );
    }
}
