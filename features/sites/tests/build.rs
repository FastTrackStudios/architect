//! A vault built into a site, end to end: pages at their URLs, links
//! resolved by the vault's index (wikilinks, markdown and Zola `@/`),
//! backlinks, tags, search, a claimed block drawn, drafts left out, broken
//! links reported, and the output folder guarded. A scoped site (a folder,
//! a tag, a note list, a base) publishes its notes and nothing else: no
//! link, embed, backlink, base row, search entry or attachment reaches a
//! note outside it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::case_sensitive_file_extension_comparisons
)]

use std::path::Path;

use architect_sites::{
    Blocks, BuildError, Drawn, FnBlock, Options, Scope, Site, SiteError, WriteError, build,
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
    touch(root, "gigs/one.md", "---\nfee: 200\nkey: G\n---\nA gig.\n");
    touch(root, "gigs/two.md", "---\nfee: 50\nkey: C\n---\nAnother.\n");
    touch(
        root,
        "gigs/Gigs.base",
        "filters:\n  and:\n    - file.inFolder(\"gigs\")\nviews:\n  - type: cards\n    name: All gigs\n    order:\n      - file.name\n      - key\n",
    );
    touch(
        root,
        "gigs/_index.md",
        "+++\ntitle = \"Gigs\"\n+++\n\n```base\nfilters:\n  and:\n    - file.inFolder(\"gigs\")\nviews:\n  - type: table\n    name: By fee\n    order:\n      - file.name\n      - fee\n    sort:\n      - property: fee\n        direction: DESC\n```\n\n```base\nGigs.base\n```\n",
    );
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
fn a_base_fence_is_drawn_as_its_views_over_the_vault() {
    let vault = vault();
    let out_dir = tempfile::tempdir().unwrap();
    let out = out_dir.path().join("site");
    let report = build(&Options::new(vault.path(), "Demo"), &blocks(), &out).expect("build");
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);

    let gigs = read(&out, "gigs/index.html");
    // The YAML fence: a table, sorted by fee, each name its note's link.
    assert!(gigs.contains("data-by=\"architect.bases\""), "{gigs}");
    assert!(gigs.contains("<figcaption>By fee</figcaption>"), "{gigs}");
    let one = gigs
        .find("href=\"/gigs/one/\">One</a>")
        .expect("one linked");
    let two = gigs
        .find("href=\"/gigs/two/\">Two</a>")
        .expect("two linked");
    assert!(one < two, "sorted by fee, descending");
    assert!(gigs.contains("<td>200</td>"));
    // The fence naming a `.base` file: its cards.
    assert!(
        gigs.contains("data-view=\"cards\"><figcaption>All gigs</figcaption>"),
        "{gigs}"
    );
    assert!(gigs.contains("<b>key</b> G"));

    // Turned off, the fence is a code block.
    let mut opts = Options::new(vault.path(), "Demo");
    opts.bases = false;
    build(&opts, &blocks(), &out).unwrap();
    assert!(read(&out, "gigs/index.html").contains("data-lang=\"base\""));
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

/// A vault with a shared folder and private notes it links to.
fn mixed() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(
        root,
        "Private/plan.md",
        "---\ntags: [secret]\n---\nThe secret plan.\n",
    );
    touch(root, "Private/cover.png", "private png");
    touch(
        root,
        "Shared/index.md",
        "---\ntitle: The album\n---\nRead [[Private/plan|the plan]], [this](../Private/plan.md) and [[track]].\n\n![[plan]]\n\n![[cover.png]]\n\n![[art.png]]\n",
    );
    touch(root, "Shared/art.png", "shared png");
    touch(
        root,
        "Shared/track.md",
        "---\ntags: [mix]\nkey: G\n---\nBack [[index|home]]. Not [[plan]].\n\n```base\nviews:\n  - type: table\n    name: Everything\n    order:\n      - file.name\n```\n",
    );
    touch(
        root,
        "Elsewhere/also-mix.md",
        "---\ntags: [mix/final]\n---\nAlso a mix. See [[plan]].\n",
    );
    touch(root, "Elsewhere/links-in.md", "Links to [[track]].\n");
    dir
}

