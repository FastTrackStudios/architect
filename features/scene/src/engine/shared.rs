//! A table in shared memory: an engine in a process of its own.
//!
//! [`EngineTable`] keeps its slots in the host's memory, so only an engine
//! running in the host can write them. An engine that is a process of its
//! own (a DAW, a lighting renderer) makes its table here instead: a file
//! both processes map, the same slots, written the same way (two atomic
//! stores). The host maps it, reads it at compose, and watches it for
//! wakes as it does a table of its own.
//!
//! The table carries the host's values too: an engine lists `host.`
//! names beside its `engine.` ones, and a drag on a pane reading the
//! table writes the value it makes into it as it makes it
//! ([`EngineTable::write_host`]), where the engine reads it lock-free
//! ([`super::EngineWriter::read`]). A fader moves the engine's gain while
//! it is held, with no message to anyone.
//!
//! **The file.** Little-endian, every field 8-byte aligned:
//!
//! ```text
//! 0   magic    b"ASEV\x01\0\0\0"
//! 8   u64      slot count N (at most MAX_SLOTS)
//! 16  u64      names' length L, in bytes
//! 24  u64      generation (atomic)
//! 32  N × u64  the slots (atomic f64 bits; u64::MAX unwritten)
//! ..  L bytes  the names, UTF-8, one per slot in slot order, '\n'-separated
//! ```
//!
//! **Lifetime.** [`create`] writes the whole file under a temporary name
//! and renames it into place, so a mapping of the previous file is never
//! truncated under its reader; an engine that restarts makes a new file,
//! and [`open_engine`] notices (the path names another file) and maps
//! that. The engine never truncates a file it published.
//!
//! **Trust.** Only a process extension writes one, and a process
//! extension is native code running as the person already, so a mapping
//! it could corrupt is no new reach. The host still checks the header,
//! every name and the file's length before it maps, and every value on
//! read (finite, in range), as for a table of its own.
//!
//! In a browser the same layout is a `SharedArrayBuffer` an engine in an
//! audio worklet writes (not built).

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, PoisonError};

use super::{EngineTable, EngineWriter, MAX_SLOTS, Store, UNSET};
use crate::Error;
use crate::anim::{Name, Space};

/// What a shared table's file starts with.
pub const MAGIC: [u8; 8] = *b"ASEV\x01\0\0\0";

const HEADER: usize = 32;

/// The most bytes of names a table may carry.
pub const MAX_NAME_BYTES: usize = MAX_SLOTS * 65;

/// A file mapped into this process, holding a table's slots.
pub(super) struct Mapping {
    map: memmap2::MmapRaw,
    count: usize,
}

/// Where slot `at` is in a table's file.
const fn slot_offset(at: usize) -> usize {
    HEADER.saturating_add(at.saturating_mul(8))
}

impl Mapping {
    /// The atomic word at `offset`, which must be 8-aligned and inside
    /// the mapping (`slot` and `generation` only ask for those).
    #[inline]
    #[expect(
        clippy::cast_ptr_alignment,
        reason = "a mapping is page-aligned and every offset asked for is a multiple of 8"
    )]
    fn word(&self, offset: usize) -> &AtomicU64 {
        // SAFETY: the mapping holds at least HEADER + 8 × count bytes
        // (checked before it was made, and never truncated: the module's
        // lifetime rule), `offset` is within that and a multiple of 8 on
        // a page-aligned mapping, so this is an aligned u64 living as long
        // as `self`. Every access to it, from either process, is atomic.
        unsafe { AtomicU64::from_ptr(self.map.as_mut_ptr().add(offset).cast::<u64>()) }
    }

    #[inline]
    pub(super) fn slot(&self, at: usize) -> Option<&AtomicU64> {
        (at < self.count).then(|| self.word(slot_offset(at)))
    }

    #[inline]
    pub(super) fn generation(&self) -> &AtomicU64 {
        self.word(24)
    }
}

fn denied(what: impl std::fmt::Display) -> Error {
    Error::Denied("share", what.to_string())
}

/// The directory engines publish their tables in: the session's runtime
/// directory (`$XDG_RUNTIME_DIR`), else the temporary one, under
/// `architect-engines`.
#[must_use]
pub fn directory() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
        .join("architect-engines")
}

