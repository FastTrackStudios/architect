//! Pairing a device that is not signed in, or is somebody else's.
//!
//! A person on a signed-in device decides to let another device in, and
//! what it may do ([`Scope`]s, narrow only). That device makes an offer
//! ([`Pairings::offer`]): a six-digit code to read out, and a ticket
//! ([`PairTicket`]) for a QR code or a link, whose secret rides the URL
//! fragment so no server it passes through sees it. The new device dials
//! the offering one by its endpoint id (from the ticket, or picked from
//! the devices nearby, `iroh_link::LanBrowser`) and redeems the offer
//! with the code or the secret ([`Pairings::redeem`], the `DevicePairing`
//! service). The connection proves its endpoint id, so that is what is
//! let in: the offering device's [`Admission`] holds it as
//! [`Grant::Paired`], with the offer's scopes, until it is revoked.
//!
//! No server is involved, so pairing works with every server stopped.
//!
//! An offer is single use and lasts [`OFFER_TTL_MS`]. A code is six
//! digits, so guessing is bounded instead: every wrong code or secret
//! costs every live offer one of its [`CODE_TRIES`], and an offer out of
//! tries is withdrawn. A secret (from the ticket) is 256 random bits.
//!
//! Time is the caller's (`now_ms`, unix milliseconds): the model has no
//! clock, so it runs the same in a browser and a test.

use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex, PoisonError};

use facet::Facet;
use uuid::Uuid;

use crate::admission::Admission;
use crate::model::{Grant, Peer, Scope, endpoint_key};

/// The resource asking to pair is checked as: any proved endpoint may.
pub const PAIR_RESOURCE: &str = "devices/pair";
/// The action asking to pair is checked as.
pub const PAIR_ACTION: &str = "pair";
/// How long an offer lasts: ten minutes.
pub const OFFER_TTL_MS: u64 = 10 * 60 * 1000;
/// How many wrong codes an offer survives.
pub const CODE_TRIES: u8 = 5;
/// What a pairing ticket's text starts with.
pub const TICKET_SCHEME: &str = "fts-pair:";

/// Why pairing did not happen.
#[derive(Debug, Clone, PartialEq, Eq, Facet, thiserror::Error)]
#[repr(u8)]
pub enum PairError {
    /// No live offer matches: a wrong or used code, or one out of time.
    #[error("no pairing offer matches that code")]
    NoOffer,
    /// What the offer would grant is not narrow ([`Scope::is_narrow`]).
    #[error("a pairing grants narrow scopes only: {0}")]
    TooWide(String),
    /// The call did not come over a device's own connection, so there is
    /// no proved endpoint to let in.
    #[error("pairing needs the device's own connection")]
    NotADevice,
    /// The new device's name or platform is not one.
    #[error("{0}")]
    Invalid(String),
}

/// What the new device presents.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum PairProof {
    /// The six digits a person read out.
    Code { code: String },
    /// The ticket's secret ([`PairTicket::secret`]).
    Secret { secret: String },
}

/// The new device's request: its proof, and what it is called.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct PairRequest {
    pub proof: PairProof,
    pub name: String,
    pub platform: String,
}

/// What the new device learns once paired: who let it in, and what it may
/// do there.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Paired {
    /// The offering device's endpoint id: where to dial from now on.
    pub endpoint: String,
    pub name: String,
    pub scopes: Vec<Scope>,
}

/// A QR code's or a link's worth of an offer: the device to dial and the
/// secret to present.
///
/// As text, `fts-pair:<endpoint>#<secret>`; as a web link
/// ([`Self::url`]), `<base>?pair=<endpoint>#<secret>`. Either way the
/// secret is after the `#`, the part a browser never sends.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PairTicket {
    pub endpoint: String,
    pub secret: String,
}

/// Why a string is not a [`PairTicket`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TicketError {
    #[error("not a pairing ticket")]
    NotATicket,
    #[error("the ticket's device is not an endpoint id")]
    Endpoint,
    #[error("the ticket has no secret")]
    Secret,
}

