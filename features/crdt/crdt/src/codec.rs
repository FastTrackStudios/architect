//! Codec primitives — wire-field ↔ `LoroMap` value reads and writes.
//!
//! The shape every `EntityCrdt::encode_into` / `decode_from` /
//! `apply_update` body uses. Hand-written today; the architect derive
//! macro will emit these calls from field attributes in a future
//! revision, at which point this module becomes the runtime support
//! crate the emitted code links against.
//!
//! ## Storage choices
//!
//! - **Uuid** → RFC4122 string. Round-trips losslessly, sorts
//!   lexicographically (which is *not* timestamp order — use a
//!   dedicated `created_at` field for that).
//! - **`DateTime`\<Utc\>** → RFC3339 string. Sub-second precision is
//!   preserved at the nanosecond level chrono emits; round-tripping
//!   through parse can normalize nanos → micros on some platforms.
//! - **i32 / u32 / i64** → `LoroValue::I64`. `i32` and `u32` are
//!   range-checked on read.
//! - **bool** → `LoroValue::Bool`.
//! - **Option\<T\>** → omit the key when `None` (also accept
//!   `LoroValue::Null` on read for forward-compat).
//! - **Vec\<String\>** → tab-separated single string. Naive LWW on
//!   the whole vec; fine for low-conflict cases (tags). Promote to
//!   a `LoroList` sub-container when concurrent edits become a hot
//!   path — the codec call sites stay the same, the codec internals
//!   change.

use architect::RepoError;
use chrono::{DateTime, Utc};
use loro::{Container, LoroMap, LoroText, LoroValue};
use uuid::Uuid;

/// Wrap any Loro error into the architect `RepoError` shape so call
/// sites stay quiet.
///
/// Used by every write primitive below.
#[must_use]
pub fn loro_err<E: std::fmt::Display>(e: E) -> RepoError {
    RepoError::Internal(format!("loro: {e}"))
}

// ── Writes ────────────────────────────────────────────────────────────

/// Write a string at `k`.
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write — the document is
/// detached, or the key already holds an incompatible container.
pub fn write_str(m: &LoroMap, k: &str, v: &str) -> Result<(), RepoError> {
    m.insert(k, v).map_err(loro_err)
}

/// Writes `v`, or deletes the key when `v` is `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write. Deleting an
/// absent key is not an error.
pub fn write_opt_str(m: &LoroMap, k: &str, v: Option<&str>) -> Result<(), RepoError> {
    v.map_or_else(
        || {
            let _ = m.delete(k);
            Ok(())
        },
        |s| write_str(m, k, s),
    )
}

/// Write a `Uuid` as its RFC4122 string form.
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write — the document is
/// detached, or the key already holds an incompatible container.
pub fn write_uuid(m: &LoroMap, k: &str, v: Uuid) -> Result<(), RepoError> {
    write_str(m, k, &v.to_string())
}

/// Writes `v`, or deletes the key when `v` is `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write. Deleting an
/// absent key is not an error.
pub fn write_opt_uuid(m: &LoroMap, k: &str, v: Option<Uuid>) -> Result<(), RepoError> {
    v.map_or_else(
        || {
            let _ = m.delete(k);
            Ok(())
        },
        |u| write_uuid(m, k, u),
    )
}

/// Write a timestamp as its RFC3339 string form.
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write — the document is
/// detached, or the key already holds an incompatible container.
pub fn write_dt(m: &LoroMap, k: &str, v: DateTime<Utc>) -> Result<(), RepoError> {
    write_str(m, k, &v.to_rfc3339())
}

/// Writes `v`, or deletes the key when `v` is `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write. Deleting an
/// absent key is not an error.
pub fn write_opt_dt(m: &LoroMap, k: &str, v: Option<DateTime<Utc>>) -> Result<(), RepoError> {
    v.map_or_else(
        || {
            let _ = m.delete(k);
            Ok(())
        },
        |dt| write_dt(m, k, dt),
    )
}

/// Write a bool at `k`.
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write — the document is
/// detached, or the key already holds an incompatible container.
pub fn write_bool(m: &LoroMap, k: &str, v: bool) -> Result<(), RepoError> {
    m.insert(k, v).map_err(loro_err)
}