/// Whether `id` may name a table's file: an extension id's characters.
fn plain(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

/// Where engine `id` (its extension's id) publishes its table.
///
/// # Errors
///
/// `id` is not an extension id.
pub fn path_of(id: &str) -> Result<PathBuf, Error> {
    if !plain(id) {
        return Err(denied(format!("engine id {id:?}")));
    }
    Ok(directory().join(format!("{id}.values")))
}

fn u64_of(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// A table's file: its header, unwritten slots and names.
fn file_bytes(count: usize, names: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(slot_offset(count).saturating_add(names.len()));
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&u64_of(count).to_le_bytes());
    out.extend_from_slice(&u64_of(names.len()).to_le_bytes());
    out.extend_from_slice(&0_u64.to_le_bytes());
    for _ in 0..count {
        out.extend_from_slice(&UNSET.to_le_bytes());
    }
    out.extend_from_slice(names.as_bytes());
    out
}

/// A shared table of `names` at `path`, unwritten, and the writer for
/// it: the engine's side.
///
/// Each name is the engine's (`engine.`, it writes them) or the host's
/// (`host.`, the host writes them and the engine reads them). The file is
/// made whole under a temporary name and renamed into place.
///
/// # Errors
///
/// A name in neither space, more than [`MAX_SLOTS`], or the file could
/// not be made.
pub fn create(
    path: &Path,
    names: impl IntoIterator<Item = Name>,
) -> Result<(Arc<EngineTable>, EngineWriter), Error> {
    let mut map = BTreeMap::new();
    let mut order = Vec::new();
    for name in names {
        if !matches!(name.space(), Space::Engine | Space::Host) {
            return Err(Error::Denied("write", name.to_string()));
        }
        if map.contains_key(&name) {
            continue;
        }
        map.insert(name.clone(), order.len());
        order.push(name);
    }
    if order.len() > MAX_SLOTS {
        return Err(Error::TooManyValues(MAX_SLOTS));
    }
    let text = order
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let count = order.len();
    let io = |e: std::io::Error| denied(format!("{}: {e}", path.display()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let temporary = path.with_extension(format!("values.{}.new", std::process::id()));
    std::fs::write(&temporary, file_bytes(count, &text)).map_err(io)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temporary)
        .map_err(io)?;
    let raw = memmap2::MmapRaw::map_raw(&file).map_err(io)?;
    std::fs::rename(&temporary, path).map_err(io)?;
    let table = Arc::new(EngineTable {
        names: map,
        store: Store::Mapped(Mapping { map: raw, count }),
    });
    let writer = EngineWriter {
        table: Arc::clone(&table),
    };
    Ok((table, writer))
}

/// What a table's file says of itself, checked against its length.
struct Head<'a> {
    count: usize,
    names: &'a str,
}

fn head_of(bytes: &[u8]) -> Result<Head<'_>, Error> {
    let word = |at: usize| -> Result<u64, Error> {
        let w: [u8; 8] = bytes
            .get(at..at.saturating_add(8))
            .and_then(|b| b.try_into().ok())
            .ok_or(Error::Truncated)?;
        Ok(u64::from_le_bytes(w))
    };
    if bytes.get(..8) != Some(MAGIC.as_slice()) {
        return Err(Error::NotAFragment);
    }
    let count = usize::try_from(word(8)?).unwrap_or(usize::MAX);
    if count > MAX_SLOTS {
        return Err(Error::TooManyValues(MAX_SLOTS));
    }
    let names_len = usize::try_from(word(16)?).unwrap_or(usize::MAX);
    if names_len > MAX_NAME_BYTES {
        return Err(Error::TooLarge(names_len, MAX_NAME_BYTES));
    }
    let names_at = slot_offset(count);
    if names_at.saturating_add(names_len) != bytes.len() {
        return Err(Error::Truncated);
    }
    let names = bytes
        .get(names_at..)
        .and_then(|b| std::str::from_utf8(b).ok())
        .ok_or_else(|| Error::BadName("not UTF-8".to_owned()))?;
    Ok(Head { count, names })
}

/// The table at `path`, mapped: the host's side.
///
/// # Errors
///
/// No such file, or one that is not a whole table: a wrong magic, more
/// than [`MAX_SLOTS`], a length that is not what its header says, a name
/// that is not one or is in neither the engine's space nor the host's.
pub fn open(path: &Path) -> Result<Arc<EngineTable>, Error> {
    let io = |e: std::io::Error| denied(format!("{}: {e}", path.display()));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io)?;
    let len = usize::try_from(file.metadata().map_err(io)?.len()).map_err(|_| Error::Truncated)?;
    if len < HEADER {
        return Err(Error::Truncated);
    }
    let raw = memmap2::MmapRaw::map_raw(&file).map_err(io)?;
    if raw.len() != len {
        return Err(Error::Truncated);
    }
    // SAFETY: the mapping is `len` bytes, checked just above (an engine
    // never truncates a file it published). The header and the names are
    // written once, before the file got this name, and never again, so
    // reading them as plain bytes races nothing; the slots between them
    // are only ever read atomically, through `Mapping`.
    let bytes = unsafe { std::slice::from_raw_parts(raw.as_ptr(), len) };
    let head = head_of(bytes)?;
    let listed: Vec<&str> = if head.names.is_empty() {
        Vec::new()
    } else {
        head.names.split('\n').collect()
    };
    if listed.len() != head.count {
        return Err(Error::Truncated);
    }
    let mut map = BTreeMap::new();
    for (at, n) in listed.into_iter().enumerate() {
        let name = Name::new(n)?;
        if !matches!(name.space(), Space::Engine | Space::Host) {
            return Err(Error::Denied("read", name.to_string()));
        }
        map.entry(name).or_insert(at);
    }
    let count = head.count;
    Ok(Arc::new(EngineTable {
        names: map,
        store: Store::Mapped(Mapping { map: raw, count }),
    }))
}

/// The tables mapped so far, by engine, with the file each maps.
static OPENED: Mutex<BTreeMap<String, ((u64, u64), Arc<EngineTable>)>> =
    Mutex::new(BTreeMap::new());

/// Engine `id`'s shared table, if it is running: mapped once, and again
/// when the engine has published a new one (it restarted, or its tracks
/// changed). One `stat` a call.
#[must_use]
pub fn open_engine(id: &str) -> Option<Arc<EngineTable>> {
    let path = path_of(id).ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    let file = (meta.dev(), meta.ino());
    let held = OPENED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(id)
        .filter(|(was, _)| *was == file)
        .map(|(_, table)| Arc::clone(table));
    if held.is_some() {
        return held;
    }
    let table = open(&path).ok()?;
    OPENED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id.to_owned(), (file, Arc::clone(&table)));
    Some(table)
}