impl PairTicket {
    /// The ticket as a link to `base` (`https://app.example/pair`): the
    /// endpoint in the query, the secret in the fragment.
    #[must_use]
    pub fn url(&self, base: &str) -> String {
        let join = if base.contains('?') { '&' } else { '?' };
        format!("{base}{join}pair={}#{}", self.endpoint, self.secret)
    }

    /// The ticket a link made by [`Self::url`] carries.
    ///
    /// # Errors
    ///
    /// The link has no `pair=` endpoint or no secret after `#`.
    pub fn from_url(url: &str) -> Result<Self, TicketError> {
        let (before, secret) = url.trim().split_once('#').ok_or(TicketError::Secret)?;
        let (_, query) = before.split_once('?').ok_or(TicketError::NotATicket)?;
        let endpoint = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("pair="))
            .ok_or(TicketError::NotATicket)?;
        Self::checked(endpoint, secret)
    }

    fn checked(endpoint: &str, secret: &str) -> Result<Self, TicketError> {
        if !is_endpoint_id(endpoint) {
            return Err(TicketError::Endpoint);
        }
        if !is_secret(secret) {
            return Err(TicketError::Secret);
        }
        Ok(Self {
            endpoint: endpoint.to_ascii_lowercase(),
            secret: secret.to_ascii_lowercase(),
        })
    }

    /// What the new device presents with this ticket.
    #[must_use]
    pub fn proof(&self) -> PairProof {
        PairProof::Secret {
            secret: self.secret.clone(),
        }
    }
}

impl fmt::Display for PairTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{TICKET_SCHEME}{}#{}", self.endpoint, self.secret)
    }
}

impl FromStr for PairTicket {
    type Err = TicketError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s
            .trim()
            .strip_prefix(TICKET_SCHEME)
            .ok_or(TicketError::NotATicket)?;
        let (endpoint, secret) = rest.split_once('#').ok_or(TicketError::Secret)?;
        Self::checked(endpoint, secret)
    }
}

/// An iroh endpoint id as text: 32 bytes, hex.
fn is_endpoint_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A ticket's secret: 64 hex digits.
fn is_secret(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// An offer, as the offering device shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairOffer {
    /// Names the offer for [`Pairings::cancel`].
    pub id: Uuid,
    /// Six digits, to read out.
    pub code: String,
    /// For a QR code or a link.
    pub ticket: PairTicket,
    /// What the paired device will be let do.
    pub scopes: Vec<Scope>,
    /// Unix milliseconds.
    pub expires_ms: u64,
}

struct Live {
    offer: PairOffer,
    tries: u8,
}

/// Who this node is, as a paired device learns it.
#[derive(Debug, Clone)]
struct ThisNode {
    endpoint: String,
    name: String,
}

/// This node's live pairing offers, redeemed into its [`Admission`].
///
/// Cheap to clone; every clone sees the same offers.
#[derive(Clone)]
pub struct Pairings {
    offers: Arc<Mutex<Vec<Live>>>,
    admission: Admission,
    this: ThisNode,
}

impl fmt::Debug for Pairings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pairings")
            .field("endpoint", &self.this.endpoint)
            .finish_non_exhaustive()
    }
}

/// Longest name a paired device may give itself.
const NAME_MAX: usize = 80;
/// Longest platform word.
const PLATFORM_MAX: usize = 24;

impl Pairings {
    /// Offers made by the node at `endpoint` (its iroh endpoint id), called
    /// `name`, admitting into `admission`.
    #[must_use]
    pub fn new(admission: Admission, endpoint: &str, name: impl Into<String>) -> Self {
        Self {
            offers: Arc::default(),
            admission,
            this: ThisNode {
                endpoint: endpoint_key(endpoint),
                name: name.into(),
            },
        }
    }