/// Writes `v`, or deletes the key when `v` is `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write. Deleting an
/// absent key is not an error.
pub fn write_opt_bool(m: &LoroMap, k: &str, v: Option<bool>) -> Result<(), RepoError> {
    v.map_or_else(
        || {
            let _ = m.delete(k);
            Ok(())
        },
        |b| write_bool(m, k, b),
    )
}

/// Write an `i64` at `k`.
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write — the document is
/// detached, or the key already holds an incompatible container.
pub fn write_i64(m: &LoroMap, k: &str, v: i64) -> Result<(), RepoError> {
    m.insert(k, v).map_err(loro_err)
}

/// Writes `v`, or deletes the key when `v` is `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write. Deleting an
/// absent key is not an error.
pub fn write_opt_i64(m: &LoroMap, k: &str, v: Option<i64>) -> Result<(), RepoError> {
    v.map_or_else(
        || {
            let _ = m.delete(k);
            Ok(())
        },
        |n| write_i64(m, k, n),
    )
}

/// Write an `i32`, widened to the stored `i64`.
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write — the document is
/// detached, or the key already holds an incompatible container.
pub fn write_i32(m: &LoroMap, k: &str, v: i32) -> Result<(), RepoError> {
    write_i64(m, k, i64::from(v))
}

/// Writes `v`, or deletes the key when `v` is `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write. Deleting an
/// absent key is not an error.
pub fn write_opt_i32(m: &LoroMap, k: &str, v: Option<i32>) -> Result<(), RepoError> {
    v.map_or_else(
        || {
            let _ = m.delete(k);
            Ok(())
        },
        |n| write_i32(m, k, n),
    )
}

/// Write a `u32`, widened to the stored `i64`.
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write — the document is
/// detached, or the key already holds an incompatible container.
pub fn write_u32(m: &LoroMap, k: &str, v: u32) -> Result<(), RepoError> {
    write_i64(m, k, i64::from(v))
}

/// Writes `v`, or deletes the key when `v` is `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write. Deleting an
/// absent key is not an error.
pub fn write_opt_u32(m: &LoroMap, k: &str, v: Option<u32>) -> Result<(), RepoError> {
    v.map_or_else(
        || {
            let _ = m.delete(k);
            Ok(())
        },
        |n| write_u32(m, k, n),
    )
}

/// Tab-separated encoding. Naive LWW on the whole vec — fine for
/// tags and other rarely-conflicting lists. Upgrade individual call
/// sites to `LoroList` sub-containers when concurrent edits matter.
///
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write.
pub fn write_string_list(m: &LoroMap, k: &str, v: &[String]) -> Result<(), RepoError> {
    let joined = v.join("\t");
    write_str(m, k, &joined)
}

/// Writes `v`, or deletes the key when `v` is `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if Loro rejects the write. Deleting an
/// absent key is not an error.
pub fn write_opt_string_list(m: &LoroMap, k: &str, v: Option<&[String]>) -> Result<(), RepoError> {
    v.map_or_else(
        || {
            let _ = m.delete(k);
            Ok(())
        },
        |slice| write_string_list(m, k, slice),
    )
}

// ── Reads ─────────────────────────────────────────────────────────────

/// Read a string at `k`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key is missing, or holds a
/// value of the wrong type.
pub fn read_str(m: &LoroMap, k: &str) -> Result<String, RepoError> {
    match m.get(k) {
        Some(loro::ValueOrContainer::Value(LoroValue::String(s))) => Ok((*s).clone()),
        Some(other) => Err(RepoError::Internal(format!(
            "expected string at `{k}`, got {other:?}"
        ))),
        None => Err(RepoError::Internal(format!("missing key `{k}`"))),
    }
}

/// Read an optional string at `k`.
///
/// An absent key and an explicit `Null` both read as `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key holds a value of the wrong
/// type.
pub fn read_opt_str(m: &LoroMap, k: &str) -> Result<Option<String>, RepoError> {
    match m.get(k) {
        None | Some(loro::ValueOrContainer::Value(LoroValue::Null)) => Ok(None),
        Some(loro::ValueOrContainer::Value(LoroValue::String(s))) => Ok(Some((*s).clone())),
        Some(other) => Err(RepoError::Internal(format!(
            "expected string at `{k}`, got {other:?}"
        ))),
    }
}

