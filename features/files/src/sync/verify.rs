//! Proving bytes are there: a blob's Merkle tree, and possession proofs.
//!
//! A scrub reads a whole blob once and gets both its sha-256 (the hash it
//! is filed under) and its Merkle root over [`LEAF`] chunks. A challenge
//! asks a holder for one random chunk and the path from it to that root,
//! so a verifier learns the holder still has the bytes without the file
//! crossing the wire. The tree:
//!
//! - leaf: `sha256(0x00 ‖ chunk)`, chunks of [`LEAF`] bytes (the last one
//!   shorter; an empty blob is one empty leaf);
//! - inner: `sha256(0x01 ‖ left ‖ right)`; an odd node is carried up.

use std::io::Read as _;
use std::path::Path;

use architect::blob::Sha256;
use facet::Facet;
use futures_util::future::BoxFuture;
use sha2::Digest as _;

/// Bytes per leaf.
pub const LEAF: u64 = 1 << 20;

pub type Node = [u8; 32];

/// A leaf's hash.
#[must_use]
pub fn leaf_hash(chunk: &[u8]) -> Node {
    let mut h = sha2::Sha256::new();
    h.update([0u8]);
    h.update(chunk);
    h.finalize().into()
}

fn inner(left: &Node, right: &Node) -> Node {
    let mut h = sha2::Sha256::new();
    h.update([1u8]);
    h.update(left);
    h.update(right);
    h.finalize().into()
}

/// How many leaves a blob of `size` bytes has.
#[must_use]
pub const fn leaf_count(size: u64) -> u64 {
    if size == 0 { 1 } else { size.div_ceil(LEAF) }
}

/// The root over `leaves`.
#[must_use]
pub fn root(leaves: &[Node]) -> Node {
    if leaves.is_empty() {
        return leaf_hash(&[]);
    }
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        level = level
            .chunks(2)
            .map(|pair| match pair {
                [l, r] => inner(l, r),
                [one] => *one,
                _ => unreachable_node(),
            })
            .collect();
    }
    level.first().copied().unwrap_or_else(|| leaf_hash(&[]))
}

const fn unreachable_node() -> Node {
    [0; 32]
}

/// The siblings from leaf `index` up to the root (carried levels have none).
#[must_use]
pub fn path(leaves: &[Node], index: u64) -> Vec<Node> {
    let mut out = Vec::new();
    let mut level = leaves.to_vec();
    let mut i = usize::try_from(index).unwrap_or(usize::MAX);
    while level.len() > 1 {
        let sibling = if i % 2 == 1 {
            level.get(i.saturating_sub(1))
        } else {
            level.get(i.saturating_add(1))
        };
        if let Some(s) = sibling {
            out.push(*s);
        }
        level = level
            .chunks(2)
            .map(|pair| match pair {
                [l, r] => inner(l, r),
                [one] => *one,
                _ => unreachable_node(),
            })
            .collect();
        i /= 2;
    }
    out
}

/// Whether `chunk` is leaf `index` of the `leaves`-leaf tree whose root is `root`.
#[must_use]
pub fn check(root_: &Node, leaves: u64, index: u64, chunk: &[u8], path: &[Node]) -> bool {
    if index >= leaves {
        return false;
    }
    let mut h = leaf_hash(chunk);
    let (mut i, mut width, mut k) = (index, leaves, 0usize);
    while width > 1 {
        if i % 2 == 1 {
            let Some(s) = path.get(k) else { return false };
            h = inner(s, &h);
            k = k.saturating_add(1);
        } else if i.saturating_add(1) < width {
            let Some(s) = path.get(k) else { return false };
            h = inner(&h, s);
            k = k.saturating_add(1);
        }
        i /= 2;
        width = width.div_ceil(2);
    }
    k == path.len() && &h == root_
}

/// A whole file read once: its sha-256 and its leaves.
///
/// # Errors
///
/// The file cannot be read.
pub fn hash_file(path: &Path) -> std::io::Result<(Sha256, Vec<Node>)> {
    let mut file = std::fs::File::open(path)?;
    let mut whole = sha2::Sha256::new();
    let mut leaves = Vec::new();
    let mut buf = vec![0u8; usize::try_from(LEAF).unwrap_or(1 << 20)];
    loop {
        let mut filled = 0;
        while filled < buf.len() {
            let n = file.read(buf.get_mut(filled..).unwrap_or_default())?;
            if n == 0 {
                break;
            }
            filled = filled.saturating_add(n);
        }
        let chunk = buf.get(..filled).unwrap_or_default();
        if filled > 0 || leaves.is_empty() {
            whole.update(chunk);
            leaves.push(leaf_hash(chunk));
        }
        if filled < buf.len() {
            break;
        }
    }
    let digest: [u8; 32] = whole.finalize().into();
    Ok((
        Sha256::from_hex(&super::scan::hex(&digest)).unwrap_or_else(|_| Sha256::of(&[])),
        leaves,
    ))
}

/// Chunk `index` of the file at `path`.
///
/// # Errors
///
/// The file cannot be read.
pub fn read_chunk(path: &Path, index: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Seek as _, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(index.saturating_mul(LEAF)))?;
    let mut out = Vec::new();
    file.take(LEAF).read_to_end(&mut out)?;
    Ok(out)
}

/// Leaves as stored: 32 bytes each, concatenated.
#[must_use]
pub fn encode_leaves(leaves: &[Node]) -> Vec<u8> {
    leaves.iter().flatten().copied().collect()
}

/// [`encode_leaves`] back.
#[must_use]
pub fn decode_leaves(bytes: &[u8]) -> Vec<Node> {
    bytes.as_chunks::<32>().0.to_vec()
}

