//! A document's attachments, in the document: one Loro map,
//! `attachments`, from each attachment's hash to its details.
//!
//! Attaching edits the document like any other change, so it works
//! offline and reaches every replica with the document; two devices that
//! attach the same bytes meet in one entry. The bytes travel apart, by
//! hash ([`architect::blob`]). [`refs`] is a `crdt::BlobRefs`: a kind
//! that holds attachments names it in `DocKind::blobs`, so what a
//! document points at is kept and fetched, and what it no longer points
//! at is collected.

use std::collections::HashMap;

use loro::{LoroDoc, LoroValue};

use crate::model::{AttachFault, Attachment};

/// The map's name in the document.
pub const MAP: &str = "attachments";

/// Attach `attachment` to `doc` (again: its details replace the old).
///
/// # Errors
///
/// [`AttachFault::Invalid`] for an attachment that does not validate, or
/// one Loro would not take.
pub fn attach(doc: &LoroDoc, attachment: &Attachment) -> Result<(), AttachFault> {
    attachment.validate()?;
    let size = i64::try_from(attachment.size)
        .map_err(|_| AttachFault::invalid("an attachment is smaller than 8 EiB"))?;
    let mut fields: HashMap<&str, LoroValue> = HashMap::from([
        ("name", attachment.name.as_str().into()),
        ("mime", attachment.mime.as_str().into()),
        ("size", size.into()),
        ("added_us", attachment.added_us.into()),
    ]);
    if let Some(by) = &attachment.added_by {
        fields.insert("added_by", by.as_str().into());
    }
    doc.get_map(MAP)
        .insert(&attachment.hash, LoroValue::from(fields))
        .map_err(|e| AttachFault::invalid(e.to_string()))?;
    doc.commit();
    Ok(())
}

/// Detach the attachment `hash` names; whether there was one.
///
/// # Errors
///
/// Loro would not take the change.
pub fn detach(doc: &LoroDoc, hash: &str) -> Result<bool, AttachFault> {
    let map = doc.get_map(MAP);
    if map.get(hash).is_none() {
        return Ok(false);
    }
    map.delete(hash)
        .map_err(|e| AttachFault::invalid(e.to_string()))?;
    doc.commit();
    Ok(true)
}

/// `doc`'s attachments, oldest first. Entries this version cannot read
/// (a newer writer's, a damaged one) are skipped.
#[must_use]
pub fn list(doc: &LoroDoc) -> Vec<Attachment> {
    let LoroValue::Map(entries) = doc.get_map(MAP).get_value() else {
        return Vec::new();
    };
    let mut out: Vec<Attachment> = entries
        .iter()
        .filter_map(|(hash, value)| read(hash, value))
        .collect();
    out.sort_by(|a, b| (a.added_us, &a.name).cmp(&(b.added_us, &b.name)));
    out
}

/// The attachment `hash` names on `doc`.
#[must_use]
pub fn get(doc: &LoroDoc, hash: &str) -> Option<Attachment> {
    let LoroValue::Map(entries) = doc.get_map(MAP).get_value() else {
        return None;
    };
    entries.get(hash).and_then(|value| read(hash, value))
}

/// The hashes `doc`'s attachments point at: a `crdt::BlobRefs`, for
/// `DocKind::blobs` (or a kind's own reader, joined with its other
/// blobs).
#[must_use]
pub fn refs(doc: &LoroDoc) -> Vec<String> {
    list(doc).into_iter().map(|a| a.hash).collect()
}

fn read(hash: &str, value: &LoroValue) -> Option<Attachment> {
    let LoroValue::Map(fields) = value else {
        return None;
    };
    let text = |key: &str| match fields.get(key) {
        Some(LoroValue::String(s)) => Some(s.to_string()),
        _ => None,
    };
    let number = |key: &str| match fields.get(key) {
        Some(LoroValue::I64(n)) => Some(*n),
        _ => None,
    };
    let attachment = Attachment {
        hash: hash.to_owned(),
        name: text("name")?,
        mime: text("mime").unwrap_or_else(|| "application/octet-stream".to_owned()),
        size: u64::try_from(number("size")?).ok()?,
        added_us: number("added_us").unwrap_or_default(),
        added_by: text("added_by"),
    };
    attachment.validate().ok().map(|()| attachment)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf(hash: char, at: i64) -> Attachment {
        Attachment {
            hash: hash.to_string().repeat(64),
            name: format!("{hash}.pdf"),
            mime: "application/pdf".into(),
            size: 10,
            added_us: at,
            added_by: Some("user:alice".into()),
        }
    }

    #[test]
    fn a_document_lists_what_is_attached_and_forgets_what_is_detached() {
        let doc = LoroDoc::new();
        attach(&doc, &pdf('b', 2)).unwrap();
        attach(&doc, &pdf('a', 1)).unwrap();
        assert_eq!(list(&doc), vec![pdf('a', 1), pdf('b', 2)]);
        assert_eq!(refs(&doc), vec!["a".repeat(64), "b".repeat(64)]);
        assert_eq!(get(&doc, &"b".repeat(64)), Some(pdf('b', 2)));
        assert!(detach(&doc, &"a".repeat(64)).unwrap());
        assert!(!detach(&doc, &"a".repeat(64)).unwrap());
        assert_eq!(list(&doc), vec![pdf('b', 2)]);
    }

    #[test]
    fn two_replicas_that_attach_offline_meet() {
        let (one, two) = (LoroDoc::new(), LoroDoc::new());
        attach(&one, &pdf('a', 1)).unwrap();
        attach(&two, &pdf('b', 2)).unwrap();
        attach(&two, &pdf('a', 1)).unwrap();
        two.import(&one.export(loro::ExportMode::all_updates()).unwrap())
            .unwrap();
        one.import(&two.export(loro::ExportMode::all_updates()).unwrap())
            .unwrap();
        assert_eq!(list(&one), list(&two));
        assert_eq!(list(&one).len(), 2);
    }

    #[test]
    fn a_bad_attachment_is_refused_and_a_bad_entry_skipped() {
        let doc = LoroDoc::new();
        let mut bad = pdf('a', 1);
        bad.hash = "nope".into();
        assert!(attach(&doc, &bad).is_err());
        doc.get_map(MAP).insert("nope", 1).unwrap();
        assert!(list(&doc).is_empty());
    }
}
