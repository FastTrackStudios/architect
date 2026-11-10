//! A zip written as a stream (#510): "Download all" of a share.
//!
//! [`ZipWriter`] holds no file and does no I/O: the caller asks it for
//! each piece's bytes and sends them on as they come, so an archive of
//! any size is never in memory or on disk. Files are stored, not
//! compressed (charts, audio and images already are), with their size
//! and checksum after their bytes (a data descriptor), since neither is
//! known before the file has been read. Archives and files past 4 GiB
//! are written as Zip64.
//!
//! ```
//! use architect_sites::archive::{ZipWriter, read_stored};
//! let mut zip = ZipWriter::new();
//! let mut out = zip.start("01 Joy/Violin I.pdf", None);
//! zip.data(b"%PDF");
//! out.extend_from_slice(b"%PDF");
//! out.extend(zip.end());
//! out.extend(zip.finish());
//! assert_eq!(
//!     read_stored(&out),
//!     Some(vec![("01 Joy/Violin I.pdf".to_owned(), b"%PDF".to_vec())])
//! );
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

const LOCAL: u32 = 0x0403_4b50;
const DESCRIPTOR: u32 = 0x0807_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
const END64: u32 = 0x0606_4b50;
const LOCATOR64: u32 = 0x0706_4b50;
/// Sizes after the bytes (bit 3), names in UTF-8 (bit 11).
const FLAGS: u16 = 0x0808;
const MAX32: u64 = 0xFFFF_FFFF;

/// One file's record, for the directory at the end.
struct Record {
    name: Vec<u8>,
    at: u64,
    crc: u32,
    size: u64,
    time: u16,
    date: u16,
}

/// The file being written.
struct Open {
    name: Vec<u8>,
    at: u64,
    crc: crc32fast::Hasher,
    size: u64,
    time: u16,
    date: u16,
}

/// A zip, a piece at a time: for each file [`Self::start`], its bytes
/// through [`Self::data`] (sent on by the caller as they are),
/// [`Self::end`]; then [`Self::finish`].
#[derive(Default)]
pub struct ZipWriter {
    offset: u64,
    done: Vec<Record>,
    open: Option<Open>,
}

impl std::fmt::Debug for ZipWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZipWriter")
            .field("offset", &self.offset)
            .field("files", &self.done.len())
            .finish_non_exhaustive()
    }
}

