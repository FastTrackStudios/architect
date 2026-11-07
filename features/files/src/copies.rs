//! Who holds which bytes, and how sure we are: the ledger as data.
//!
//! A holder (a device, the headless node, an S3 bucket) publishes a
//! signed [`Attestation`] per blob it holds; verifiers publish signed
//! [`Check`]s of the copies they challenged. A copy is counted only on
//! fresh evidence ([`confidence`]). Wasm-clean; the engine is
//! `crate::sync`. The spec: `docs/content/architecture/file-registry.md`.

use facet::Facet;

/// What stores blobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(C)]
pub enum HolderKind {
    /// A laptop's or a desk's agent.
    Device,
    /// The headless node, holding every root in place.
    Node,
    /// An S3 or Garage bucket used as a blob backend; its records are
    /// signed by the device that manages it ([`Holder::manager`]).
    S3,
}

impl HolderKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Node => "node",
            Self::S3 => "s3",
        }
    }
}

/// A holder, as its own ledger names it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Holder {
    /// A device's or node's iroh endpoint id (the hex of its ed25519
    /// public key, which verifies its records); `s3:<endpoint>/<bucket>`.
    pub id: String,
    pub kind: HolderKind,
    /// Shown: the device's name, the bucket's.
    pub name: String,
    /// Where it is (`home`, `starcommand`, `backblaze`): two copies at
    /// one site are not off-site.
    pub site: String,
    /// The holder whose key signs this one's records (an S3 bucket's
    /// manager); `None` when it signs its own.
    #[facet(default)]
    pub manager: Option<String>,
}

impl Holder {
    /// The id whose key verifies this holder's records.
    #[must_use]
    pub fn signer(&self) -> &str {
        self.manager.as_deref().unwrap_or(&self.id)
    }
}

/// How a holder holds a blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(C)]
pub enum CopyState {
    /// Held deliberately: a pinned working copy, a root held in place,
    /// the node's version store.
    Full,
    /// Fetched on open into the blob cache; evictable.
    Cached,
    /// A download in progress ([`Attestation::have`] bytes so far).
    Partial,
    /// Being removed.
    Evicting,
}

impl CopyState {
    /// Whether a copy in this state can count at all.
    #[must_use]
    pub const fn counts(self) -> bool {
        matches!(self, Self::Full | Self::Cached)
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Cached => "cached",
            Self::Partial => "partial",
            Self::Evicting => "evicting",
        }
    }
}

/// "I hold blob `hash`, in `state`, last verified at `verified_at_us`."
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Attestation {
    /// sha-256, hex.
    pub hash: String,
    pub size: u64,
    pub state: CopyState,
    /// When the holder last read the whole blob and found its sha-256
    /// right (a scrub, a fetch, a commit), µs since the epoch; 0 never.
    pub verified_at_us: i64,
    /// The blob's Merkle root (hex) over 1 MiB chunks, once a scrub made it.
    #[facet(default)]
    pub merkle: String,
    /// Bytes held: `size` unless [`CopyState::Partial`].
    pub have: u64,
    /// When this record was written.
    pub at_us: i64,
    /// The signer's ed25519 signature over [`Self::message`], hex.
    #[facet(default)]
    pub sig: String,
}

impl Attestation {
    /// What is signed: every field but the signature, bound to the org
    /// and the holder, so a record cannot be replayed into another ledger.
    #[must_use]
    pub fn message(&self, org: &str, holder: &str) -> Vec<u8> {
        format!(
            "architect-files/attest/v1\n{org}\n{holder}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.hash,
            self.size,
            self.state.label(),
            self.verified_at_us,
            self.merkle,
            self.have,
            self.at_us
        )
        .into_bytes()
    }
}

/// "I challenged `holder` for chunk `chunk` of `hash` at `at_us`."
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Check {
    /// The holder challenged.
    pub holder: String,
    pub hash: String,
    pub at_us: i64,
    pub passed: bool,
    pub chunk: u64,
    /// Whose Merkle root the answer was checked against (a holder id).
    pub root_from: String,
    /// Why it failed, when it did.
    #[facet(default)]
    pub why: String,
    #[facet(default)]
    pub sig: String,
}

impl Check {
    /// What is signed, bound to the org and the verifier.
    #[must_use]
    pub fn message(&self, org: &str, by: &str) -> Vec<u8> {
        format!(
            "architect-files/check/v1\n{org}\n{by}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.holder, self.hash, self.at_us, self.passed, self.chunk, self.root_from, self.why
        )
        .into_bytes()
    }
}

