//! The bytes of a document's file: a header, then records, each framed
//! so a torn write is found and dropped.
//!
//! ```text
//! "ARCHDOC\0"  u32 format             the header (12 bytes)
//! u32 len  payload[len]  u32 crc32    a record, again and again
//! ```
//!
//! A payload is a tag byte and its fields: integers little-endian,
//! strings and byte strings a `u32` length then the bytes, an optional
//! one a `0`/`1` byte then the value. A frame whose length runs past the
//! end, whose checksum is wrong or whose payload does not decode is where
//! the file stops being trusted: everything before it is kept, it and
//! what follows are a write a crash cut short.
//!
//! Pure bytes in, bytes out — no I/O, nothing native — so a browser
//! reads a file it was handed exactly as a desktop does.

use super::FileError;

/// What every document file starts with.
const MAGIC: &[u8; 8] = b"ARCHDOC\0";
/// The layout this build writes and reads.
pub const FORMAT: u32 = 1;
/// The header's length: the magic and the format.
pub const HEADER_LEN: usize = 12;

/// One fact the file keeps. A file is these, in the order they were
/// written; reading them in order rebuilds the document's index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// A `meta` value (`kind`, `schema`, `id`).
    Meta { key: String, value: String },
    /// A blob, by its SHA-256 (lowercase hex).
    Blob {
        hash: String,
        kind: String,
        data: Vec<u8>,
    },
    /// A branch made (or its parent changed): forked from `parent` at
    /// `base` (encoded Loro frontiers); `main` has neither.
    Branch {
        name: String,
        parent: Option<String>,
        base: Option<Vec<u8>>,
        at: i64,
    },
    /// A branch deleted, with everything it kept.
    DropBranch { name: String },
    /// One finished step on a branch: its name, when, the frontiers it
    /// left the branch at, and its Loro update (empty once folded).
    Step {
        seq: u64,
        branch: String,
        label: String,
        at: i64,
        frontiers: Vec<u8>,
        data: Vec<u8>,
    },
    /// A branch folded: its whole state as of step `upto`.
    Snapshot {
        branch: String,
        upto: u64,
        at: i64,
        data: Vec<u8>,
    },
    /// A named point on a branch (a saved version).
    Checkpoint {
        seq: u64,
        branch: String,
        name: String,
        at: i64,
        frontiers: Vec<u8>,
    },
    /// A checkpoint deleted.
    DropCheckpoint { branch: String, name: String },
}

const META: u8 = 1;
const BLOB: u8 = 2;
const BRANCH: u8 = 3;
const DROP_BRANCH: u8 = 4;
const STEP: u8 = 5;
const SNAPSHOT: u8 = 6;
const CHECKPOINT: u8 = 7;
const DROP_CHECKPOINT: u8 = 8;

/// The header a new file starts with.
#[must_use]
pub fn header() -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT.to_le_bytes());
    out
}

/// `record`, framed: what is appended to the file.
pub fn frame(record: &Record) -> Result<Vec<u8>, FileError> {
    let payload = encode(record)?;
    let len = u32::try_from(payload.len()).map_err(|_| too_big())?;
    let mut out = Vec::with_capacity(payload.len().saturating_add(8));
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
    Ok(out)
}

/// What reading a file found.
#[derive(Debug)]
pub struct Parsed {
    pub records: Vec<Record>,
    /// The bytes that are whole records (with the header): where the
    /// file is cut back to when the rest is torn.
    pub good_len: usize,
}

impl Parsed {
    /// Bytes past the last whole record — a write a crash cut short.
    #[must_use]
    pub const fn torn(&self, total: usize) -> usize {
        total.saturating_sub(self.good_len)
    }
}

/// Every whole record in `bytes`.
///
/// # Errors
///
/// `bytes` is not a document file (no header), or one of a newer format.
pub fn parse(bytes: &[u8]) -> Result<Parsed, FileError> {
    let (head, mut rest) = bytes
        .split_at_checked(HEADER_LEN)
        .ok_or_else(|| FileError::NotADocument("shorter than a header".into()))?;
    let (magic, format) = head.split_at(MAGIC.len());
    if magic != MAGIC {
        return Err(FileError::NotADocument("no ARCHDOC header".into()));
    }
    let format = u32::from_le_bytes(format.try_into().map_err(|_| too_big())?);
    if format > FORMAT {
        return Err(FileError::NotADocument(format!(
            "file format {format}, this build reads {FORMAT}"
        )));
    }
    let mut records = Vec::new();
    let mut good_len = HEADER_LEN;
    while let Some((record, used)) = next_frame(rest) {
        records.push(record);
        good_len = good_len.saturating_add(used);
        rest = rest.get(used..).unwrap_or_default();
    }
    Ok(Parsed { records, good_len })
}