/// Read a `Uuid` from its RFC4122 string form.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key is missing or is not a valid UUID, or holds a
/// value of the wrong type.
pub fn read_uuid(m: &LoroMap, k: &str) -> Result<Uuid, RepoError> {
    Uuid::parse_str(&read_str(m, k)?)
        .map_err(|e| RepoError::Internal(format!("bad uuid at `{k}`: {e}")))
}

/// Read an optional `Uuid`.
///
/// An absent key and an explicit `Null` both read as `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key holds a value of the wrong
/// type or is not a valid UUID.
pub fn read_opt_uuid(m: &LoroMap, k: &str) -> Result<Option<Uuid>, RepoError> {
    read_opt_str(m, k)?.map_or(Ok(None), |s| {
        Uuid::parse_str(&s)
            .map(Some)
            .map_err(|e| RepoError::Internal(format!("bad uuid at `{k}`: {e}")))
    })
}

/// Read a timestamp from its RFC3339 string form.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key is missing or is not valid RFC3339, or holds a
/// value of the wrong type.
pub fn read_dt(m: &LoroMap, k: &str) -> Result<DateTime<Utc>, RepoError> {
    parse_dt(&read_str(m, k)?, k)
}

/// Read an optional timestamp.
///
/// An absent key and an explicit `Null` both read as `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key holds a value of the wrong
/// type or is not valid RFC3339.
pub fn read_opt_dt(m: &LoroMap, k: &str) -> Result<Option<DateTime<Utc>>, RepoError> {
    read_opt_str(m, k)?.map_or(Ok(None), |s| parse_dt(&s, k).map(Some))
}

fn parse_dt(s: &str, k: &str) -> Result<DateTime<Utc>, RepoError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| RepoError::Internal(format!("bad timestamp at `{k}`: {e}")))
}

/// Read a bool at `k`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key is missing, or holds a
/// value of the wrong type.
pub fn read_bool(m: &LoroMap, k: &str) -> Result<bool, RepoError> {
    match m.get(k) {
        Some(loro::ValueOrContainer::Value(LoroValue::Bool(b))) => Ok(b),
        Some(other) => Err(RepoError::Internal(format!(
            "expected bool at `{k}`, got {other:?}"
        ))),
        None => Err(RepoError::Internal(format!("missing key `{k}`"))),
    }
}

/// Read an optional bool at `k`.
///
/// An absent key and an explicit `Null` both read as `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key holds a value of the wrong
/// type.
pub fn read_opt_bool(m: &LoroMap, k: &str) -> Result<Option<bool>, RepoError> {
    match m.get(k) {
        None | Some(loro::ValueOrContainer::Value(LoroValue::Null)) => Ok(None),
        Some(loro::ValueOrContainer::Value(LoroValue::Bool(b))) => Ok(Some(b)),
        Some(other) => Err(RepoError::Internal(format!(
            "expected bool at `{k}`, got {other:?}"
        ))),
    }
}

/// Read an `i64` at `k`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key is missing, or holds a
/// value of the wrong type.
pub fn read_i64(m: &LoroMap, k: &str) -> Result<i64, RepoError> {
    match m.get(k) {
        Some(loro::ValueOrContainer::Value(LoroValue::I64(n))) => Ok(n),
        Some(other) => Err(RepoError::Internal(format!(
            "expected i64 at `{k}`, got {other:?}"
        ))),
        None => Err(RepoError::Internal(format!("missing key `{k}`"))),
    }
}

/// Read an optional `i64` at `k`.
///
/// An absent key and an explicit `Null` both read as `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key holds a value of the wrong
/// type.
pub fn read_opt_i64(m: &LoroMap, k: &str) -> Result<Option<i64>, RepoError> {
    match m.get(k) {
        None | Some(loro::ValueOrContainer::Value(LoroValue::Null)) => Ok(None),
        Some(loro::ValueOrContainer::Value(LoroValue::I64(n))) => Ok(Some(n)),
        Some(other) => Err(RepoError::Internal(format!(
            "expected i64 at `{k}`, got {other:?}"
        ))),
    }
}

/// Read an `i32` from the stored `i64`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key is missing or is outside `i32` range, or holds a
/// value of the wrong type.
pub fn read_i32(m: &LoroMap, k: &str) -> Result<i32, RepoError> {
    let n = read_i64(m, k)?;
    narrow_i32(n, k)
}

