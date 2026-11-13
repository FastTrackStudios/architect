//! The navigator's layout of the org's vaults in the browser, read-only
//! ([`NodeLayout`]): used by `app::org`, tested natively.
//!
//! Every change the navigator makes (a rename, move, delete, duplicate,
//! tag or property, from its context menu, its keys, the ⌘K palette or a
//! drag and drop) reaches its source through one call,
//! `NavigatorSource::write` (`architect_navigator`'s `Navigator::write`,
//! which updates its own state only after the source took the change).
//! Refusing that call is what keeps the browser from showing a change the
//! node never got.

use architect_navigator::source::{FileOp, FolderMeta, Listener, Subscription};
use architect_navigator::{MemorySource, NavigatorSource, NoteFile};

/// The navigator's view of the org's vaults: the notes as listed and
/// read, read-only. Renames, moves, deletes, tags and properties change
/// files the node does not take from the browser yet; doing them in the
/// tab's memory alone would look saved and not be. New notes go through
/// the app (`app::org::OrgVaults::create`).
pub struct NodeLayout(pub MemorySource);

/// Why the navigator does nothing, said where it tried.
pub const READ_ONLY: &str = "Renaming, moving, deleting, tags and properties are read-only in the \
                             browser for now: do them in the desktop app. Editing a note's text \
                             and new notes are saved.";

impl NavigatorSource for NodeLayout {
    fn name(&self) -> String {
        self.0.name()
    }
    fn folders(&self) -> Vec<String> {
        self.0.folders()
    }
    fn child_order(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        self.0.child_order()
    }
    fn files(&self) -> Vec<NoteFile> {
        self.0.files()
    }
    fn browses_properties(&self) -> bool {
        self.0.browses_properties()
    }
    fn mounts(&self) -> Vec<architect_navigator::Mount> {
        self.0.mounts()
    }
    fn folder_meta(&self) -> std::collections::BTreeMap<String, FolderMeta> {
        self.0.folder_meta()
    }
    fn subscribe(&self, listener: Listener) -> Subscription {
        self.0.subscribe(listener)
    }
    fn resource_url(&self, path: &str) -> Option<String> {
        self.0.resource_url(path)
    }
    fn can_write(&self) -> bool {
        false
    }
    fn write(&self, _ops: &[FileOp]) -> Result<(), String> {
        Err(READ_ONLY.to_owned())
    }
    fn search_text(&self, query: &str) -> Option<Vec<String>> {
        self.0.search_text(query)
    }
    fn search_excerpts(&self, query: &str) -> Option<Vec<(String, String)>> {
        self.0.search_excerpts(query)
    }
    fn read_text(&self, path: &str) -> Option<String> {
        self.0.read_text(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use architect_navigator::PropertyValue;

    /// #628 M3: every kind of change the navigator can ask for is refused
    /// on the org's vaults, and the vault is as it was. The navigator's
    /// menu, keys, palette commands and drag and drop all end in this one
    /// call.
    #[test]
    fn the_org_s_layout_takes_no_change_from_the_navigator() {
        let source = MemorySource::new("acme")
            .with_folder("demo".to_owned())
            .with_files(vec![NoteFile::new("demo/Welcome.md".to_owned(), 1, 1)]);
        source.set_text(
            NoteFile::new("demo/Welcome.md".to_owned(), 1, 1),
            "# Welcome\n".to_owned(),
        );
        let layout = NodeLayout(source.clone());
        let before = (
            source.folders(),
            source.files(),
            source.read_text("demo/Welcome.md"),
        );
        assert!(!layout.can_write());
        let path = || "demo/Welcome.md".to_owned();
        let paths = || vec![path()];
        let ops = [
            FileOp::CreateFolder("demo/New".into()),
            FileOp::CreateFile {
                path: "demo/x.md".into(),
                text: String::new(),
            },
            FileOp::Move {
                from: path(),
                to: "demo/Renamed.md".into(),
            },
            FileOp::SetProperty {
                path: path(),
                key: "k".into(),
                value: PropertyValue::Bool(true),
            },
            FileOp::RemoveProperty {
                path: path(),
                key: "k".into(),
            },
            FileOp::AddTag {
                paths: paths(),
                tag: "t".into(),
            },
            FileOp::RemoveTag {
                paths: paths(),
                tag: "t".into(),
            },
            FileOp::ClearTags { paths: paths() },
            FileOp::RenameTag {
                from: "t".into(),
                to: "u".into(),
            },
            FileOp::DeleteTag("t".into()),
            FileOp::RenamePropertyKey {
                from: "k".into(),
                to: "j".into(),
            },
            FileOp::DeletePropertyKey("k".into()),
            FileOp::RenamePropertyValue {
                key: "k".into(),
                from: "a".into(),
                to: "b".into(),
            },
            FileOp::Copy {
                from: path(),
                to: "demo/Copy.md".into(),
            },
            FileOp::MoveFolder {
                from: "demo".into(),
                to: "other".into(),
            },
            FileOp::CopyFolder {
                from: "demo".into(),
                to: "other".into(),
            },
            FileOp::Trash(path()),
        ];
        for op in ops {
            assert_eq!(
                layout.write(std::slice::from_ref(&op)),
                Err(READ_ONLY.to_owned()),
                "{op:?}"
            );
        }
        assert_eq!(
            before,
            (
                source.folders(),
                source.files(),
                source.read_text("demo/Welcome.md")
            )
        );
    }
}