/// How sure this device is of one copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(C)]
pub enum Confidence {
    /// Fresh evidence: a scrub by its holder or a passed challenge, within
    /// the window, and no failed challenge since. Only these are counted.
    Verified,
    /// The newest evidence is older than the window: "unverified since".
    Stale,
    /// No evidence, a bad signature, a size that disagrees, or a state
    /// that never counts (partial, evicting).
    Unknown,
    /// A challenge failed after the last good evidence.
    Failed,
}

impl Confidence {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
            Self::Failed => "failed",
        }
    }
}

/// The verdict on one copy: [`confidence`]'s answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub confidence: Confidence,
    /// The newest good evidence, µs; 0 none.
    pub evidence_at_us: i64,
    /// `scrub`, or `challenge by <holder>`.
    pub how: String,
}

/// How sure we are of `att`, given the checks of it (`(verifier, check)`
/// whose signatures verified), at `now_us` with a window of `window_us`.
///
/// Signatures are checked before this is asked: an attestation whose
/// signature does not verify is never passed in.
#[must_use]
pub fn confidence<'a>(
    att: &Attestation,
    expected_size: Option<u64>,
    checks: impl IntoIterator<Item = (&'a str, &'a Check)>,
    now_us: i64,
    window_us: i64,
) -> Verdict {
    let unknown = |how: &str| Verdict {
        confidence: Confidence::Unknown,
        evidence_at_us: 0,
        how: how.to_owned(),
    };
    if !att.state.counts() {
        return unknown(att.state.label());
    }
    if expected_size.is_some_and(|s| s != att.size) || att.have < att.size {
        return unknown("size disagrees");
    }
    let (mut at, mut how) = (att.verified_at_us, "scrub".to_owned());
    let mut failed_at = 0i64;
    let mut failed_by = String::new();
    for (by, c) in checks {
        if c.passed && c.at_us > at {
            at = c.at_us;
            how = format!("challenge by {by}");
        }
        if !c.passed && c.at_us > failed_at {
            failed_at = c.at_us;
            failed_by = format!("challenge by {by} failed: {}", c.why);
        }
    }
    if failed_at > at {
        return Verdict {
            confidence: Confidence::Failed,
            evidence_at_us: at,
            how: failed_by,
        };
    }
    if at <= 0 {
        return unknown("never verified");
    }
    let confidence = if now_us.saturating_sub(at) <= window_us {
        Confidence::Verified
    } else {
        Confidence::Stale
    };
    Verdict {
        confidence,
        evidence_at_us: at,
        how,
    }
}

/// A minimum of copies: per org (`""` scope) or per root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct Policy {
    /// Verified copies.
    pub min_verified: u32,
    /// Distinct sites among the verified copies (2 = one off-site).
    pub min_sites: u32,
}

impl Default for Policy {
    /// When none is set: two verified copies, anywhere.
    fn default() -> Self {
        Self {
            min_verified: 2,
            min_sites: 1,
        }
    }
}

impl Policy {
    /// What `verified` copies at `sites` sites lack, if anything.
    #[must_use]
    pub fn lacks(&self, verified: u32, sites: u32) -> Option<String> {
        let mut out = Vec::new();
        if verified < self.min_verified {
            out.push(format!(
                "{verified} of {} verified copies",
                self.min_verified
            ));
        }
        if sites < self.min_sites {
            out.push(format!("{sites} of {} sites", self.min_sites));
        }
        (!out.is_empty()).then(|| out.join(", "))
    }
}

/// One copy of a blob, as this device sees it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Copy {
    pub holder: Holder,
    pub state: CopyState,
    pub confidence: Confidence,
    /// The newest good evidence, µs; 0 none ("unverified since" when stale).
    pub verified_at_us: i64,
    /// `scrub`, `challenge by <holder>`, or why it is not counted.
    pub how: String,
    /// The holder is this device, or one it is connected to now.
    pub reachable: bool,
    /// This device.
    pub here: bool,
}

/// A file in a root.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Place {
    pub org: String,
    /// The root's place (`<org>/<section>/<name>`).
    pub root: String,
    pub path: String,
}

/// Where a blob's bytes are.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Locations {
    pub hash: String,
    pub size: u64,
    /// Every live path holding these bytes, in the roots this device sees.
    pub places: Vec<Place>,
    pub copies: Vec<Copy>,
    /// Copies counted: [`Confidence::Verified`].
    pub verified: u32,
    /// Distinct sites among them.
    pub sites: u32,
    /// The strictest policy over its places.
    pub policy: Policy,
    /// What it lacks under that policy, if anything.
    #[facet(default)]
    pub lacks: Option<String>,
}

