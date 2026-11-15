//! A script's run is one undo step across the app's documents, kept when
//! it succeeds and taken back when it fails, for both script runners: a
//! TypeScript action (`architect_pages`' `TsRunner`, #536) and a Luau one
//! (`architect_extensions::host::LuauRunner`), each given the documents as
//! its transactions as the app gives them (`apps/ui/src/extensions.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use architect::action::{ActionCaller, ActionError, ActionList, Principal};
use architect_extensions::documents::{CrdtDocuments, Documents as _};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, LuauRunner, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::{Origin, Package};
use architect_pages::host::AppHost;
use architect_pages::js::action::TsRunner;
use architect_scripting::Transactions;
use architect_settings::Registry;
use futures::executor::block_on;

fn manifest(id: &str, kind: &str, script: &str) -> String {
    let prefix = id.replace(['.', '-'], "_").to_uppercase();
    format!(
        "id = \"{id}\"\nname = \"Undo\"\nversion = \"0.1.0\"\nkind = \"{kind}\"\n\
         [capabilities]\nreads = [\"documents\"]\nwrites = [\"documents\"]\n\
         [[actions]]\nid = \"{prefix}_KEEP\"\nname = \"Keep\"\neffect = \"mutating\"\nscript = \"keep.{script}\"\n\
         [[actions]]\nid = \"{prefix}_FAIL\"\nname = \"Fail\"\neffect = \"mutating\"\nscript = \"fail.{script}\"\n"
    )
}

fn write(dir: &Path, files: &[(&str, &str)]) {
    std::fs::create_dir_all(dir).unwrap();
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
}

fn run(list: &ActionList, id: &str) -> Result<Vec<u8>, ActionError> {
    block_on(list.run_json_async_with(&ActionCaller::app(), id, b""))
}

fn host(docs: &CrdtDocuments, tmp: &Path) -> (ActionList, Host) {
    let list = ActionList::new();
    let undo: Arc<dyn Transactions> = Arc::new(docs.clone());
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_documents(docs.clone())
        .with_runner(LuauRunner::new().with_transactions(Arc::clone(&undo)))
        .with_runner(TsRunner::new(AppHost::new(list.clone())).with_transactions(undo));
    // Its host-service calls (`EXT_PAGE_CALL`), as the app installs them.
    let ext = Extensions::new(host, Installs::new(tmp.join("installs")));
    ext.install_contribution_actions().unwrap();
    (list, ext.host().clone())
}

fn documents() -> CrdtDocuments {
    let docs = CrdtDocuments::new().with_space("notes");
    let app = Principal::Service {
        name: "lab".to_owned(),
    };
    docs.create("notes/kept.md", "a", &app).unwrap();
    docs
}

#[test]
fn a_failed_typescript_run_takes_back_what_it_made_and_edited() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("ts");
    write(
        &dir,
        &[
            (
                "extension.toml",
                &manifest("lab.undo-ts", "typescript", "ts"),
            ),
            (
                "keep.ts",
                "import { documents } from \"architect\";\nexport default async () => {\n  await documents.create(\"notes/new.md\", \"made\");\n  await documents.edit(\"notes/kept.md\", [{ append: { text: \" b\" } }]);\n};\n",
            ),
            (
                "fail.ts",
                "import { documents } from \"architect\";\nexport default async () => {\n  await documents.create(\"notes/half.md\", \"made\");\n  await documents.edit(\"notes/kept.md\", [{ append: { text: \" c\" } }]);\n  throw new Error(\"halfway\");\n};\n",
            ),
        ],
    );
    let docs = documents();
    let (list, host) = host(&docs, tmp.path());
    host.load(&Package::read(&dir).unwrap(), Origin::Dev)
        .unwrap();
    let why = run(&list, "LAB_UNDO_TS_FAIL").unwrap_err().to_string();
    assert!(
        why.contains("halfway") && why.contains("fail.ts:5:"),
        "{why}"
    );
    assert!(
        docs.text("notes/half.md").is_none(),
        "its new note went with its step"
    );
    assert_eq!(docs.text("notes/kept.md").as_deref(), Some("a"));
    run(&list, "LAB_UNDO_TS_KEEP").unwrap();
    assert_eq!(docs.text("notes/new.md").as_deref(), Some("made"));
    assert_eq!(docs.text("notes/kept.md").as_deref(), Some("a b"));
    assert_eq!(
        docs.steps("notes/kept.md").last().map(String::as_str),
        Some("Undo: Keep")
    );
}

#[test]
fn a_failed_luau_run_takes_back_what_it_made_and_edited() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("luau");
    write(
        &dir,
        &[
            ("extension.toml", &manifest("lab.undo-luau", "luau", "luau")),
            (
                "keep.luau",
                "host.create_document(\"notes/new.md\", \"made\")\nhost.edit_document(\"notes/kept.md\", { { append = { text = \" b\" } } }, \"e\")\nreturn true\n",
            ),
            (
                "fail.luau",
                "host.create_document(\"notes/half.md\", \"made\")\nhost.edit_document(\"notes/kept.md\", { { append = { text = \" c\" } } }, \"e\")\nerror(\"halfway\")\n",
            ),
        ],
    );
    let docs = documents();
    let (list, host) = host(&docs, tmp.path());
    host.load(&Package::read(&dir).unwrap(), Origin::Dev)
        .unwrap();
    let why = run(&list, "LAB_UNDO_LUAU_FAIL").unwrap_err().to_string();
    assert!(why.contains("halfway"), "{why}");
    assert!(
        docs.text("notes/half.md").is_none(),
        "its new note went with its step"
    );
    assert_eq!(docs.text("notes/kept.md").as_deref(), Some("a"));
    run(&list, "LAB_UNDO_LUAU_KEEP").unwrap();
    assert_eq!(docs.text("notes/kept.md").as_deref(), Some("a b"));
    assert_eq!(docs.text("notes/new.md").as_deref(), Some("made"));
}