/// Read an optional `i32` from the stored `i64`.
///
/// An absent key and an explicit `Null` both read as `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key holds a value of the wrong
/// type or is outside `i32` range.
pub fn read_opt_i32(m: &LoroMap, k: &str) -> Result<Option<i32>, RepoError> {
    read_opt_i64(m, k)?.map_or(Ok(None), |n| narrow_i32(n, k).map(Some))
}

// `try_from` IS the range check — the hand-written comparison plus an
// `as` cast said the same thing twice, and only the comparison was
// checked by the compiler.
fn narrow_i32(n: i64, k: &str) -> Result<i32, RepoError> {
    i32::try_from(n).map_err(|_| RepoError::Internal(format!("out of range i32 at `{k}`: {n}")))
}

fn narrow_u32(n: i64, k: &str) -> Result<u32, RepoError> {
    u32::try_from(n).map_err(|_| RepoError::Internal(format!("out of range u32 at `{k}`: {n}")))
}

/// Read a `u32` from the stored `i64`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key is missing or is outside `u32` range, or holds a
/// value of the wrong type.
pub fn read_u32(m: &LoroMap, k: &str) -> Result<u32, RepoError> {
    let n = read_i64(m, k)?;
    narrow_u32(n, k)
}

/// Read an optional `u32` from the stored `i64`.
///
/// An absent key and an explicit `Null` both read as `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key holds a value of the wrong
/// type or is outside `u32` range.
pub fn read_opt_u32(m: &LoroMap, k: &str) -> Result<Option<u32>, RepoError> {
    read_opt_i64(m, k)?.map_or(Ok(None), |n| narrow_u32(n, k).map(Some))
}

/// Read a tab-separated string list. An empty string reads as an
/// empty `Vec`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key is missing, or holds a
/// value of the wrong type.
pub fn read_string_list(m: &LoroMap, k: &str) -> Result<Vec<String>, RepoError> {
    let raw = read_str(m, k)?;
    if raw.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(raw.split('\t').map(str::to_string).collect())
    }
}

/// Read an optional tab-separated string list.
///
/// An absent key and an explicit `Null` both read as `None`.
///
/// # Errors
///
/// [`RepoError::Internal`] if the key holds a value of the wrong
/// type.
pub fn read_opt_string_list(m: &LoroMap, k: &str) -> Result<Option<Vec<String>>, RepoError> {
    match read_opt_str(m, k)? {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(Some(Vec::new())),
        Some(s) => Ok(Some(s.split('\t').map(str::to_string).collect())),
    }
}

// ── LoroText ──────────────────────────────────────────────────────────
//
// Helpers for character-level text fields. Wire as a child `LoroText`
// container under a map key. Positions are unicode scalar units —
// matches Loro's default `insert`/`delete` API. Callers working in
// UTF-16 (DOM) must translate at the boundary.

/// Edit op against a `LoroText` child container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextOp {
    Insert { pos: u32, text: String },
    Delete { pos: u32, len: u32 },
}

/// Get-or-create a `LoroText` child container under `map[key]`.
///
/// Idempotent: a second call returns the same attached container.
/// Returns `Err` if the key already holds a non-text value (legacy
/// snapshot string) — callers wanting migration use
/// [`read_text_with_migration`].
///
/// # Errors
///
/// [`RepoError::Internal`] if `key` holds a non-text value, or if Loro
/// rejects creating the container.
pub fn text_child(m: &LoroMap, key: &str) -> Result<LoroText, RepoError> {
    if let Some(loro::ValueOrContainer::Container(Container::Text(t))) = m.get(key) {
        return Ok(t);
    }
    if let Some(other) = m.get(key) {
        return Err(RepoError::Internal(format!(
            "expected text container at `{key}`, got {other:?}"
        )));
    }
    m.ensure_mergeable_text(key).map_err(loro_err)
}

/// Read the current text. Returns "" if the key is absent.
///
/// # Errors
///
/// [`RepoError::Internal`] if `key` holds something other than a text
/// container.
pub fn read_text(m: &LoroMap, key: &str) -> Result<String, RepoError> {
    match m.get(key) {
        None => Ok(String::new()),
        Some(loro::ValueOrContainer::Container(Container::Text(t))) => Ok(t.to_string()),
        Some(other) => Err(RepoError::Internal(format!(
            "expected text container at `{key}`, got {other:?}"
        ))),
    }
}