fn u16le(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u64le(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// `v` in a 32-bit field: all ones when it does not fit (Zip64 holds it).
fn clamp32(v: u64) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

fn len64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

fn len16(bytes: &[u8]) -> u16 {
    u16::try_from(bytes.len()).unwrap_or(u16::MAX)
}

impl ZipWriter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many files are finished.
    #[must_use]
    pub const fn files(&self) -> usize {
        self.done.len()
    }

    /// Begin a file at `name` (folders with `/`): its header's bytes.
    /// A file still open is ended first.
    #[must_use]
    pub fn start(&mut self, name: &str, modified: Option<SystemTime>) -> Vec<u8> {
        let mut out = self.end();
        // A name is at most what its 16-bit length says.
        let mut name = name.trim_start_matches('/').as_bytes().to_vec();
        name.truncate(usize::from(u16::MAX));
        let (time, date) = dos_time(modified);
        let at = self.offset;
        let mut head = Vec::with_capacity(name.len().saturating_add(30));
        u32le(&mut head, LOCAL);
        u16le(&mut head, 20);
        u16le(&mut head, FLAGS);
        u16le(&mut head, 0);
        u16le(&mut head, time);
        u16le(&mut head, date);
        u32le(&mut head, 0);
        u32le(&mut head, 0);
        u32le(&mut head, 0);
        u16le(&mut head, len16(&name));
        u16le(&mut head, 0);
        head.extend_from_slice(&name);
        self.advance(head.len());
        self.open = Some(Open {
            name,
            at,
            crc: crc32fast::Hasher::new(),
            size: 0,
            time,
            date,
        });
        out.extend(head);
        out
    }

    /// Count `chunk` as the open file's next bytes. The caller sends the
    /// chunk itself: stored bytes are the file's own.
    pub fn data(&mut self, chunk: &[u8]) {
        if let Some(open) = &mut self.open {
            open.crc.update(chunk);
            open.size = open.size.saturating_add(len64(chunk.len()));
            self.advance(chunk.len());
        }
    }

    /// End the open file: its size and checksum, as bytes. Nothing when
    /// no file is open.
    #[must_use]
    pub fn end(&mut self) -> Vec<u8> {
        let Some(open) = self.open.take() else {
            return Vec::new();
        };
        let crc = open.crc.finalize();
        let mut out = Vec::with_capacity(24);
        u32le(&mut out, DESCRIPTOR);
        u32le(&mut out, crc);
        if open.size > MAX32 {
            u64le(&mut out, open.size);
            u64le(&mut out, open.size);
        } else {
            u32le(&mut out, clamp32(open.size));
            u32le(&mut out, clamp32(open.size));
        }
        self.advance(out.len());
        self.done.push(Record {
            name: open.name,
            at: open.at,
            crc,
            size: open.size,
            time: open.time,
            date: open.date,
        });
        out
    }

    /// The archive's last bytes: its directory. A file still open is
    /// ended first.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        let mut out = self.end();
        let directory_at = self.offset;
        let mut directory = Vec::new();
        for record in &self.done {
            let mut extra = Vec::new();
            if record.size > MAX32 {
                u64le(&mut extra, record.size);
                u64le(&mut extra, record.size);
            }
            if record.at > MAX32 {
                u64le(&mut extra, record.at);
            }
            let zip64 = !extra.is_empty();
            u32le(&mut directory, CENTRAL);
            // Made on Unix (3), by version 4.5.
            u16le(&mut directory, 0x032D);
            u16le(&mut directory, if zip64 { 45 } else { 20 });
            u16le(&mut directory, FLAGS);
            u16le(&mut directory, 0);
            u16le(&mut directory, record.time);
            u16le(&mut directory, record.date);
            u32le(&mut directory, record.crc);
            u32le(&mut directory, clamp32(record.size));
            u32le(&mut directory, clamp32(record.size));
            u16le(&mut directory, len16(&record.name));
            u16le(
                &mut directory,
                if zip64 {
                    len16(&extra).saturating_add(4)
                } else {
                    0
                },
            );
            u16le(&mut directory, 0);
            u16le(&mut directory, 0);
            u16le(&mut directory, 0);
            // A regular file, rw-r--r--.
            u32le(&mut directory, 0o100_644 << 16);
            u32le(&mut directory, clamp32(record.at));
            directory.extend_from_slice(&record.name);
            if zip64 {
                u16le(&mut directory, 1);
                u16le(&mut directory, len16(&extra));
                directory.extend_from_slice(&extra);
            }
        }
        let count = len64(self.done.len());
        let size = len64(directory.len());
        out.extend(directory);
        if count >= 0xFFFF || size >= MAX32 || directory_at >= MAX32 {
            let end64_at = directory_at.saturating_add(size);
            u32le(&mut out, END64);
            u64le(&mut out, 44);
            u16le(&mut out, 45);
            u16le(&mut out, 45);
            u32le(&mut out, 0);
            u32le(&mut out, 0);
            u64le(&mut out, count);
            u64le(&mut out, count);
            u64le(&mut out, size);
            u64le(&mut out, directory_at);
            u32le(&mut out, LOCATOR64);
            u32le(&mut out, 0);
            u64le(&mut out, end64_at);
            u32le(&mut out, 1);
        }
        let count16 = u16::try_from(count).unwrap_or(u16::MAX);
        u32le(&mut out, END);
        u16le(&mut out, 0);
        u16le(&mut out, 0);
        u16le(&mut out, count16);
        u16le(&mut out, count16);
        u32le(&mut out, clamp32(size));
        u32le(&mut out, clamp32(directory_at));
        u16le(&mut out, 0);
        out
    }

    fn advance(&mut self, by: usize) {
        self.offset = self.offset.saturating_add(len64(by));
    }
}

/// A moment as MS-DOS (time, date), UTC; 1980-01-01 when unknown or
/// before it.
#[allow(clippy::arithmetic_side_effects, clippy::integer_division)]
fn dos_time(at: Option<SystemTime>) -> (u16, u16) {
    const EPOCH: (u16, u16) = (0, (1 << 5) | 1);
    let Some(secs) = at
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
    else {
        return EPOCH;
    };
    let (days, of_day) = (secs / 86_400, secs % 86_400);
    // Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    if !(1980..=2107).contains(&year) {
        return EPOCH;
    }
    let time = (of_day / 3600) << 11 | (of_day % 3600 / 60) << 5 | (of_day % 60 / 2);
    let date = (year - 1980) << 9 | month << 5 | day;
    (
        u16::try_from(time).unwrap_or(0),
        u16::try_from(date).unwrap_or(EPOCH.1),
    )
}