/// The record at the start of `bytes` and the bytes its frame took, if
/// it is whole and sound.
fn next_frame(bytes: &[u8]) -> Option<(Record, usize)> {
    let (len, rest) = bytes.split_first_chunk::<4>()?;
    let len = usize::try_from(u32::from_le_bytes(*len)).ok()?;
    let (payload, rest) = rest.split_at_checked(len)?;
    let (crc, _) = rest.split_first_chunk::<4>()?;
    if crc32fast::hash(payload) != u32::from_le_bytes(*crc) {
        return None;
    }
    let record = decode(payload)?;
    Some((record, len.checked_add(8)?))
}

fn too_big() -> FileError {
    FileError::NotADocument("a record larger than 4 GiB".into())
}

fn encode(record: &Record) -> Result<Vec<u8>, FileError> {
    let mut w = Writer(Vec::new());
    match record {
        Record::Meta { key, value } => {
            w.u8(META);
            w.str(key)?;
            w.str(value)?;
        }
        Record::Blob { hash, kind, data } => {
            w.u8(BLOB);
            w.str(hash)?;
            w.str(kind)?;
            w.bytes(data)?;
        }
        Record::Branch {
            name,
            parent,
            base,
            at,
        } => {
            w.u8(BRANCH);
            w.str(name)?;
            w.opt(parent.as_deref().map(str::as_bytes))?;
            w.opt(base.as_deref())?;
            w.i64(*at);
        }
        Record::DropBranch { name } => {
            w.u8(DROP_BRANCH);
            w.str(name)?;
        }
        Record::Step {
            seq,
            branch,
            label,
            at,
            frontiers,
            data,
        } => {
            w.u8(STEP);
            w.u64(*seq);
            w.str(branch)?;
            w.str(label)?;
            w.i64(*at);
            w.bytes(frontiers)?;
            w.bytes(data)?;
        }
        Record::Snapshot {
            branch,
            upto,
            at,
            data,
        } => {
            w.u8(SNAPSHOT);
            w.str(branch)?;
            w.u64(*upto);
            w.i64(*at);
            w.bytes(data)?;
        }
        Record::Checkpoint {
            seq,
            branch,
            name,
            at,
            frontiers,
        } => {
            w.u8(CHECKPOINT);
            w.u64(*seq);
            w.str(branch)?;
            w.str(name)?;
            w.i64(*at);
            w.bytes(frontiers)?;
        }
        Record::DropCheckpoint { branch, name } => {
            w.u8(DROP_CHECKPOINT);
            w.str(branch)?;
            w.str(name)?;
        }
    }
    Ok(w.0)
}

