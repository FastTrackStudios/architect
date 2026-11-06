//! A vault built into a site, end to end: pages at their URLs, links
//! resolved by the vault's index (wikilinks, markdown and Zola `@/`),
//! backlinks, tags, search, a claimed block drawn, drafts left out, broken
//! links reported, and the output folder guarded.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::case_sensitive_file_extension_comparisons
)]

use std::path::Path;

use architect_sites::{
    Blocks, BuildError, Drawn, FnBlock, Options, Site, SiteError, WriteError, build,
};

fn touch(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A small docs vault: Zola's TOML sections, an Obsidian YAML note.
fn vault() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(
        root,
        "_index.md",
        "+++\ntitle = \"Demo\"\ndescription = \"A demo site.\"\n+++\n\nWelcome. Start with [[Getting started]].\n",
    );
    touch(
        root,
        "guide/_index.md",
        "+++\ntitle = \"Guide\"\nweight = 1\n+++\n\nThe guide.\n",
    );
    touch(
        root,
        "guide/start.md",
        "---\ntitle: Getting started\ntags: [intro, Setup]\n---\n\n# Getting started\n\nOpen [[shell]], read [its tabs](@/features/shell.md#tabs) and `[[not a link]]`.\n\n## Install\n\nRun the thing.\n",
    );
    touch(
        root,
        "guide/later.md",
        "+++\ntitle = \"Later\"\nweight = 5\n+++\n\nSee [[Nope]].\n",
    );
    touch(
        root,
        "features/shell.md",
        "+++\ntitle = \"The shell\"\nweight = 2\n[taxonomies]\ntags = [\"ui\"]\n+++\n\n![[pic.png]]\n\n## Tabs\n\nTabs live here.\n\n```shout\nhello\n```\n",
    );
    touch(root, "features/pic.png", "png");
    touch(root, "drafts/wip.md", "---\ndraft: true\n---\nNot yet.\n");
    dir
}

fn blocks() -> Blocks {
    Blocks::new().with(FnBlock::new("demo", &["shout"], |b| {
        Ok(Drawn::Html(format!(
            "<strong>{}</strong>",
            b.source.trim().to_uppercase()
        )))
    }))
}

fn read(out: &Path, rel: &str) -> String {
    std::fs::read_to_string(out.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn a_vault_builds_into_a_linked_site() {
    let vault = vault();
    let out_dir = tempfile::tempdir().unwrap();
    let out = out_dir.path().join("site");
    let report = build(&Options::new(vault.path(), ""), &blocks(), &out).expect("build");

    // Home, two sections, three notes, the draft left out; tag pages too.
    for page in [
        "index.html",
        "guide/index.html",
        "guide/start/index.html",
        "guide/later/index.html",
        "features/index.html",
        "features/shell/index.html",
        "tags/index.html",
        "tags/intro/index.html",
        "tags/ui/index.html",
        "404.html",
    ] {
        assert!(out.join(page).is_file(), "{page} missing");
    }
    assert!(!out.join("drafts/wip").exists());
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    assert!(
        report.problems[0]
            .to_string()
            .contains("guide/later.md: links to `Nope`")
    );

    let start = read(&out, "guide/start/index.html");
    // The site's title is the home page's.
    assert!(
        start.contains("<title>Getting started · Demo</title>"),
        "{start}"
    );
    // A wikilink by basename (any case) and a Zola link both resolve.
    assert!(
        start.contains("<a class=\"site-link\" href=\"/features/shell/\">shell</a>"),
        "{start}"
    );
    assert!(start.contains("href=\"/features/shell/#tabs\""));
    // Code is verbatim; the title heading is not repeated.
    assert!(start.contains("<code>[[not a link]]</code>"));
    assert_eq!(start.matches("<h1").count(), 1);
    // Tags, lower case, linked.
    assert!(start.contains("href=\"/tags/setup/\""));
    // Its contents rail and breadcrumbs.
    assert!(start.contains("href=\"#install\""));
    assert!(start.contains("<a href=\"/guide/\">Guide</a>"));

    let shell = read(&out, "features/shell/index.html");
    // Backlinks from the index: the Zola link and the wikilink count.
    assert!(shell.contains("Linked from"), "{shell}");
    assert!(shell.contains("href=\"/guide/start/\""));
    // The attachment is copied and linked, the block drawn.
    assert!(shell.contains("src=\"/features/pic.png\""));
    assert!(out.join("features/pic.png").is_file());
    assert!(
        shell.contains("data-by=\"demo\"><strong>HELLO</strong>"),
        "{shell}"
    );

    let later = read(&out, "guide/later/index.html");
    assert!(later.contains("site-broken-link"));

    // Reading order: the guide (weight 1) before features; in the guide,
    // the weighted note last.
    let home = read(&out, "index.html");
    let guide_at = home.find("href=\"/guide/\"").unwrap();
    let features_at = home.find("href=\"/features/\"").unwrap();
    assert!(guide_at < features_at);

    // Fingerprinted assets, and a search index with the headings.
    let assets: Vec<_> = std::fs::read_dir(out.join("_site"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        assets
            .iter()
            .any(|a| a.starts_with("site.") && a.ends_with(".css")),
        "{assets:?}"
    );
    let search = assets.iter().find(|a| a.starts_with("search.")).unwrap();
    let index = read(&out.join("_site"), search);
    assert!(index.contains("\"install\",\"Install\""), "{index}");
    let css = assets.iter().find(|a| a.ends_with(".css")).unwrap();
    assert!(read(&out.join("_site"), css).contains("--background:"));
}

#[test]
fn a_strict_build_fails_on_a_broken_link() {
    let vault = vault();
    let out = tempfile::tempdir().unwrap();
    let mut opts = Options::new(vault.path(), "Demo");
    opts.strict = true;
    let err = build(&opts, &blocks(), &out.path().join("site")).unwrap_err();
    assert!(
        matches!(err, BuildError::Read(SiteError::Problems(ref p)) if p.len() == 1),
        "{err}"
    );
}

#[test]
fn an_empty_or_missing_vault_fails() {
    let empty = tempfile::tempdir().unwrap();
    let err = Site::read(&Options::new(empty.path(), "x"), &Blocks::new()).unwrap_err();
    assert!(matches!(err, SiteError::Empty(_)));
    let err = Site::read(
        &Options::new(empty.path().join("nope"), "x"),
        &Blocks::new(),
    )
    .unwrap_err();
    assert!(matches!(err, SiteError::Missing(_)));
}

#[test]
fn a_rebuild_replaces_a_site_but_never_another_folder() {
    let vault = vault();
    let out_dir = tempfile::tempdir().unwrap();
    let out = out_dir.path().join("site");
    let opts = Options::new(vault.path(), "Demo");
    build(&opts, &blocks(), &out).unwrap();
    // A deleted note's page goes with the next build.
    std::fs::remove_file(vault.path().join("guide/later.md")).unwrap();
    build(&opts, &blocks(), &out).unwrap();
    assert!(!out.join("guide/later").exists());
    assert!(!out_dir.path().join("site.next").exists());

    // A folder holding anything else is left alone.
    let mine = out_dir.path().join("mine");
    touch(&mine, "notes.txt", "keep me");
    let err = build(&opts, &blocks(), &mine).unwrap_err();
    assert!(
        matches!(err, BuildError::Write(WriteError::NotASite(_))),
        "{err}"
    );
    assert!(mine.join("notes.txt").is_file());
}
