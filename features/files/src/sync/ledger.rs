//! The ledger: each holder's signed statements of what it holds.
//!
//! One Loro document per holder per org, `ledger/<org>/<holder>` on the
//! peer lane, written only by that holder (or an S3 bucket's manager):
//!
//! - `holder`: the [`Holder`] record;
//! - `copies`: hash → [`Attestation`];
//! - `checks`: `<holder>:<hash>` → the latest [`Check`] it made of a peer's copy.
//!
//! And one per org, `ledger/<org>/policy`: scope (`""` or a root's place)
//! → [`Policy`]. Every record is JSON in a map entry; attestations and
//! checks carry an ed25519 signature by the holder's key, whose public
//! half is the holder's id, so a relay cannot speak for a holder.

use std::collections::BTreeMap;
use std::path::Path;

use ed25519_dalek::{Signer as _, SigningKey, Verifier as _, VerifyingKey};
use loro::{LoroDoc, LoroValue};
use uuid::Uuid;

use super::verify::unhex;
use crate::copies::{Attestation, Check, Holder, Policy};

const HOLDER: &str = "holder";
const COPIES: &str = "copies";
const CHECKS: &str = "checks";
const POLICY: &str = "policy";

/// A ledger document, by its lane name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LedgerDoc {
    /// `ledger/<org>/<holder>`.
    Holder { org: String, holder: String },
    /// `ledger/<org>/policy`.
    Policy { org: String },
}

impl LedgerDoc {
    /// The lane name.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Holder { org, holder } => format!("ledger/{org}/{holder}"),
            Self::Policy { org } => format!("ledger/{org}/policy"),
        }
    }

    /// The document's id: derived from the name, so every device agrees.
    #[must_use]
    pub fn doc_id(&self) -> Uuid {
        Uuid::new_v5(
            &super::manifest::NAMESPACE,
            format!("architect-files-ledger:{}", self.name()).as_bytes(),
        )
    }

    #[must_use]
    pub fn org(&self) -> &str {
        match self {
            Self::Holder { org, .. } | Self::Policy { org } => org,
        }
    }

    /// The ledger a lane name names, if it is one.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        let rest = name.strip_prefix("ledger/")?;
        let (org, holder) = rest.split_once('/')?;
        if org.is_empty() || holder.is_empty() || org.contains('/') {
            return None;
        }
        Some(if holder == "policy" {
            Self::Policy {
                org: org.to_owned(),
            }
        } else {
            Self::Holder {
                org: org.to_owned(),
                holder: holder.to_owned(),
            }
        })
    }
}

/// A holder's key: signs its records; its public half is its id.
#[derive(Clone)]
pub struct Identity {
    key: SigningKey,
    id: String,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// From an ed25519 secret (a node's iroh key: the same id as its endpoint).
    #[must_use]
    pub fn from_secret(secret: [u8; 32]) -> Self {
        let key = SigningKey::from_bytes(&secret);
        let id = super::scan::hex(key.verifying_key().as_bytes());
        Self { key, id }
    }

    /// The key kept at `path`, made on first use.
    ///
    /// # Errors
    ///
    /// The file cannot be read or written.
    pub fn load_or_create(path: &Path) -> std::io::Result<Self> {
        if let Ok(bytes) = std::fs::read(path)
            && let Ok(secret) = <[u8; 32]>::try_from(bytes.as_slice())
        {
            return Ok(Self::from_secret(secret));
        }
        let secret: [u8; 32] = rand::random();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, secret)?;
        Ok(Self::from_secret(secret))
    }

    /// The holder id: the public key, hex.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// `message`'s signature, hex.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> String {
        super::scan::hex(&self.key.sign(message).to_bytes())
    }
}

/// Whether `sig` (hex) is `signer`'s (a holder id) over `message`.
#[must_use]
pub fn verify(signer: &str, message: &[u8], sig: &str) -> bool {
    let Some(key) = unhex(signer).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()) else {
        return false;
    };
    let Some(sig) = unhex(sig).and_then(|b| <[u8; 64]>::try_from(b.as_slice()).ok()) else {
        return false;
    };
    VerifyingKey::from_bytes(&key).is_ok_and(|k| {
        k.verify(message, &ed25519_dalek::Signature::from_bytes(&sig))
            .is_ok()
    })
}

/// A holder's ledger document, read.
#[derive(Debug, Clone, Default)]
pub struct HolderLedger {
    pub holder: Option<Holder>,
    pub copies: BTreeMap<String, Attestation>,
    /// `(holder challenged, hash)` → the check.
    pub checks: BTreeMap<(String, String), Check>,
}

fn json_entries(doc: &LoroDoc, map: &str) -> Vec<(String, String)> {
    let LoroValue::Map(entries) = doc.get_map(map).get_value() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(k, v)| match v {
            LoroValue::String(s) => Some((k.clone(), s.to_string())),
            _ => None,
        })
        .collect()
}

/// Read a holder's ledger.
#[must_use]
pub fn read(doc: &LoroDoc) -> HolderLedger {
    let holder = json_entries(doc, HOLDER)
        .into_iter()
        .find(|(k, _)| k == "json")
        .and_then(|(_, v)| facet_json::from_str::<Holder>(&v).ok());
    let copies = json_entries(doc, COPIES)
        .into_iter()
        .filter_map(|(k, v)| Some((k, facet_json::from_str::<Attestation>(&v).ok()?)))
        .collect();
    let checks = json_entries(doc, CHECKS)
        .into_iter()
        .filter_map(|(_, v)| {
            let c = facet_json::from_str::<Check>(&v).ok()?;
            Some(((c.holder.clone(), c.hash.clone()), c))
        })
        .collect();
    HolderLedger {
        holder,
        copies,
        checks,
    }
}