fn u16_at(bytes: &[u8], at: usize) -> Option<usize> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(usize::from(u16::from_le_bytes(b.try_into().ok()?)))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(b.try_into().ok()?))
}

/// The files of a zip [`ZipWriter`] wrote, in the directory's order.
///
/// Each one's name and bytes, its checksum checked. `None` for anything
/// else (a compressed or Zip64 archive, a wrong checksum). For tests,
/// and for a caller that was handed an archive and counts what is in it.
#[must_use]
pub fn read_stored(zip: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let end = zip.len().checked_sub(22)?;
    if u32_at(zip, end)? != END {
        return None;
    }
    let count = u16_at(zip, end.checked_add(10)?)?;
    let mut at = usize::try_from(u32_at(zip, end.checked_add(16)?)?).ok()?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if u32_at(zip, at)? != CENTRAL || u16_at(zip, at.checked_add(10)?)? != 0 {
            return None;
        }
        let crc = u32_at(zip, at.checked_add(16)?)?;
        let size = usize::try_from(u32_at(zip, at.checked_add(24)?)?).ok()?;
        let name_len = u16_at(zip, at.checked_add(28)?)?;
        let extra_len = u16_at(zip, at.checked_add(30)?)?;
        let comment_len = u16_at(zip, at.checked_add(32)?)?;
        let local = usize::try_from(u32_at(zip, at.checked_add(42)?)?).ok()?;
        let name_at = at.checked_add(46)?;
        let name = zip.get(name_at..name_at.checked_add(name_len)?)?;
        if u32_at(zip, local)? != LOCAL {
            return None;
        }
        let data_at = local
            .checked_add(30)?
            .checked_add(u16_at(zip, local.checked_add(26)?)?)?
            .checked_add(u16_at(zip, local.checked_add(28)?)?)?;
        let data = zip.get(data_at..data_at.checked_add(size)?)?;
        if crc32fast::hash(data) != crc {
            return None;
        }
        out.push((String::from_utf8(name.to_vec()).ok()?, data.to_vec()));
        at = name_at
            .checked_add(name_len)?
            .checked_add(extra_len)?
            .checked_add(comment_len)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn files_come_back_in_order_with_their_bytes() {
        let files: [(&str, &[u8]); 3] = [
            ("01 Joy/Violin I.pdf", b"violin one"),
            ("01 Joy/é.txt", b""),
            ("02 Noel/Violin I.pdf", b"another"),
        ];
        let mut zip = ZipWriter::new();
        let mut out = Vec::new();
        for (name, bytes) in files {
            out.extend(zip.start(name, None));
            // In pieces, as a stream reads a file.
            for chunk in bytes.chunks(3) {
                zip.data(chunk);
                out.extend_from_slice(chunk);
            }
        }
        assert_eq!(zip.files(), 2, "the third is still open");
        out.extend(zip.finish());
        let read = read_stored(&out).expect("a zip");
        assert_eq!(read.len(), 3);
        for ((name, bytes), (got_name, got)) in files.iter().zip(&read) {
            assert_eq!(name, got_name);
            assert_eq!(bytes, &got.as_slice());
        }
        // A flipped byte of a file is a wrong checksum.
        let at = out.windows(6).position(|w| w == b"violin").unwrap();
        let mut bad = out.clone();
        bad[at] ^= 1;
        assert_eq!(read_stored(&bad), None);
    }

    #[test]
    fn an_empty_archive_is_a_zip() {
        let out = ZipWriter::new().finish();
        assert_eq!(out.len(), 22);
        assert_eq!(read_stored(&out), Some(Vec::new()));
    }

    #[test]
    fn dates_are_dos_dates() {
        // 2026-11-10 12:34:56 UTC.
        let at = UNIX_EPOCH + Duration::from_secs(1_794_314_096);
        let (time, date) = dos_time(Some(at));
        assert_eq!((date >> 9) + 1980, 2026);
        assert_eq!((date >> 5) & 15, 11);
        assert_eq!(date & 31, 10);
        assert_eq!(
            (time >> 11, (time >> 5) & 63, (time & 31) * 2),
            (12, 34, 56)
        );
        assert_eq!(dos_time(None), (0, 33));
        assert_eq!(dos_time(Some(UNIX_EPOCH)), (0, 33));
    }
}