/// Read text with migration from a legacy string `LoroValue`.
///
/// If `key` holds a plain string (from older snapshots), seeds a
/// `LoroText` container with that content and returns the value.
/// Idempotent after the first call: subsequent reads hit the
/// text-container path. # Errors
///
/// [`RepoError::Internal`] if `key` holds neither a text container nor
/// a legacy string, or if Loro rejects the in-place migration.
pub fn read_text_with_migration(m: &LoroMap, key: &str) -> Result<String, RepoError> {
    match m.get(key) {
        Some(loro::ValueOrContainer::Container(Container::Text(t))) => Ok(t.to_string()),
        Some(loro::ValueOrContainer::Value(LoroValue::String(s))) => {
            let legacy = (*s).clone();
            m.delete(key).map_err(loro_err)?;
            let t = m.ensure_mergeable_text(key).map_err(loro_err)?;
            if !legacy.is_empty() {
                t.insert(0, &legacy).map_err(loro_err)?;
            }
            Ok(t.to_string())
        }
        Some(loro::ValueOrContainer::Value(LoroValue::Null)) | None => Ok(String::new()),
        Some(other) => Err(RepoError::Internal(format!(
            "expected text container or legacy string at `{key}`, got {other:?}"
        ))),
    }
}

/// Apply a sequence of `TextOp` against the text child at `map[key]`.
/// Each op runs against the post-state of the previous ops, matching
/// the editor's stream-of-keystrokes semantics.
///
/// # Errors
///
/// [`RepoError::Internal`] if the container can't be reached (see
/// [`text_child`]), if a position doesn't fit `usize` on this target, or
/// if Loro rejects an op (a position past the end of the text).
pub fn apply_text_ops(m: &LoroMap, key: &str, ops: &[TextOp]) -> Result<(), RepoError> {
    let t = text_child(m, key)?;
    // `u32 as usize` is lossless on 64- and 32-bit targets but not on
    // 16-bit ones, and `as` would silently wrap there. `try_from` turns
    // that into an ordinary decode error.
    let fit = |n: u32| -> Result<usize, RepoError> {
        usize::try_from(n)
            .map_err(|_| RepoError::Internal(format!("text position {n} out of range")))
    };
    for op in ops {
        match op {
            TextOp::Insert { pos, text } => {
                t.insert(fit(*pos)?, text).map_err(loro_err)?;
            }
            TextOp::Delete { pos, len } => {
                t.delete(fit(*pos)?, fit(*len)?).map_err(loro_err)?;
            }
        }
    }
    Ok(())
}

/// Fallback diff: compute a minimal insert/delete pair from old → new
/// using common-prefix + common-suffix stripping.
///
/// O(n). Correct for typing flows and most paste/programmatic-overwrite
/// cases. For non-prefix/suffix edits, the change collapses to a single
/// delete-middle + insert-middle pair — not optimal but correct. #
/// Errors
///
/// [`RepoError::Internal`] if the container can't be reached (see
/// [`text_child`]), if either side is longer than `u32::MAX` characters,
/// or if Loro rejects the resulting ops.
pub fn apply_text_diff(m: &LoroMap, key: &str, old: &str, new: &str) -> Result<(), RepoError> {
    if old == new {
        // Touch the container so callers can rely on its existence.
        let _ = text_child(m, key)?;
        return Ok(());
    }
    let old_chars: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();

    // Common prefix. Zipping the two sequences makes the bound implicit
    // instead of asserting it with two `<` checks and an index.
    let prefix = old_chars
        .iter()
        .zip(new_chars.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Common suffix, over what the prefix left behind on each side.
    let old_rest = old_chars.len().saturating_sub(prefix);
    let new_rest = new_chars.len().saturating_sub(prefix);
    let suffix = old_chars
        .iter()
        .rev()
        .take(old_rest)
        .zip(new_chars.iter().rev().take(new_rest))
        .take_while(|(a, b)| a == b)
        .count();

    let del_len = old_rest.saturating_sub(suffix);
    let ins: String = new_chars
        .get(prefix..new_chars.len().saturating_sub(suffix))
        .unwrap_or_default()
        .iter()
        .collect();

    // Loro's positions are u32. A document longer than 4G characters is
    // a decode error, not a silently wrapped position.
    let fit = |n: usize| -> Result<u32, RepoError> {
        u32::try_from(n).map_err(|_| RepoError::Internal(format!("text position {n} out of range")))
    };
    let pos = fit(prefix)?;

    let mut ops: Vec<TextOp> = Vec::new();
    if del_len > 0 {
        ops.push(TextOp::Delete {
            pos,
            len: fit(del_len)?,
        });
    }
    if !ins.is_empty() {
        ops.push(TextOp::Insert { pos, text: ins });
    }
    apply_text_ops(m, key, &ops)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::panic,
    clippy::float_cmp,
    clippy::string_slice,
    clippy::significant_drop_tightening,
    clippy::too_many_lines
)]
mod scalar_tests {
    use super::*;
    use loro::LoroDoc;