fn decode(payload: &[u8]) -> Option<Record> {
    let mut r = Reader(payload);
    let record = match r.u8()? {
        META => Record::Meta {
            key: r.str()?,
            value: r.str()?,
        },
        BLOB => Record::Blob {
            hash: r.str()?,
            kind: r.str()?,
            data: r.bytes()?,
        },
        BRANCH => Record::Branch {
            name: r.str()?,
            parent: r.opt()?.map(String::from_utf8).transpose().ok()?,
            base: r.opt()?,
            at: r.i64()?,
        },
        DROP_BRANCH => Record::DropBranch { name: r.str()? },
        STEP => Record::Step {
            seq: r.u64()?,
            branch: r.str()?,
            label: r.str()?,
            at: r.i64()?,
            frontiers: r.bytes()?,
            data: r.bytes()?,
        },
        SNAPSHOT => Record::Snapshot {
            branch: r.str()?,
            upto: r.u64()?,
            at: r.i64()?,
            data: r.bytes()?,
        },
        CHECKPOINT => Record::Checkpoint {
            seq: r.u64()?,
            branch: r.str()?,
            name: r.str()?,
            at: r.i64()?,
            frontiers: r.bytes()?,
        },
        DROP_CHECKPOINT => Record::DropCheckpoint {
            branch: r.str()?,
            name: r.str()?,
        },
        _ => return None,
    };
    // A payload with bytes left over is not one this build wrote.
    r.0.is_empty().then_some(record)
}

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) -> Result<(), FileError> {
        let len = u32::try_from(v.len()).map_err(|_| too_big())?;
        self.0.extend_from_slice(&len.to_le_bytes());
        self.0.extend_from_slice(v);
        Ok(())
    }
    fn str(&mut self, v: &str) -> Result<(), FileError> {
        self.bytes(v.as_bytes())
    }
    fn opt(&mut self, v: Option<&[u8]>) -> Result<(), FileError> {
        match v {
            None => {
                self.u8(0);
                Ok(())
            }
            Some(v) => {
                self.u8(1);
                self.bytes(v)
            }
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|[b]| b)
    }
    fn u64(&mut self) -> Option<u64> {
        self.take::<8>().map(u64::from_le_bytes)
    }
    fn i64(&mut self) -> Option<i64> {
        self.take::<8>().map(i64::from_le_bytes)
    }
    fn bytes(&mut self) -> Option<Vec<u8>> {
        let len = usize::try_from(u32::from_le_bytes(self.take::<4>()?)).ok()?;
        let (head, rest) = self.0.split_at_checked(len)?;
        self.0 = rest;
        Some(head.to_vec())
    }
    fn str(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?).ok()
    }
    // The outer `None` is a payload that does not decode; the inner one
    // a field written as absent.
    #[allow(clippy::option_option)]
    fn opt(&mut self) -> Option<Option<Vec<u8>>> {
        match self.u8()? {
            0 => Some(None),
            1 => self.bytes().map(Some),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_record() -> Vec<Record> {
        vec![
            Record::Meta {
                key: "kind".into(),
                value: "studio.song".into(),
            },
            Record::Blob {
                hash: "ab".into(),
                kind: "fx".into(),
                data: vec![1, 2, 3],
            },
            Record::Branch {
                name: "mix B".into(),
                parent: Some("main".into()),
                base: Some(vec![9]),
                at: -5,
            },
            Record::DropBranch { name: "x".into() },
            Record::Step {
                seq: 7,
                branch: "main".into(),
                label: "Split item".into(),
                at: 1,
                frontiers: vec![4],
                data: vec![5; 40],
            },
            Record::Snapshot {
                branch: "main".into(),
                upto: 7,
                at: 2,
                data: vec![6; 10],
            },
            Record::Checkpoint {
                seq: 8,
                branch: "main".into(),
                name: "v1".into(),
                at: 3,
                frontiers: vec![],
            },
            Record::DropCheckpoint {
                branch: "main".into(),
                name: "v1".into(),
            },
        ]
    }

    fn file_of(records: &[Record]) -> Vec<u8> {
        let mut out = header();
        for r in records {
            out.extend(frame(r).unwrap());
        }
        out
    }

    #[test]
    fn every_record_round_trips() {
        let records = every_record();
        let bytes = file_of(&records);
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.records, records);
        assert_eq!(parsed.good_len, bytes.len());
    }

    #[test]
    fn a_torn_or_corrupt_tail_is_cut_off_at_the_last_whole_record() {
        let records = every_record();
        let whole = file_of(&records);
        let last = frame(records.last().unwrap()).unwrap();
        let before_last = whole.len() - last.len();
        // Cut anywhere inside the last frame: the rest is kept.
        for cut in before_last..whole.len() {
            let parsed = parse(&whole[..cut]).unwrap();
            assert_eq!(parsed.records.len(), records.len() - 1, "cut at {cut}");
            assert_eq!(parsed.good_len, before_last);
        }
        // A flipped bit in the last frame's payload fails its checksum.
        let mut flipped = whole;
        flipped[before_last + 6] ^= 0x10;
        assert_eq!(parse(&flipped).unwrap().records.len(), records.len() - 1);
    }

    #[test]
    fn not_a_document_is_refused() {
        assert!(parse(b"").is_err());
        assert!(parse(b"SQLite format 3\0").is_err());
        let mut newer = header();
        newer[8] = 2;
        assert!(parse(&newer).is_err());
    }
}
