//! An S3 bucket as a holder in the ledger (feature `s3`): what a node
//! that stores blobs in a bucket (`architect::blob::S3Store`) attests for
//! it, signed by the node. The spec: `docs/content/architecture/file-registry.md`.

use architect::blob::{HolderEvents, S3Store, Sha256};

use super::engine::Engine;
use crate::copies::{Attestation, CopyState, Holder, HolderKind};

/// The node's bucket as a holder in the ledger: `kind: s3`, its id
/// [`S3Store::holder_id`], managed (and signed for) by this node. The
/// store's [`HolderEvents`] are what it attests:
///
/// - `held`: `full`, verified now: `S3Store::put_reader` places only
///   bytes that hashed to their name;
/// - `dropped`, or a `verified` that failed: withdrawn;
/// - `verified` that passed: re-stamped.
pub struct BucketHolder {
    engine: Engine,
    holder: Holder,
}

impl BucketHolder {
    /// The holder for `s3`, at the site its endpoint names (`aws` without one).
    #[must_use]
    pub fn new(engine: Engine, s3: &S3Store, endpoint: Option<&str>) -> Self {
        let site = endpoint
            .and_then(|e| e.split("://").nth(1).or(Some(e)))
            .and_then(|e| e.split(['/', ':']).next())
            .filter(|h| !h.is_empty())
            .unwrap_or("aws")
            .to_owned();
        Self {
            holder: Holder {
                id: s3.holder_id().to_owned(),
                kind: HolderKind::S3,
                name: format!("s3 {}", s3.bucket()),
                site,
                manager: None,
            },
            engine,
        }
    }

    fn attest(&self, hash: Sha256, size: Option<u64>) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (engine, holder) = (self.engine.clone(), self.holder.clone());
        rt.spawn(async move {
            let hex = hash.to_hex();
            match size {
                Some(size) => {
                    let now = super::engine::now_us();
                    let att = Attestation {
                        hash: hex,
                        size,
                        state: CopyState::Full,
                        verified_at_us: now,
                        merkle: String::new(),
                        have: size,
                        at_us: now,
                        sig: String::new(),
                    };
                    engine.attest_external(holder, vec![att], Vec::new()).await;
                }
                None => engine.attest_external(holder, Vec::new(), vec![hex]).await,
            }
        });
    }
}

impl HolderEvents for BucketHolder {
    fn held(&self, _holder: &str, hash: Sha256, size: u64) {
        self.attest(hash, Some(size));
    }

    fn dropped(&self, _holder: &str, hash: Sha256) {
        self.attest(hash, None);
    }

    fn verified(&self, _holder: &str, hash: Sha256, ok: bool) {
        if ok {
            // Read back whole and right: its record, verified now.
            let (engine, holder) = (self.engine.clone(), self.holder.clone());
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move { engine.restamp_external(&holder, &hash.to_hex()).await });
            }
        } else {
            self.attest(hash, None);
        }
    }
}