    fn live(&self) -> std::sync::MutexGuard<'_, Vec<Live>> {
        self.offers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Offer to let a device in with `scopes`, from `now_ms` for
    /// [`OFFER_TTL_MS`].
    ///
    /// # Errors
    ///
    /// [`PairError::TooWide`] when `scopes` is empty or one is not narrow.
    pub fn offer(&self, scopes: Vec<Scope>, now_ms: u64) -> Result<PairOffer, PairError> {
        if scopes.is_empty() {
            return Err(PairError::TooWide("nothing to grant".into()));
        }
        if let Some(wide) = scopes.iter().find(|s| !s.is_narrow()) {
            return Err(PairError::TooWide(format!(
                "{} {:?}",
                wide.resource, wide.actions
            )));
        }
        let mut live = self.live();
        live.retain(|l| l.offer.expires_ms > now_ms);
        let code = loop {
            let code = fresh_code();
            if !live.iter().any(|l| l.offer.code == code) {
                break code;
            }
        };
        let offer = PairOffer {
            id: Uuid::new_v4(),
            code,
            ticket: PairTicket {
                endpoint: self.this.endpoint.clone(),
                secret: fresh_secret(),
            },
            scopes,
            expires_ms: now_ms.saturating_add(OFFER_TTL_MS),
        };
        live.push(Live {
            offer: offer.clone(),
            tries: CODE_TRIES,
        });
        drop(live);
        Ok(offer)
    }

    /// Withdraw an offer before it is used.
    pub fn cancel(&self, id: Uuid) {
        self.live().retain(|l| l.offer.id != id);
    }

    /// The offers still open at `now_ms`.
    #[must_use]
    pub fn open(&self, now_ms: u64) -> Vec<PairOffer> {
        let mut live = self.live();
        live.retain(|l| l.offer.expires_ms > now_ms);
        live.iter().map(|l| l.offer.clone()).collect()
    }

    /// The device at `caller` (the endpoint id its connection proved)
    /// presents `request` at `now_ms`. A match uses the offer up and lets
    /// the device in with its scopes; a miss costs every open offer a try.
    ///
    /// # Errors
    ///
    /// [`PairError::NoOffer`] for a miss, [`PairError::Invalid`] for a
    /// name or platform that is not one.
    pub fn redeem(
        &self,
        caller: &str,
        request: PairRequest,
        now_ms: u64,
    ) -> Result<Paired, PairError> {
        let PairRequest {
            proof,
            name,
            platform,
        } = request;
        let name: String = name.trim().chars().filter(|c| !c.is_control()).collect();
        if name.is_empty() || name.chars().count() > NAME_MAX {
            return Err(PairError::Invalid(
                "a device needs a name of at most 80 characters".into(),
            ));
        }
        let platform = platform.trim().to_ascii_lowercase();
        if platform.is_empty()
            || platform.len() > PLATFORM_MAX
            || !platform
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(PairError::Invalid(
                "platform is a short word: ios, macos, linux…".into(),
            ));
        }
        let offer = {
            let mut live = self.live();
            live.retain(|l| l.offer.expires_ms > now_ms);
            if let Some(at) = live.iter().position(|l| matches(&l.offer, &proof)) {
                Some(live.swap_remove(at).offer)
            } else {
                for l in live.iter_mut() {
                    l.tries = l.tries.saturating_sub(1);
                }
                live.retain(|l| l.tries > 0);
                None
            }
        };
        let Some(offer) = offer else {
            tracing::debug!(caller, "pairing: a proof matched no open offer");
            return Err(PairError::NoOffer);
        };
        self.admission
            .admit(Peer::new(caller, name, platform).granted(Grant::Paired {
                scopes: offer.scopes.clone(),
                by: self.this.name.clone(),
            }));
        Ok(Paired {
            endpoint: self.this.endpoint.clone(),
            name: self.this.name.clone(),
            scopes: offer.scopes,
        })
    }
}

fn matches(offer: &PairOffer, proof: &PairProof) -> bool {
    match proof {
        PairProof::Code { code } => same(offer.code.as_bytes(), code.trim().as_bytes()),
        PairProof::Secret { secret } => same(
            offer.ticket.secret.as_bytes(),
            secret.trim().to_ascii_lowercase().as_bytes(),
        ),
    }
}

/// Compares in time that depends on the lengths only.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Six random digits.
fn fresh_code() -> String {
    let n = Uuid::new_v4().as_u128().checked_rem(1_000_000).unwrap_or(0);
    format!("{n:06}")
}