fn all_html(out: &Path) -> String {
    fn walk(dir: &Path, into: &mut String) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, into);
            } else if path
                .extension()
                .is_some_and(|e| e == "html" || e == "json" || e == "xml")
            {
                into.push_str(&std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut all = String::new();
    walk(out, &mut all);
    all
}

#[test]
fn a_folder_scope_publishes_the_folder_and_nothing_outside_it() {
    let vault = mixed();
    let out_dir = tempfile::tempdir().unwrap();
    let out = out_dir.path().join("site");
    let mut opts = Options::new(vault.path(), "");
    opts.base = "/abc123/".to_owned();
    opts.scope = Scope::Folder("Shared".to_owned());
    let report = build(&opts, &blocks(), &out).expect("build");

    // Rooted at the folder: its note is home, its other note one level down.
    let home = read(&out, "index.html");
    assert!(home.contains("The album"), "{home}");
    let track = read(&out, "track/index.html");
    assert!(track.contains("href=\"/abc123/\""), "{track}");
    assert!(home.contains("href=\"/abc123/track/\""), "{home}");

    // Links to a note outside: plain text, no URL, no name in a title.
    assert!(
        home.contains("<span class=\"site-outside\">the plan</span>"),
        "{home}"
    );
    assert!(
        home.contains("<span class=\"site-outside\">this</span>"),
        "{home}"
    );
    assert!(
        track.contains("<span class=\"site-outside\">plan</span>"),
        "{track}"
    );
    // The embed of a note outside is gone, its own attachment is kept.
    assert!(home.contains("/abc123/Shared/art.png"), "{home}");
    assert!(out.join("Shared/art.png").is_file());
    // Nothing outside the folder anywhere in the site: pages, search,
    // feeds, tag pages, backlinks, the base's rows, the attachment.
    let everything = all_html(&out);
    for leak in [
        "secret plan",
        "Private",
        "Elsewhere",
        "also-mix",
        "Links to",
        "links-in",
        "cover.png",
        "secret",
    ] {
        assert!(!everything.contains(leak), "`{leak}` leaked");
    }
    assert!(!out.join("Private").exists());
    // An out-of-scope link is not a problem to report.
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    // The base lists the scope's notes only.
    assert!(track.contains("data-by=\"architect.bases\""), "{track}");
    assert!(
        track.contains("href=\"/abc123/track/\">Track</a>"),
        "{track}"
    );
}

#[test]
fn a_tag_a_note_list_and_a_base_scope_publish_only_their_notes() {
    let vault = mixed();
    let out_dir = tempfile::tempdir().unwrap();
    let pages = |scope: Scope| {
        let mut opts = Options::new(vault.path(), "Shared");
        opts.scope = scope;
        Site::read(&opts, &blocks())
            .unwrap()
            .pages
            .iter()
            .filter_map(|p| p.rel_path.clone())
            .collect::<Vec<_>>()
    };
    // A tag holds its nested tags too, any case.
    let mut tagged = pages(Scope::Tag("#MIX".to_owned()));
    tagged.sort();
    assert_eq!(tagged, ["Elsewhere/also-mix.md", "Shared/track.md"]);
    assert_eq!(
        pages(Scope::Notes(vec!["Shared/track".to_owned()])),
        ["Shared/track.md"]
    );
    // One note is the site's home page.
    let mut opts = Options::new(vault.path(), "");
    opts.scope = Scope::Notes(vec!["Shared/track.md".to_owned()]);
    let one = Site::read(&opts, &blocks()).unwrap();
    assert_eq!(one.pages.len(), 1);
    assert_eq!(one.pages[0].url, "/");
    assert!(one.pages[0].html.contains("Back"), "{}", one.pages[0].html);

    touch(
        vault.path(),
        "Mixes.base",
        "filters:\n  and:\n    - file.hasTag(\"mix\")\nviews:\n  - type: table\n    name: Mixes\n",
    );
    // The base's own evaluator decides (its `hasTag` is exact).
    assert_eq!(
        pages(Scope::Base("Mixes.base".to_owned())),
        ["Shared/track.md"]
    );
    assert_eq!(pages(Scope::Base("Mixes".to_owned())), ["Shared/track.md"]);

    // A scope with nothing in it builds nothing.
    let mut opts = Options::new(vault.path(), "");
    opts.scope = Scope::Folder("Nowhere".to_owned());
    assert!(matches!(
        build(&opts, &blocks(), &out_dir.path().join("none")),
        Err(BuildError::Read(SiteError::Empty(_)))
    ));
}

/// A setlist: charts with an `instrument`, each attaching its part.
fn setlist() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let v = dir.path();
    touch(v, "Set/index.md", "---\ntitle: Carols\n---\nThe set.\n");
    touch(
        v,
        "Set/02 silent.md",
        "---\ntitle: Silent Night\ninstrument: violin\n---\n![[silent-violin.pdf]]\n",
    );
    touch(
        v,
        "Set/01 joy.md",
        "---\ntitle: Joy to the World\ninstrument: [violin, cello]\n---\n![[joy-violin.pdf]] and [the cello part](joy-cello.pdf)\n",
    );
    touch(
        v,
        "Set/03 noel.md",
        "---\ntitle: Noel\ninstrument: cello\n---\n![[noel-cello.pdf]] see [[01 joy]]\n",
    );
    touch(
        v,
        "Set/draft.md",
        "---\npublish: false\ninstrument: violin\n---\nNot yet.\n",
    );
    for file in ["silent-violin", "joy-violin", "joy-cello", "noel-cello"] {
        touch(v, &format!("Set/{file}.pdf"), file);
    }
    touch(
        v,
        "Elsewhere/other.md",
        "---\ninstrument: violin\n---\nNot in the set.\n",
    );
    dir
}