/// A file under its policy.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Violation {
    pub root: String,
    pub path: String,
    pub hash: String,
    pub size: u64,
    pub verified: u32,
    pub sites: u32,
    pub lacks: String,
}

/// One holder's share of a scope.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct HolderShare {
    pub holder: Holder,
    pub files: u64,
    pub bytes: u64,
    /// Of `bytes`, those whose copy here is verified.
    pub verified_bytes: u64,
    pub reachable: bool,
}

/// An org's or a root's copies: the storage health view.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct CopyReport {
    /// An org slug, a root's place, or `""` for everything.
    pub scope: String,
    pub policy: Policy,
    pub files: u64,
    pub bytes: u64,
    /// Files (and their bytes) at fewer verified copies than the policy's minimum.
    pub under_files: u64,
    pub under_bytes: u64,
    /// Bytes no more than one holder has (verified or not).
    pub single_holder_bytes: u64,
    /// Bytes with no verified copy at all.
    pub unverified_bytes: u64,
    pub holders: Vec<HolderShare>,
    /// The first files under policy (at most 500), fewest copies first.
    pub violations: Vec<Violation>,
    /// How many files are under policy in all.
    pub violation_count: u64,
}

/// What a scrub did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct ScrubReport {
    /// Blobs re-hashed here, and their bytes.
    pub scrubbed: u32,
    pub bytes: u64,
    /// Blobs whose bytes were wrong here (removed from the count).
    pub corrupt: Vec<String>,
    /// Copies of peers challenged, and how many passed.
    pub challenged: u32,
    pub passed: u32,
    /// `<holder> <hash>` of every challenge that failed.
    pub failed: Vec<String>,
}

/// A holder this device knows, and whether it is reachable now.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct HolderStatus {
    pub holder: Holder,
    pub reachable: bool,
    pub here: bool,
    /// Copies it attests in the orgs this device sees.
    pub copies: u64,
    pub bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(verified: i64) -> Attestation {
        Attestation {
            hash: "h".into(),
            size: 10,
            state: CopyState::Full,
            verified_at_us: verified,
            merkle: String::new(),
            have: 10,
            at_us: verified,
            sig: String::new(),
        }
    }

    fn check(at: i64, passed: bool) -> Check {
        Check {
            holder: "x".into(),
            hash: "h".into(),
            at_us: at,
            passed,
            chunk: 0,
            root_from: "v".into(),
            why: if passed {
                String::new()
            } else {
                "bad chunk".into()
            },
            sig: String::new(),
        }
    }

    #[test]
    fn a_fresh_scrub_is_verified_and_an_old_one_stale() {
        let v = confidence(&att(100), Some(10), [], 150, 100);
        assert_eq!(v.confidence, Confidence::Verified);
        let v = confidence(&att(100), Some(10), [], 500, 100);
        assert_eq!(v.confidence, Confidence::Stale);
        assert_eq!(v.evidence_at_us, 100);
    }

    #[test]
    fn a_passed_challenge_refreshes_and_a_failed_one_wins_over_older_evidence() {
        let ok = check(450, true);
        let v = confidence(&att(100), Some(10), [("v", &ok)], 500, 100);
        assert_eq!(v.confidence, Confidence::Verified);
        assert_eq!(v.how, "challenge by v");
        let bad = check(480, false);
        let v = confidence(&att(100), Some(10), [("v", &ok), ("w", &bad)], 500, 100);
        assert_eq!(v.confidence, Confidence::Failed);
    }

    #[test]
    fn never_verified_partial_or_wrong_size_is_unknown() {
        assert_eq!(
            confidence(&att(0), Some(10), [], 1, 100).confidence,
            Confidence::Unknown
        );
        assert_eq!(
            confidence(&att(1), Some(11), [], 1, 100).confidence,
            Confidence::Unknown
        );
        let partial = Attestation {
            state: CopyState::Partial,
            have: 3,
            ..att(1)
        };
        assert_eq!(
            confidence(&partial, Some(10), [], 1, 100).confidence,
            Confidence::Unknown
        );
    }

    #[test]
    fn a_policy_names_what_is_missing() {
        let p = Policy {
            min_verified: 2,
            min_sites: 2,
        };
        assert_eq!(p.lacks(2, 2), None);
        assert_eq!(
            p.lacks(1, 1).as_deref(),
            Some("1 of 2 verified copies, 1 of 2 sites")
        );
    }
}
