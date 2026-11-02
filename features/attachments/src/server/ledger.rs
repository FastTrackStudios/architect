//! One org's ledger: what its attachments hold, uploads under way, and
//! each record's list — `ledger.json` beside the uploads' staged bytes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use architect::blob::Sha256;
use facet::Facet;

use super::io;
use crate::model::{AttachFault, AttachUsage, Attachment};

/// Bytes the org's attachments hold.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub(super) struct Held {
    pub hash: String,
    pub size: u64,
    /// When the org first held them.
    pub since_us: i64,
}

/// An upload under way: its size, reserved against the quota.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub(super) struct Pending {
    pub hash: String,
    pub size: u64,
}

/// One record's attachments.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub(super) struct RecordList {
    /// The parent's text (`task:T-1`).
    pub parent: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub(super) struct Ledger {
    pub quota: Option<u64>,
    pub held: Vec<Held>,
    pub pending: Vec<Pending>,
    pub records: Vec<RecordList>,
}

impl Ledger {
    const FILE: &'static str = "ledger.json";

    pub async fn load(dir: &Path) -> Result<Self, AttachFault> {
        match tokio::fs::read_to_string(dir.join(Self::FILE)).await {
            Ok(json) => facet_json::from_str(&json)
                .map_err(|e| AttachFault::Internal(format!("the attachments ledger: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(io(e)),
        }
    }

    /// Bytes held and reserved.
    pub fn used(&self) -> u64 {
        self.held
            .iter()
            .map(|h| h.size)
            .chain(self.pending.iter().map(|p| p.size))
            .fold(0, u64::saturating_add)
    }

    pub fn usage(&self) -> AttachUsage {
        AttachUsage {
            used: self.used(),
            quota: self.quota,
        }
    }

    /// Whether `adding` more bytes fit.
    pub fn room_for(&self, adding: u64) -> Result<(), AttachFault> {
        let used = self.used();
        match self.quota {
            Some(quota) if used.saturating_add(adding) > quota => Err(AttachFault::QuotaExceeded {
                used,
                quota,
                adding,
            }),
            _ => Ok(()),
        }
    }

    pub fn held(&self, hash: &str) -> Option<&Held> {
        self.held.iter().find(|h| h.hash == hash)
    }

    pub fn pending(&self, hash: &str) -> Option<&Pending> {
        self.pending.iter().find(|p| p.hash == hash)
    }

    pub fn hold(&mut self, hash: &str, size: u64, now_us: i64) {
        self.pending.retain(|p| p.hash != hash);
        if self.held(hash).is_none() {
            self.held.push(Held {
                hash: hash.to_owned(),
                size,
                since_us: now_us,
            });
        }
    }

    pub fn record(&self, parent: &str) -> &[Attachment] {
        self.records
            .iter()
            .find(|r| r.parent == parent)
            .map_or(&[], |r| r.attachments.as_slice())
    }

    /// Set a record's list (an empty one removes it).
    pub fn set_record(&mut self, parent: &str, attachments: Vec<Attachment>) {
        self.records.retain(|r| r.parent != parent);
        if !attachments.is_empty() {
            self.records.push(RecordList {
                parent: parent.to_owned(),
                attachments,
            });
        }
    }

    pub fn held_hashes(&self) -> HashSet<Sha256> {
        self.held
            .iter()
            .filter_map(|h| Sha256::from_hex(&h.hash).ok())
            .collect()
    }

    /// Drop what is held, not in `live`, on no record, and older than
    /// `before_us`.
    pub fn release(&mut self, live: &HashSet<Sha256>, before_us: i64) -> Vec<Sha256> {
        let on_records: HashSet<&str> = self
            .records
            .iter()
            .flat_map(|r| r.attachments.iter().map(|a| a.hash.as_str()))
            .collect();
        let (gone, kept): (Vec<Held>, Vec<Held>) =
            std::mem::take(&mut self.held).into_iter().partition(|h| {
                let hash = Sha256::from_hex(&h.hash).ok();
                h.since_us < before_us
                    && !on_records.contains(h.hash.as_str())
                    && hash.is_some_and(|hash| !live.contains(&hash))
            });
        self.held = kept;
        gone.iter()
            .filter_map(|h| Sha256::from_hex(&h.hash).ok())
            .collect()
    }
}

/// An org's ledger, loaded, and where it lives.
pub(super) struct OrgLedger {
    pub dir: PathBuf,
    pub state: tokio::sync::Mutex<Ledger>,
}

impl OrgLedger {
    pub fn new(dir: PathBuf, state: Ledger) -> Self {
        Self {
            dir,
            state: tokio::sync::Mutex::new(state),
        }
    }

    /// Where an upload's bytes are staged.
    pub fn staging(&self, hash: &str) -> PathBuf {
        self.dir.join("uploads").join(format!("{hash}.part"))
    }

    /// Write `state` (atomically: a temporary file renamed over).
    pub async fn save(&self, state: &Ledger) -> Result<(), AttachFault> {
        let json = facet_json::to_string(state)
            .map_err(|e| AttachFault::Internal(format!("the attachments ledger: {e}")))?;
        tokio::fs::create_dir_all(&self.dir).await.map_err(io)?;
        let tmp = self.dir.join("ledger.json.tmp");
        tokio::fs::write(&tmp, json).await.map_err(io)?;
        tokio::fs::rename(&tmp, self.dir.join(Ledger::FILE))
            .await
            .map_err(io)
    }
}
