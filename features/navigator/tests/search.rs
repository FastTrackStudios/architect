//! The filter search, as Notebook Navigator's (#338): names and aliases,
//! tags, properties, dates, folders, extensions and tasks; AND and OR,
//! exclusions, quoted text; relative and absolute dates on the created or
//! modified time; the box kept across runs, the vault-wide search, and the
//! full text through the source.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;

use architect_navigator::list::Clock;
use architect_navigator::period::DayMonthOrder;
use architect_navigator::{
    FilterSearch, Join, MemorySource, Mods, NavTarget, NavigatorSettings, NavigatorSource,
    NavigatorState, NoteFile, Operand, Pane, PropertyValue, SearchContext, SearchProvider,
    VaultIndex,
};
use chrono::NaiveDate;

/// 2026-11-05 14:13 UTC.
const NOW: i64 = 1_793_880_000_000;
const DAY: i64 = 86_400_000;

fn text(t: &str) -> PropertyValue {
    PropertyValue::Text(t.to_owned())
}

fn vault() -> MemorySource {
    MemorySource::new("Lab")
        .with_file(
            NoteFile::new("Work/Meetings/Weekly sync.md", NOW - 40 * DAY, NOW)
                .tag("work/meetings")
                .property("status", text("open"))
                .property("aliases", PropertyValue::List(vec!["Standup".into()]))
                .tasks(3, 1),
        )
        .with_file(
            NoteFile::new("Work/Plan.md", NOW - 2 * DAY, NOW - 10 * DAY)
                .tag("work")
                .property("status", text("done"))
                .property("owner", text(""))
                .tasks(2, 0),
        )
        .with_file(
            NoteFile::new("Home/Café notes.md", NOW - 400 * DAY, NOW - 3 * DAY)
                .tag("home")
                .property(
                    "type",
                    PropertyValue::List(vec!["recipe".into(), "list".into()]),
                ),
        )
        .with_file(NoteFile::new(
            "Archive/Old plan.md",
            NOW - 900 * DAY,
            NOW - 800 * DAY,
        ))
        .with_file(NoteFile::new("Home/Brief.pdf", NOW - DAY, NOW - DAY))
        .with_file(NoteFile::new("Root note.md", NOW, NOW).preview("The tomato harvest."))
}

const fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()
}

/// The paths `query` matches, sorted.
fn hits(query: &str) -> Vec<String> {
    let src = vault();
    let search = FilterSearch::parse(query, today(), DayMonthOrder::Dmy);
    let cx = SearchContext::default();
    let mut out: Vec<String> = src
        .files()
        .into_iter()
        .filter(|f| search.matches(f, &cx))
        .map(|f| f.path)
        .collect();
    out.sort();
    out
}

#[test]
fn words_match_names_and_aliases_in_any_case_with_accents_folded() {
    assert_eq!(hits("plan"), ["Archive/Old plan.md", "Work/Plan.md"]);
    assert_eq!(hits("standup"), ["Work/Meetings/Weekly sync.md"]);
    // Every word, across the name and the aliases.
    assert_eq!(hits("weekly STANDUP"), ["Work/Meetings/Weekly sync.md"]);
    assert_eq!(hits("cafe"), ["Home/Café notes.md"]);
    assert!(hits("plan standup").is_empty());
}

#[test]
fn tags_properties_and_exclusions() {
    assert_eq!(
        hits("#work"),
        ["Work/Meetings/Weekly sync.md", "Work/Plan.md"],
        "a tag holds the tags below it"
    );
    assert_eq!(hits("#work/meetings"), ["Work/Meetings/Weekly sync.md"]);
    assert_eq!(hits("#").len(), 3, "any tag");
    assert_eq!(
        hits("-#"),
        ["Archive/Old plan.md", "Home/Brief.pdf", "Root note.md"],
        "no tag"
    );
    assert_eq!(hits(".status=done"), ["Work/Plan.md"]);
    assert_eq!(hits(".stat").len(), 2, "a key by its start");
    assert_eq!(hits(".owner=null"), ["Work/Plan.md"], "a key with no value");
    assert!(hits(".owner=\"null\"").is_empty(), "quoted, the word null");
    assert_eq!(hits(".type=recipe"), ["Home/Café notes.md"]);
    assert_eq!(
        hits("#work -.status=done"),
        ["Work/Meetings/Weekly sync.md"]
    );
    assert_eq!(hits("plan -old"), ["Work/Plan.md"]);
}