#[test]
fn a_listing_is_the_scopes_notes_a_filter_holds_for_in_list_order() {
    use architect_sites::listing::{parse_filter, select};
    let dir = setlist();
    let vault = vault::obsidian::Vault::open(dir.path()).expect("vault");
    let scope = Scope::Folder("Set".to_owned());
    let titles = |scope: &Scope, filter: &str| -> Vec<String> {
        select(&vault, scope, &parse_filter(filter).expect("filter"))
            .into_iter()
            .map(|e| e.title)
            .collect()
    };
    // By path under a folder; the hidden note never; nothing outside.
    assert_eq!(
        titles(&scope, ""),
        ["Joy to the World", "Silent Night", "Noel", "Carols"]
    );
    assert_eq!(
        titles(&scope, "instrument.contains(\"violin\")"),
        ["Joy to the World", "Silent Night"]
    );
    assert_eq!(titles(&scope, "instrument == \"cello\""), ["Noel"]);
    // A note list is in its own order.
    let list = Scope::Notes(vec![
        "Set/03 noel.md".to_owned(),
        "Set/01 joy".to_owned(),
        "Set/03 noel.md".to_owned(),
    ]);
    assert_eq!(titles(&list, ""), ["Noel", "Joy to the World"]);
    // Each note with the files it attaches, in the order it names them.
    let joy = select(
        &vault,
        &scope,
        &parse_filter("file.name == \"01 joy\"").expect("filter"),
    );
    assert_eq!(joy.len(), 1);
    assert_eq!(joy[0].files, ["Set/joy-violin.pdf", "Set/joy-cello.pdf"]);
}

