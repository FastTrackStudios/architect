//! A project's extensions in its document (feature `crdt`).
//!
//! The [`Requirements`] in its `meta` map, and each extension's own data in a
//! container namespaced by its id — saved with the project, part of its
//! undo, synced with it (REAPER's `ProjExtState` and `P_EXT`).

use crdt::loro::{LoroDoc, LoroError, LoroMap, LoroValue, ValueOrContainer};

use crate::project::Requirements;

/// The `meta` key the requirements are kept under.
pub const KEY: &str = "extensions";

/// What `doc`'s `meta` says the project needs (none when it says
/// nothing).
///
/// # Errors
///
/// What is there is not requirements.
pub fn requirements(doc: &LoroDoc) -> Result<Requirements, String> {
    match doc.get_map("meta").get(KEY) {
        None | Some(ValueOrContainer::Value(LoroValue::Null)) => Ok(Requirements::default()),
        Some(ValueOrContainer::Value(LoroValue::String(text))) => Requirements::from_json(&text),
        Some(other) => Err(format!("meta.{KEY} is not text: {other:?}")),
    }
}

/// Keeps `requirements` in `doc`'s `meta` (commit it with the change
/// that needed it).
///
/// # Errors
///
/// Loro refused the write.
pub fn set_requirements(doc: &LoroDoc, requirements: &Requirements) -> Result<(), String> {
    let text = requirements.to_json()?;
    doc.get_map("meta")
        .insert(KEY, text)
        .map_err(|e: LoroError| e.to_string())
}

/// Extension `id`'s container for the project: its own, read by no other
/// extension.
#[must_use]
pub fn data(doc: &LoroDoc, id: &str) -> LoroMap {
    doc.get_map(format!("ext:{id}").as_str())
}

/// Extension `id`'s container for one object of the project (a track, an
/// item, a section), by the object's id.
///
/// # Errors
///
/// Loro refused to make it.
pub fn object_data(doc: &LoroDoc, id: &str, object: &str) -> Result<LoroMap, String> {
    data(doc, id)
        .ensure_mergeable_map(object)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{data, object_data, requirements, set_requirements};
    use crate::project::{Requirement, Requirements};
    use crdt::loro::{LoroDoc, LoroValue, ValueOrContainer};

    #[test]
    fn a_projects_requirements_and_extension_data_live_in_its_document() {
        let doc = LoroDoc::new();
        assert_eq!(requirements(&doc).unwrap(), Requirements::default());
        let mut needs = Requirements::default();
        needs.need(Requirement::new("fts.metronome").versions("^0.1"));
        set_requirements(&doc, &needs).unwrap();
        doc.commit();
        assert_eq!(requirements(&doc).unwrap(), needs);

        data(&doc, "fts.metronome")
            .insert("count_in", 2_i64)
            .unwrap();
        object_data(&doc, "fts.metronome", "track-1")
            .unwrap()
            .insert("mute_click", true)
            .unwrap();
        doc.commit();
        let peer = LoroDoc::new();
        peer.import(&doc.export(crdt::loro::ExportMode::Snapshot).unwrap())
            .unwrap();
        assert_eq!(
            requirements(&peer).unwrap(),
            needs,
            "synced with the project"
        );
        let count = data(&peer, "fts.metronome").get("count_in");
        assert!(matches!(
            count,
            Some(ValueOrContainer::Value(LoroValue::I64(2)))
        ));
        let mute = object_data(&peer, "fts.metronome", "track-1")
            .unwrap()
            .get("mute_click");
        assert!(matches!(
            mute,
            Some(ValueOrContainer::Value(LoroValue::Bool(true)))
        ));
    }
}