#[test]
fn and_or_join_tags_and_properties_alone_and_are_words_elsewhere() {
    // AND binds tighter than OR.
    assert_eq!(
        hits("#home OR #work AND .status=done"),
        ["Home/Café notes.md", "Work/Plan.md"]
    );
    assert!(FilterSearch::parse("#a OR #b", today(), DayMonthOrder::Dmy).is_expression());
    // Mixed with a word: AND and OR are words to find.
    let mixed = FilterSearch::parse("plan OR #work", today(), DayMonthOrder::Dmy);
    assert!(!mixed.is_expression());
    assert_eq!(mixed.words(), ["plan", "or"]);
    // A leading or doubled operator: words too.
    assert!(!FilterSearch::parse("OR #a", today(), DayMonthOrder::Dmy).is_expression());
    assert!(!FilterSearch::parse("#a AND OR #b", today(), DayMonthOrder::Dmy).is_expression());
}

#[test]
fn quoted_terms_are_text() {
    let src = MemorySource::new("q")
        .with_file(NoteFile::new("a/.Folder note.md", NOW, NOW))
        .with_file(NoteFile::new("a/#work log.md", NOW, NOW).tag("work"))
        .with_file(NoteFile::new("a/Other.md", NOW, NOW).tag("work"));
    let run = |q: &str| {
        let search = FilterSearch::parse(q, today(), DayMonthOrder::Dmy);
        let mut out: Vec<String> = src
            .files()
            .into_iter()
            .filter(|f| search.matches(f, &SearchContext::default()))
            .map(|f| f.path)
            .collect();
        out.sort();
        out
    };
    assert_eq!(run("\".folder\""), ["a/.Folder note.md"]);
    assert_eq!(run("\"#work log\""), ["a/#work log.md"]);
    assert_eq!(run("#work -\"#work\""), ["a/Other.md"]);
}

#[test]
fn dates_relative_and_absolute_on_created_or_modified() {
    // The list's own date: modified.
    assert_eq!(
        hits("@today"),
        ["Root note.md", "Work/Meetings/Weekly sync.md"]
    );
    assert_eq!(hits("@yesterday"), ["Home/Brief.pdf"]);
    assert_eq!(hits("@last7d").len(), 4);
    assert_eq!(hits("@c:2026-11-03"), ["Work/Plan.md"]);
    assert_eq!(hits("@m:2026-10-26"), ["Work/Plan.md"]);
    assert_eq!(hits("@c:2024"), ["Archive/Old plan.md"]);
    assert_eq!(
        hits("@c:2026-09-01..2026-11-04"),
        [
            "Home/Brief.pdf",
            "Work/Meetings/Weekly sync.md",
            "Work/Plan.md"
        ]
    );
    assert_eq!(hits("@..2025").len(), 1);
    assert_eq!(hits("-@thismonth"), ["Archive/Old plan.md", "Work/Plan.md"]);
    assert_eq!(
        hits("@c:2026-w45"),
        ["Home/Brief.pdf", "Root note.md", "Work/Plan.md"]
    );
    // By created when the list sorts by it.
    let search = FilterSearch::parse("@2026-09-26", today(), DayMonthOrder::Dmy);
    let by_created = SearchContext {
        by_created: true,
        ..SearchContext::default()
    };
    let files: Vec<String> = vault()
        .files()
        .into_iter()
        .filter(|f| search.matches(f, &by_created))
        .map(|f| f.path)
        .collect();
    assert_eq!(files, ["Work/Meetings/Weekly sync.md"]);
}

#[test]
fn folders_extensions_and_tasks() {
    assert_eq!(hits("folder:meet"), ["Work/Meetings/Weekly sync.md"]);
    assert_eq!(hits("folder:/work"), ["Work/Plan.md"], "right in it");
    assert_eq!(hits("folder:/"), ["Root note.md"]);
    assert_eq!(
        hits("-folder:archive -folder:work"),
        ["Home/Brief.pdf", "Home/Café notes.md", "Root note.md"]
    );
    assert_eq!(hits("ext:pdf"), ["Home/Brief.pdf"]);
    assert_eq!(hits("-ext:md"), ["Home/Brief.pdf"]);
    assert_eq!(
        hits("has:task"),
        ["Work/Meetings/Weekly sync.md"],
        "one not done"
    );
    assert_eq!(hits("#work -has:task"), ["Work/Plan.md"]);
}

