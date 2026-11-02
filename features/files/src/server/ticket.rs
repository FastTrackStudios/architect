//! Read tickets: a short-lived, signed claim that `principal` may read
//! one path (and what is under it) in one root of one org.
//!
//! `fft1.<payload>.<mac>`: the payload is the claims as JSON, the mac its
//! HMAC-SHA256 under the server's key, both URL-safe base64, so a ticket
//! rides a query string as is. The ticket proves who minted it and what
//! for; what that person may read is asked again on every use, so a
//! grant revoked or a role lowered binds before the ticket runs out.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use facet::Facet;
use hmac::{Hmac, Mac as _};

use architect_permissions::Principal;

use crate::id::RootId;
use crate::path::RootPath;

/// How long a ticket reads: long enough to load and play a review
/// without minting again mid-cut, short enough that one leaked through a
/// browser's history or a proxy's log stops working the same day.
pub const TICKET_TTL_US: i64 = 6 * 60 * 60 * 1_000_000;

const PREFIX: &str = "fft1.";

type Mac = Hmac<sha2::Sha256>;

/// What a ticket says.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Claims {
    pub org: String,
    pub root: RootId,
    /// The path it reads, and everything under it.
    pub path: RootPath,
    /// `user:<id>` or `service:<name>`.
    pub who: String,
    pub expires_us: i64,
}

impl Claims {
    pub fn principal(&self) -> Option<Principal> {
        let (kind, name) = self.who.split_once(':')?;
        match kind {
            "user" => Some(Principal::User {
                user_id: name.to_owned(),
            }),
            "service" => Some(Principal::Service {
                name: name.to_owned(),
            }),
            _ => None,
        }
    }

    /// Whether it covers `path` in `root` of `org`.
    pub fn covers(&self, org: &str, root: RootId, path: &RootPath) -> bool {
        self.org == org && self.root == root && path.is_within(&self.path)
    }
}

/// How a principal is written into a ticket; `None` for one a ticket
/// cannot carry (a guest, a peer host, nobody).
pub fn who(principal: &Principal) -> Option<String> {
    match principal {
        Principal::User { user_id } => Some(format!("user:{user_id}")),
        Principal::Service { name } => Some(format!("service:{name}")),
        Principal::Guest { .. } | Principal::Host { .. } | Principal::Anonymous => None,
    }
}

/// Whether `token` is a ticket (and not a session).
pub fn is_ticket(token: &str) -> bool {
    token.starts_with(PREFIX)
}

/// The key tickets are signed with.
#[derive(Clone)]
pub struct TicketKey([u8; 32]);

impl TicketKey {
    pub const fn new(key: [u8; 32]) -> Self {
        Self(key)
    }

    /// A fresh random key: tickets read until this server stops.
    pub fn random() -> Self {
        Self(rand::random())
    }

    /// HMAC takes a key of any length, so this is always `Some`.
    fn mac(&self) -> Option<Mac> {
        <Mac as hmac::digest::KeyInit>::new_from_slice(&self.0).ok()
    }

    pub fn mint(&self, claims: &Claims) -> Option<String> {
        let payload = URL_SAFE_NO_PAD.encode(facet_json::to_string(claims).ok()?);
        let mut mac = self.mac()?;
        mac.update(payload.as_bytes());
        let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        Some(format!("{PREFIX}{payload}.{sig}"))
    }

    /// The claims of `token` if this key signed it and it has not run
    /// out at `now_us`.
    pub fn open(&self, token: &str, now_us: i64) -> Option<Claims> {
        let (payload, sig) = token.strip_prefix(PREFIX)?.split_once('.')?;
        let sig = URL_SAFE_NO_PAD.decode(sig).ok()?;
        let mut mac = self.mac()?;
        mac.update(payload.as_bytes());
        mac.verify_slice(&sig).ok()?;
        let json = String::from_utf8(URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
        let claims: Claims = facet_json::from_str(&json).ok()?;
        (claims.expires_us > now_us).then_some(claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn claims() -> Claims {
        Claims {
            org: "fts".into(),
            root: RootId::new(Uuid::nil()),
            path: RootPath::parse("Review/cut 2").unwrap(),
            who: "user:alice".into(),
            expires_us: 1_000,
        }
    }

    #[test]
    fn a_ticket_opens_under_its_key_until_it_runs_out() {
        let key = TicketKey::new([7; 32]);
        let token = key.mint(&claims()).unwrap();
        assert!(is_ticket(&token));
        assert_eq!(key.open(&token, 999), Some(claims()));
        assert_eq!(key.open(&token, 1_000), None);
        assert_eq!(TicketKey::new([8; 32]).open(&token, 0), None);
    }

    #[test]
    fn a_changed_claim_does_not_open() {
        let key = TicketKey::new([7; 32]);
        let token = key.mint(&claims()).unwrap();
        let (_, sig) = token.rsplit_once('.').unwrap();
        let forged = Claims {
            path: RootPath::root(),
            ..claims()
        };
        let payload = URL_SAFE_NO_PAD.encode(facet_json::to_string(&forged).unwrap());
        assert_eq!(key.open(&format!("{PREFIX}{payload}.{sig}"), 0), None);
    }

    #[test]
    fn it_covers_its_path_and_below_in_its_root_and_org() {
        let c = claims();
        let root = c.root;
        assert!(c.covers("fts", root, &RootPath::parse("Review/cut 2/a.mov").unwrap()));
        assert!(c.covers("fts", root, &RootPath::parse("Review/cut 2").unwrap()));
        assert!(!c.covers("fts", root, &RootPath::parse("Review/cut 20").unwrap()));
        assert!(!c.covers("hp", root, &RootPath::parse("Review/cut 2").unwrap()));
        assert!(!c.covers(
            "fts",
            RootId::new(Uuid::max()),
            &RootPath::parse("Review/cut 2").unwrap()
        ));
    }
}