    fn root(doc: &LoroDoc) -> LoroMap {
        doc.get_map("root")
    }

    #[test]
    fn i32_round_trip() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        write_i32(&m, "n", -42).unwrap();
        assert_eq!(read_i32(&m, "n").unwrap(), -42);
    }

    #[test]
    fn i32_extremes_round_trip() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        write_i32(&m, "min", i32::MIN).unwrap();
        write_i32(&m, "max", i32::MAX).unwrap();
        assert_eq!(read_i32(&m, "min").unwrap(), i32::MIN);
        assert_eq!(read_i32(&m, "max").unwrap(), i32::MAX);
    }

    #[test]
    fn read_i32_missing_key_errors() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        assert!(read_i32(&m, "absent").is_err());
    }

    #[test]
    fn read_i32_out_of_range_errors() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        write_i64(&m, "big", i64::from(i32::MAX) + 1).unwrap();
        write_i64(&m, "small", i64::from(i32::MIN) - 1).unwrap();
        assert!(matches!(
            read_i32(&m, "big"),
            Err(RepoError::Internal(msg)) if msg.contains("out of range i32")
        ));
        assert!(read_i32(&m, "small").is_err());
    }

    #[test]
    fn opt_i32_some_round_trips() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        write_opt_i32(&m, "n", Some(7)).unwrap();
        assert_eq!(read_opt_i32(&m, "n").unwrap(), Some(7));
    }

    #[test]
    fn opt_i32_none_omits_the_key() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        // None on a fresh map: key stays absent → reads back as None.
        write_opt_i32(&m, "n", None).unwrap();
        assert_eq!(read_opt_i32(&m, "n").unwrap(), None);
        // Some then None: the key is deleted again.
        write_opt_i32(&m, "n", Some(3)).unwrap();
        write_opt_i32(&m, "n", None).unwrap();
        assert_eq!(read_opt_i32(&m, "n").unwrap(), None);
    }

    #[test]
    fn read_opt_i32_out_of_range_errors() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        write_i64(&m, "big", i64::from(u32::MAX)).unwrap();
        assert!(read_opt_i32(&m, "big").is_err());
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::panic,
    clippy::float_cmp,
    clippy::string_slice,
    clippy::significant_drop_tightening,
    clippy::too_many_lines
)]
mod text_tests {
    use super::*;
    use loro::{ContainerTrait, ExportMode, LoroDoc};

    fn root(doc: &LoroDoc) -> LoroMap {
        doc.get_map("root")
    }