fn state(src: &MemorySource, settings: NavigatorSettings) -> NavigatorState {
    let mut s = NavigatorState::new(
        VaultIndex::read(src),
        settings,
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    s.set_view(Pane::Navigation, 2000.0);
    s.set_view(Pane::Files, 600.0);
    s
}

fn listed(s: &NavigatorState) -> Vec<String> {
    let mut v: Vec<String> = s.list_order().into_iter().map(str::to_owned).collect();
    v.sort();
    v
}

#[test]
fn the_box_narrows_the_list_clicks_write_to_it_and_it_is_kept() {
    let src = vault();
    let mut s = state(
        &src,
        NavigatorSettings {
            include_descendants: true,
            ..NavigatorSettings::default()
        },
    );
    s.set_search("#work");
    assert_eq!(listed(&s).len(), 2);
    // ⇧⌘-click: OR, in the expression.
    s.click_tag(
        "home",
        Mods {
            command: true,
            shift: true,
        },
    );
    assert_eq!(s.search().text(), "#work OR #home");
    assert!(s.search_open);
    // A search with a word: the tag is a term of its own.
    s.set_search("plan");
    s.toggle_query(Operand::tag("work"), Join::And);
    assert_eq!(s.search().text(), "plan #work");
    assert_eq!(listed(&s), ["Work/Plan.md"]);
    s.toggle_query(Operand::tag("work"), Join::And);
    assert_eq!(s.search().text(), "plan");
    // Kept across runs.
    let kept = s.view_state();
    let mut again = state(&src, NavigatorSettings::default());
    again.restore(&kept);
    assert_eq!(again.search().text(), "plan");
    assert!(again.search_open);
    // Closing clears it.
    again.toggle_search();
    assert_eq!(again.search().text(), "");
}

#[test]
fn the_vault_wide_search_lists_every_folder() {
    let src = vault();
    let mut s = state(&src, NavigatorSettings::default());
    s.select_folder("Work");
    s.search_vault();
    assert_eq!(s.target(), NavTarget::Folder(String::new()));
    assert!(s.settings().include_descendants);
    assert!(s.search_open);
    s.set_search("#work");
    assert_eq!(listed(&s).len(), 2);
}

#[test]
fn the_full_text_provider_takes_the_words_to_the_source() {
    let src = vault();
    let mut s = state(
        &src,
        NavigatorSettings {
            include_descendants: true,
            search_provider: SearchProvider::FullText,
            ..NavigatorSettings::default()
        },
    );
    s.set_search("tomato");
    assert!(s.full_text_due());
    let hits: BTreeSet<String> = src
        .search_text(&s.full_text_words())
        .unwrap()
        .into_iter()
        .collect();
    s.set_full_text(Some(hits));
    assert!(!s.full_text_due());
    assert_eq!(listed(&s), ["Root note.md"]);
    // The rest of the search still narrows it.
    s.set_search("tomato #work");
    let hits = src.search_text(&s.full_text_words()).unwrap();
    s.set_full_text(Some(hits.into_iter().collect()));
    assert!(listed(&s).is_empty());
}

#[test]
fn dates_come_from_frontmatter_when_the_settings_say_so() {
    let src = MemorySource::new("fm")
        .with_file(
            NoteFile::new("Old.md", NOW, NOW)
                .property("created", text("2025-03-14"))
                .property("updated", text("2025-04-01T09:00:00")),
        )
        .with_file(NoteFile::new("New.md", NOW, NOW));
    let mut s = state(
        &src,
        NavigatorSettings {
            use_frontmatter: true,
            frontmatter_created_field: "created".to_owned(),
            frontmatter_modified_field: "updated".to_owned(),
            ..NavigatorSettings::default()
        },
    );
    s.set_search("@c:2025-03");
    assert_eq!(listed(&s), ["Old.md"]);
    s.set_search("@m:2025-04-01");
    assert_eq!(listed(&s), ["Old.md"]);
    s.set_search("@today");
    assert_eq!(listed(&s), ["New.md"]);
}