/// Writes to one's own ledger: nothing is committed until [`Writer::commit`].
pub struct Writer<'a> {
    doc: &'a LoroDoc,
    org: &'a str,
    identity: &'a Identity,
    /// Whose records these are: this device, or an S3 bucket it manages.
    holder: &'a str,
    changed: bool,
}

impl<'a> Writer<'a> {
    #[must_use]
    pub const fn new(
        doc: &'a LoroDoc,
        org: &'a str,
        identity: &'a Identity,
        holder: &'a str,
    ) -> Self {
        Self {
            doc,
            org,
            identity,
            holder,
            changed: false,
        }
    }

    /// Say who the holder is (only when it changed).
    pub fn holder(&mut self, holder: &Holder) {
        let json = facet_json::to_string(holder).unwrap_or_default();
        let map = self.doc.get_map(HOLDER);
        let same = map
            .get("json")
            .and_then(|v| v.into_value().ok())
            .is_some_and(|v| matches!(v, LoroValue::String(s) if *s == json));
        if !same {
            let _ = map.insert("json", json);
            self.changed = true;
        }
    }

    /// Sign and put an attestation.
    pub fn put(&mut self, mut att: Attestation) {
        att.sig = self.identity.sign(&att.message(self.org, self.holder));
        let _ = self.doc.get_map(COPIES).insert(
            &att.hash.clone(),
            facet_json::to_string(&att).unwrap_or_default(),
        );
        self.changed = true;
    }

    /// Withdraw the attestation of `hash`.
    pub fn remove(&mut self, hash: &str) {
        let map = self.doc.get_map(COPIES);
        if map.get(hash).is_some() {
            let _ = map.delete(hash);
            self.changed = true;
        }
    }

    /// Sign and put a check of a peer's copy.
    pub fn check(&mut self, mut check: Check) {
        check.sig = self
            .identity
            .sign(&check.message(self.org, self.identity.id()));
        let key = format!("{}:{}", check.holder, check.hash);
        let _ = self
            .doc
            .get_map(CHECKS)
            .insert(&key, facet_json::to_string(&check).unwrap_or_default());
        self.changed = true;
    }

    /// Commit what was written, as one change.
    pub fn commit(self) {
        if self.changed {
            self.doc.commit();
        }
    }
}

/// An org's policies: scope → policy.
#[must_use]
pub fn policies(doc: &LoroDoc) -> BTreeMap<String, Policy> {
    json_entries(doc, POLICY)
        .into_iter()
        .filter_map(|(k, v)| Some((k, facet_json::from_str::<Policy>(&v).ok()?)))
        .collect()
}

/// Set (or with `None`, clear) the policy of `scope`.
pub fn set_policy(doc: &LoroDoc, scope: &str, policy: Option<Policy>) {
    let map = doc.get_map(POLICY);
    let _ = policy.map_or_else(
        || map.delete(scope),
        |p| map.insert(scope, facet_json::to_string(&p).unwrap_or_default()),
    );
    doc.commit();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copies::{CopyState, HolderKind};

    #[test]
    fn a_ledger_name_round_trips() {
        let h = LedgerDoc::Holder {
            org: "tombrooksmusic".into(),
            holder: "ab12".into(),
        };
        assert_eq!(LedgerDoc::parse(&h.name()), Some(h.clone()));
        let p = LedgerDoc::Policy { org: "cbu".into() };
        assert_eq!(LedgerDoc::parse(&p.name()), Some(p));
        assert_eq!(LedgerDoc::parse("files/cbu/Projects/x"), None);
        assert_ne!(
            h.doc_id(),
            LedgerDoc::Policy {
                org: "tombrooksmusic".into()
            }
            .doc_id()
        );
    }

    #[test]
    fn a_record_verifies_by_its_holder_s_id_and_not_once_changed_or_moved() {
        let me = Identity::from_secret([7; 32]);
        let doc = LoroDoc::new();
        let mut w = Writer::new(&doc, "cbu", &me, me.id());
        w.holder(&Holder {
            id: me.id().to_owned(),
            kind: HolderKind::Device,
            name: "laptop".into(),
            site: "home".into(),
            manager: None,
        });
        w.put(Attestation {
            hash: "aa".into(),
            size: 3,
            state: CopyState::Full,
            verified_at_us: 5,
            merkle: String::new(),
            have: 3,
            at_us: 5,
            sig: String::new(),
        });
        w.commit();
        let read = read(&doc);
        let att = read
            .copies
            .get("aa")
            .cloned()
            .unwrap_or_else(|| panic!("put"));
        assert!(verify(me.id(), &att.message("cbu", me.id()), &att.sig));
        assert!(!verify(
            me.id(),
            &att.message("other-org", me.id()),
            &att.sig
        ));
        let forged = Attestation {
            size: 4,
            ..att.clone()
        };
        assert!(!verify(me.id(), &forged.message("cbu", me.id()), &att.sig));
        let other = Identity::from_secret([8; 32]);
        assert!(!verify(other.id(), &att.message("cbu", me.id()), &att.sig));
    }
}