    #[test]
    fn text_child_is_idempotent() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        let a = text_child(&m, "content").unwrap();
        a.insert(0, "abc").unwrap();
        let b = text_child(&m, "content").unwrap();
        assert_eq!(b.to_string(), "abc");
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn read_text_empty_when_missing() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        assert_eq!(read_text(&m, "missing").unwrap(), "");
    }

    #[test]
    fn apply_text_ops_insert_then_delete() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        apply_text_ops(
            &m,
            "c",
            &[
                TextOp::Insert {
                    pos: 0,
                    text: "hello world".into(),
                },
                TextOp::Delete { pos: 5, len: 1 },
                TextOp::Insert {
                    pos: 5,
                    text: "—".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(read_text(&m, "c").unwrap(), "hello—world");
    }

    #[test]
    fn apply_text_diff_seeds_empty() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        apply_text_diff(&m, "c", "", "fresh").unwrap();
        assert_eq!(read_text(&m, "c").unwrap(), "fresh");
    }

    #[test]
    fn apply_text_diff_is_minimal_for_suffix_change() {
        // Common-prefix typing flow: "hello" → "hello!" should produce
        // exactly one Insert at pos=5 of "!", no deletes, no full
        // replacement. We can't observe the ops directly via the
        // public surface, so we assert via remote merge: a concurrent
        // edit at pos 0 must survive.
        let a = LoroDoc::new();
        a.set_peer_id(1).unwrap();
        let b = LoroDoc::new();
        b.set_peer_id(2).unwrap();
        // Seed the container on A and replicate to B before forking.
        apply_text_diff(&root(&a), "c", "", "hello").unwrap();
        a.commit();
        b.import(&a.export(ExportMode::all_updates()).unwrap())
            .unwrap();

        // Peer A appends "!" via diff.
        apply_text_diff(&root(&a), "c", "hello", "hello!").unwrap();
        // Peer B prepends ">" via diff at the same time.
        apply_text_diff(&root(&b), "c", "hello", ">hello").unwrap();
        a.commit();
        b.commit();

        // Cross-import.
        let a_to_b = a.export(ExportMode::updates(&b.oplog_vv())).unwrap();
        let b_to_a = b.export(ExportMode::updates(&a.oplog_vv())).unwrap();
        b.import(&a_to_b).unwrap();
        a.import(&b_to_a).unwrap();

        let final_a = read_text(&root(&a), "c").unwrap();
        let final_b = read_text(&root(&b), "c").unwrap();
        assert_eq!(final_a, final_b, "peers diverged");
        // Both inserts must be present — minimal-diff property.
        assert!(final_a.contains('>'), "lost peer B prepend: {final_a}");
        assert!(final_a.contains('!'), "lost peer A append: {final_a}");
        assert!(final_a.contains("hello"));
    }

    #[test]
    fn two_doc_concurrent_insert_at_same_pos() {
        // Container creation must happen once on one peer and sync to
        // the other before forked edits, otherwise each peer creates
        // its own LoroText under the same key and one wins.
        let a = LoroDoc::new();
        a.set_peer_id(1).unwrap();
        let b = LoroDoc::new();
        b.set_peer_id(2).unwrap();
        let _ = text_child(&root(&a), "c").unwrap();
        a.commit();
        b.import(&a.export(ExportMode::all_updates()).unwrap())
            .unwrap();

        apply_text_ops(
            &root(&a),
            "c",
            &[TextOp::Insert {
                pos: 0,
                text: "abc".into(),
            }],
        )
        .unwrap();
        apply_text_ops(
            &root(&b),
            "c",
            &[TextOp::Insert {
                pos: 0,
                text: "xyz".into(),
            }],
        )
        .unwrap();
        a.commit();
        b.commit();

        let a_to_b = a.export(ExportMode::updates(&b.oplog_vv())).unwrap();
        let b_to_a = b.export(ExportMode::updates(&a.oplog_vv())).unwrap();
        b.import(&a_to_b).unwrap();
        a.import(&b_to_a).unwrap();

        let final_a = read_text(&root(&a), "c").unwrap();
        let final_b = read_text(&root(&b), "c").unwrap();
        assert_eq!(final_a, final_b);
        assert_eq!(final_a.len(), 6, "both inserts must survive: {final_a}");
        for ch in "abcxyz".chars() {
            assert!(final_a.contains(ch));
        }
    }

    #[test]
    fn read_text_with_migration_seeds_from_legacy_string() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        write_str(&m, "c", "legacy body").unwrap();
        let seen = read_text_with_migration(&m, "c").unwrap();
        assert_eq!(seen, "legacy body");
        // Second call now hits the text-container path and matches.
        assert_eq!(read_text_with_migration(&m, "c").unwrap(), "legacy body");
        // And further ops apply against the seeded container.
        apply_text_ops(
            &m,
            "c",
            &[TextOp::Insert {
                pos: 11,
                text: "!".into(),
            }],
        )
        .unwrap();
        assert_eq!(read_text(&m, "c").unwrap(), "legacy body!");
    }

    #[test]
    fn apply_text_diff_noop_when_unchanged() {
        let doc = LoroDoc::new();
        let m = root(&doc);
        apply_text_diff(&m, "c", "", "same").unwrap();
        apply_text_diff(&m, "c", "same", "same").unwrap();
        assert_eq!(read_text(&m, "c").unwrap(), "same");
    }
}