/// A hex string's bytes.
#[must_use]
pub fn unhex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i.saturating_add(2))?, 16).ok())
        .collect()
}

/// A hex root.
#[must_use]
pub fn node_of(hex: &str) -> Option<Node> {
    unhex(hex).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
}

/// One chunk and its path: the answer to a challenge.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Proof {
    pub chunk: Vec<u8>,
    /// Siblings, leaf to root, each 32 bytes.
    pub path: Vec<Vec<u8>>,
    pub leaves: u64,
}

impl Proof {
    /// Whether this proves leaf `index` under `root_`, for a blob of `size` bytes.
    #[must_use]
    pub fn proves(&self, root_: &Node, size: u64, index: u64) -> bool {
        let expected_len = if size == 0 {
            0
        } else {
            size.saturating_sub(index.saturating_mul(LEAF)).min(LEAF)
        };
        let path: Option<Vec<Node>> = self
            .path
            .iter()
            .map(|p| <[u8; 32]>::try_from(p.as_slice()).ok())
            .collect();
        self.leaves == leaf_count(size)
            && u64::try_from(self.chunk.len()).is_ok_and(|n| n == expected_len)
            && path.is_some_and(|p| check(root_, self.leaves, index, &self.chunk, &p))
    }
}

/// Why a prover gave no proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProveError {
    /// The holder says it does not hold the blob, or cannot read it:
    /// evidence against the copy.
    Lacks(String),
    /// No answer (offline, a dropped link, a timeout): no evidence either way.
    Unreachable(String),
}

impl std::fmt::Display for ProveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lacks(why) => write!(f, "{why}"),
            Self::Unreachable(why) => write!(f, "unreachable: {why}"),
        }
    }
}

impl From<PossessionError> for ProveError {
    fn from(e: PossessionError) -> Self {
        Self::Lacks(e.to_string())
    }
}

/// Something that can prove it holds a blob: a peer over
/// [`Possession`], an S3 bucket's manager, another engine in a test.
pub trait Prover: Send + Sync {
    /// Chunk `index` of `hash`, with its path.
    fn prove(&self, hash: Sha256, index: u64) -> BoxFuture<'_, Result<Proof, ProveError>>;
}

/// Why a holder would not prove.
#[derive(Debug, Clone, PartialEq, Eq, Facet, thiserror::Error)]
#[repr(u8)]
pub enum PossessionError {
    #[error("not a sha256: {0}")]
    BadHash(String),
    /// Not held here, or not one this caller may ask about.
    #[error("not held")]
    NotHeld,
    #[error("{0}")]
    Internal(String),
}

/// Possession proofs over vox: a node serves it on its peer lane.
#[vox::service]
pub trait Possession {
    /// Chunk `index` of blob `hash` (64 hex) and its Merkle path.
    async fn prove(&self, hash: String, index: u64) -> Result<Proof, PossessionError>;
}

/// A peer's [`Possession`] as a [`Prover`].
#[derive(Clone)]
pub struct VoxProver(pub PossessionClient);

impl Prover for VoxProver {
    fn prove(&self, hash: Sha256, index: u64) -> BoxFuture<'_, Result<Proof, ProveError>> {
        Box::pin(async move {
            self.0
                .prove(hash.to_hex(), index)
                .await
                .map_err(|e| match e {
                    vox::VoxError::User(e) => ProveError::from(*e),
                    other => ProveError::Unreachable(format!("{other:?}")),
                })
        })
    }
}

#[cfg(test)]
#[allow(clippy::as_conversions, clippy::string_slice)]
mod tests {
    use super::*;

    fn leaves(n: usize) -> Vec<Node> {
        (0..n)
            .map(|i| leaf_hash(&[u8::try_from(i % 251).unwrap_or(0)]))
            .collect()
    }

    #[test]
    fn every_leaf_proves_under_its_root_for_any_width() {
        for n in 1..=9 {
            let l = leaves(n);
            let r = root(&l);
            for i in 0..n {
                let p = path(&l, i as u64);
                let chunk = [u8::try_from(i % 251).unwrap_or(0)];
                assert!(check(&r, n as u64, i as u64, &chunk, &p), "n={n} i={i}");
                assert!(!check(&r, n as u64, i as u64, &[9, 9], &p));
            }
        }
    }

    #[test]
    fn a_file_hashes_once_into_its_sha_and_its_leaves() {
        let dir = tempfile::tempdir().unwrap_or_else(|_| panic!("tmp"));
        let f = dir.path().join("a.wav");
        let bytes: Vec<u8> = (0..(LEAF * 2 + 17)).map(|i| (i % 253) as u8).collect();
        std::fs::write(&f, &bytes).unwrap_or_default();
        let (sha, l) = hash_file(&f).unwrap_or_else(|_| panic!("read"));
        assert_eq!(sha, Sha256::of(&bytes));
        assert_eq!(l.len(), 3);
        let chunk = read_chunk(&f, 2).unwrap_or_default();
        let proof = Proof {
            chunk,
            path: path(&l, 2).iter().map(|n| n.to_vec()).collect(),
            leaves: 3,
        };
        assert!(proof.proves(&root(&l), bytes.len() as u64, 2));
        assert!(!proof.proves(&root(&l), bytes.len() as u64, 1));
    }

    #[test]
    fn an_empty_file_is_one_empty_leaf() {
        let dir = tempfile::tempdir().unwrap_or_else(|_| panic!("tmp"));
        let f = dir.path().join("e");
        std::fs::write(&f, b"").unwrap_or_default();
        let (_, l) = hash_file(&f).unwrap_or_else(|_| panic!("read"));
        assert_eq!(root(&l), leaf_hash(&[]));
        assert_eq!(leaf_count(0), 1);
    }
}
