//! The language registry: filled by owner, asked by id, fence name and
//! file type, withdrawn by owner; a `[[languages]]` table reads into it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use architect_editor::languages::{APP, Grammar, Language, Languages, builtin};

const KEYFLOW: &str = r#"
[[languages]]
id = "keyflow"
name = "Keyflow chart"
extensions = ["kf"]
fences = ["kf"]
grammar = "grammars/keyflow.wasm"
highlights = "queries/highlights.scm"
comment = "//"
[languages.server]
command = "kf-lsp"
args = ["--stdio"]
"#;

#[derive(serde::Deserialize)]
struct Tables {
    languages: Vec<Language>,
}

#[test]
fn a_languages_table_reads_as_the_issue_writes_it() {
    let tables: Tables = toml::from_str(KEYFLOW).unwrap();
    let kf = &tables.languages[0];
    assert_eq!(kf.id, "keyflow");
    assert_eq!(kf.title(), "Keyflow chart");
    assert_eq!(
        kf.grammar,
        Grammar::File("grammars/keyflow.wasm".to_owned())
    );
    assert_eq!(kf.highlights.as_deref(), Some("queries/highlights.scm"));
    assert_eq!(kf.comment.as_deref(), Some("//"));
    let server = kf.server.as_ref().unwrap();
    assert_eq!(
        (server.command.as_str(), server.args.as_slice()),
        ("kf-lsp", &["--stdio".to_owned()][..])
    );
    kf.check().unwrap();
    // `linked` is the default, and what it is written back as.
    let linked: Language = toml::from_str("id = \"rust\"").unwrap();
    assert_eq!(linked.grammar, Grammar::Linked);
    assert_eq!(serde_json::to_value(&linked).unwrap()["grammar"], "linked");
}

#[test]
fn a_language_is_checked_as_written() {
    let ok = Language::linked("kf", "Keyflow");
    ok.check().unwrap();
    let bad = |edit: fn(&mut Language)| {
        let mut l = Language::linked("kf", "Keyflow");
        edit(&mut l);
        l.check().unwrap_err()
    };
    assert!(bad(|l| l.id = "Key Flow".to_owned()).contains("id"));
    assert!(bad(|l| l.extensions = vec![String::new()]).contains("extension"));
    assert!(bad(|l| l.fences = vec!["a b".to_owned()]).contains("fence"));
    assert!(bad(|l| l.grammar = Grammar::File("../g.wasm".to_owned())).contains("outside"));
    assert!(bad(|l| l.highlights = Some("/etc/q.scm".to_owned())).contains("outside"));
    assert!(bad(|l| l.server = Some(architect_editor::Server::default())).contains("command"));
}

#[test]
fn the_registry_is_asked_by_id_fence_and_file_type() {
    let languages = Languages::with_app(builtin());
    assert_eq!(languages.get("rust").unwrap().owner, APP);
    assert_eq!(languages.for_fence(" RS ").unwrap().language.id, "rust");
    assert_eq!(languages.for_fence("rust").unwrap().language.id, "rust");
    assert_eq!(
        languages.for_extension(".TS").unwrap().language.id,
        "typescript"
    );
    assert_eq!(
        languages.for_path("src/main.rs").unwrap().language.id,
        "rust"
    );
    assert_eq!(
        languages
            .for_path("notes/a.b/readme.md")
            .unwrap()
            .language
            .id,
        "markdown"
    );
    assert!(
        languages.for_path(".rs").is_none(),
        "a dotfile has no extension"
    );
    assert!(languages.for_path("Makefile").is_none());
    assert_eq!(languages.resolve("py").unwrap().language.id, "python");
    assert!(languages.resolve("kf").is_none());
}

#[test]
fn an_owner_adds_replaces_and_withdraws_its_languages() {
    let languages = Languages::with_app(builtin());
    let before = languages.revision();
    let kf = Language::linked("keyflow", "Keyflow chart")
        .extensions(&["kf"])
        .fences(&["kf"]);
    assert!(languages.set("lab.keyflow", vec![kf.clone()]));
    assert!(languages.revision() > before);
    assert_eq!(languages.for_fence("kf").unwrap().owner, "lab.keyflow");
    assert_eq!(
        languages.for_path("song.kf").unwrap().language.id,
        "keyflow"
    );
    // The same again changes nothing.
    let same = languages.revision();
    assert!(!languages.set("lab.keyflow", vec![kf.clone()]));
    assert_eq!(languages.revision(), same);
    // Replaced together.
    let renamed = Language {
        name: "Keyflow".to_owned(),
        ..kf
    };
    assert!(languages.set("lab.keyflow", vec![renamed]));
    assert_eq!(languages.get("keyflow").unwrap().language.name, "Keyflow");
    // A language two owners register is the first's; the second's shows
    // once the first goes.
    languages.set("lab.other", vec![Language::linked("rust", "Other Rust")]);
    assert_eq!(languages.get("rust").unwrap().owner, APP);
    assert_eq!(languages.of("lab.other").len(), 1);
    // Withdrawn together; the app's stay.
    assert!(languages.withdraw("lab.keyflow"));
    assert!(!languages.withdraw("lab.keyflow"));
    assert!(languages.for_fence("kf").is_none());
    assert!(languages.get("rust").is_some());
    assert!(languages.withdraw(APP));
    assert_eq!(languages.get("rust").unwrap().owner, "lab.other");
}