#[test]
fn a_filter_publishes_only_the_notes_it_holds_for() {
    let dir = setlist();
    let out = tempfile::tempdir().expect("out");
    let site = out.path().join("site");
    let mut opts = Options::new(dir.path(), "Cellos");
    opts.scope = Scope::Folder("Set".to_owned());
    opts.filter = Some("instrument == \"cello\"".to_owned());
    let built = Site::read(&opts, &blocks()).expect("read");
    let notes: Vec<&str> = built
        .pages
        .iter()
        .filter_map(|p| p.rel_path.as_deref())
        .collect();
    assert_eq!(notes, ["Set/03 noel.md"]);
    assert_eq!(built.attachments, ["Set/noel-cello.pdf"]);
    architect_sites::write(&built, &site).expect("write");
    let html = all_html(&site);
    assert!(html.contains("Noel"), "{html}");
    // The violins' notes and files are outside it: no page, no file, no
    // title in a link, the nav or the search index.
    assert!(
        !html.contains("Silent Night") && !html.contains("Joy to the World"),
        "{html}"
    );
    assert!(site.join("Set/noel-cello.pdf").exists());
    assert!(!site.join("Set/joy-violin.pdf").exists());
    assert!(!site.join("Set/joy-cello.pdf").exists());
    let index = std::fs::read_dir(site.join("_site"))
        .expect("assets")
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("search."))
        .map(|e| std::fs::read_to_string(e.path()).expect("index"))
        .expect("a search index");
    assert!(
        !index.contains("Joy") && !index.contains("Silent"),
        "{index}"
    );

    opts.filter = Some("instrument ==".to_owned());
    assert!(matches!(
        Site::read(&opts, &blocks()),
        Err(SiteError::Filter(_))
    ));
}

/// The page's Content-Security-Policy (the node's) runs no inline script
/// but the theme's, by its hash: every other `<script>` is a file, and
/// live reload is a `<meta>` the site's script reads.
#[test]
fn a_page_runs_no_inline_script_but_the_hashed_theme() {
    use base64::Engine as _;
    use sha2::Digest as _;
    let hash = base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(
        architect_sites::ui::THEME_BOOT.as_bytes(),
    ));
    assert_eq!(
        architect_sites::ui::THEME_BOOT_HASH,
        format!("'sha256-{hash}'")
    );

    let vault = vault();
    let out_dir = tempfile::tempdir().unwrap();
    let out = out_dir.path().join("site");
    let mut opts = Options::new(vault.path(), "");
    opts.live_reload = Some("/s/abc/_live".to_owned());
    build(&opts, &blocks(), &out).expect("build");
    let page = read(&out, "guide/start/index.html");
    let inline = format!("<script>{}</script>", architect_sites::ui::THEME_BOOT);
    assert_eq!(page.matches(&inline).count(), 1, "{page}");
    for (at, _) in page.match_indices("<script") {
        let tag = page.get(at..).unwrap_or_default();
        assert!(
            tag.starts_with(&inline) || tag.starts_with("<script src="),
            "an inline script: {}",
            tag.get(..80).unwrap_or(tag)
        );
    }
    assert!(
        page.contains("<meta name=\"site-live\" content=\"/s/abc/_live\""),
        "{page}"
    );
}

/// A vault file named `index.html` (or `404.html`) is not published: at a
/// folder's URL it would be served as one of the site's own pages
/// (`architect::host::BUILT_PAGE`), and the page's player trusts those.
#[test]
fn a_vault_index_html_is_not_published() {
    let dir = tempfile::tempdir().unwrap();
    touch(
        dir.path(),
        "gig.md",
        "---\ntitle: Gig\n---\n[[raw/index.html]] [[raw/404.html]] [[raw/page.html]]\n",
    );
    touch(dir.path(), "raw/index.html", "<p>not a page</p>");
    touch(dir.path(), "raw/404.html", "<p>not a page</p>");
    touch(dir.path(), "raw/page.html", "<p>an attachment</p>");
    let out = tempfile::tempdir().unwrap();
    let report = build(
        &Options::new(dir.path(), "Gig"),
        &Blocks::new(),
        &out.path().join("site"),
    )
    .unwrap();
    let site = out.path().join("site");
    assert!(!site.join("raw/index.html").exists());
    assert!(!site.join("raw/404.html").exists());
    assert!(
        site.join("raw/page.html").is_file(),
        "other files are copied"
    );
    let said: Vec<&str> = report.problems.iter().map(|p| p.page.as_str()).collect();
    assert!(
        said.contains(&"raw/index.html") && said.contains(&"raw/404.html"),
        "{said:?}"
    );
}