/// 256 random bits as 64 hex digits (two v4 ids: 244 of them random).
fn fresh_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    const NODE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const NOW: u64 = 1_000_000;

    fn pairings() -> (Admission, Pairings) {
        let admission = Admission::new();
        (admission.clone(), Pairings::new(admission, NODE, "laptop"))
    }

    fn request(proof: PairProof) -> PairRequest {
        PairRequest {
            proof,
            name: "Sam's iPad".into(),
            platform: "iOS".into(),
        }
    }

    fn code(c: &str) -> PairProof {
        PairProof::Code { code: c.into() }
    }

    fn scopes() -> Vec<Scope> {
        vec![Scope::doc(Uuid::from_u128(1), &["read", "write"])]
    }

    #[test]
    fn a_code_lets_its_device_in_once_with_the_offers_scopes() {
        let (admission, pairings) = pairings();
        let offer = pairings.offer(scopes(), NOW).unwrap();
        assert_eq!(offer.code.len(), 6);
        assert!(offer.code.bytes().all(|b| b.is_ascii_digit()));

        let paired = pairings
            .redeem("BB", request(code(&offer.code)), NOW + 1)
            .unwrap();
        assert_eq!(paired.endpoint, NODE);
        assert_eq!(paired.scopes, scopes());
        let peer = admission.get("bb").unwrap();
        assert_eq!(peer.name, "Sam's iPad");
        assert_eq!(peer.platform, "ios");
        assert!(!peer.is_account());

        assert_eq!(
            pairings.redeem("cc", request(code(&offer.code)), NOW + 2),
            Err(PairError::NoOffer),
            "single use"
        );
        assert!(!admission.admits("cc"));
    }

    #[test]
    fn a_ticket_redeems_by_its_secret_and_reads_back_as_text_and_link() {
        let (admission, pairings) = pairings();
        let offer = pairings.offer(scopes(), NOW).unwrap();
        let text = offer.ticket.to_string();
        assert!(text.starts_with("fts-pair:"));
        assert_eq!(text.parse::<PairTicket>().unwrap(), offer.ticket);
        let link = offer.ticket.url("https://app.example/pair");
        let (before, fragment) = link.split_once('#').unwrap();
        assert!(
            !before.contains(&offer.ticket.secret),
            "the secret is in the fragment"
        );
        assert_eq!(fragment, offer.ticket.secret);
        let ticket = PairTicket::from_url(&link).unwrap();
        assert_eq!(ticket, offer.ticket);
        assert!("fts-pair:abc#def".parse::<PairTicket>().is_err());

        pairings.redeem("bb", request(ticket.proof()), NOW).unwrap();
        assert!(admission.admits("bb"));
    }

    #[test]
    fn wrong_codes_use_up_an_offer() {
        let (_, pairings) = pairings();
        let offer = pairings.offer(scopes(), NOW).unwrap();
        let wrong = if offer.code == "000000" {
            "000001"
        } else {
            "000000"
        };
        for _ in 0..CODE_TRIES {
            assert_eq!(
                pairings.redeem("bb", request(code(wrong)), NOW),
                Err(PairError::NoOffer)
            );
        }
        assert_eq!(
            pairings.redeem("bb", request(code(&offer.code)), NOW),
            Err(PairError::NoOffer),
            "out of tries, withdrawn"
        );
    }

    #[test]
    fn an_offer_expires_and_can_be_cancelled() {
        let (_, pairings) = pairings();
        let late = pairings.offer(scopes(), NOW).unwrap();
        assert_eq!(
            pairings.redeem("bb", request(code(&late.code)), NOW + OFFER_TTL_MS),
            Err(PairError::NoOffer)
        );
        let cancelled = pairings.offer(scopes(), NOW).unwrap();
        assert_eq!(pairings.open(NOW).len(), 1);
        pairings.cancel(cancelled.id);
        assert!(pairings.open(NOW).is_empty());
    }

    #[test]
    fn an_offer_grants_narrow_scopes_only() {
        let (_, pairings) = pairings();
        assert!(matches!(
            pairings.offer(vec![Scope::new("**", &["write"])], NOW),
            Err(PairError::TooWide(_))
        ));
        assert!(matches!(
            pairings.offer(Vec::new(), NOW),
            Err(PairError::TooWide(_))
        ));
    }
}
