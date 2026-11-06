//! The engine: sources asked together or by prefix, each ranked its own
//! way, merged by score.

use architect_palette::{
    Engine, FnSource, Item, OpenIn, OpenItem, Opener, Ranking, Recents, Source,
};

fn commands() -> FnSource {
    FnSource::new("commands", "Commands", |_| {
        vec![
            Item::new("WORKSPACE_CLOSE_TAB", "Close tab").group("Tabs"),
            Item::new("WORKSPACE_SPLIT_RIGHT", "Split right").group("Panes"),
            Item::new("THEME_PICK", "Choose a theme…")
                .group("Theme")
                .disabled("no themes"),
        ]
    })
    .ranking(Ranking::Commands)
}

fn notes() -> Opener {
    Opener::new(
        "note",
        "Notes",
        |_| {
            vec![
                OpenItem::new("note", "a.md", "Close reading").recency(2),
                OpenItem::new("note", "b.md", "Split ends").recency(5),
            ]
        },
        |_, _: OpenIn| {},
    )
}

#[test]
fn every_source_is_asked_and_merged_by_score() {
    let mut engine = Engine::new();
    engine.register(commands());
    engine.register(notes());
    let rows = engine.search("close", &Recents::default());
    let ids: Vec<&str> = rows.iter().map(|r| r.item.id.as_str()).collect();
    assert_eq!(ids, ["WORKSPACE_CLOSE_TAB", "a.md"]);
    // A source's id is stamped on its rows.
    assert_eq!(rows[0].item.source, "commands");
    assert_eq!(rows[1].item.source, "note");
    assert_eq!(rows[1].positions, [0, 1, 2, 3, 4]);
}

#[test]
fn a_prefix_narrows_to_one_source() {
    let mut engine = Engine::new();
    engine.register(commands());
    engine.register(notes());
    assert_eq!(engine.narrow("note: spl"), (Some("note"), "spl"));
    assert_eq!(engine.narrow("nope: spl"), (None, "nope: spl"));
    let rows = engine.search("note: spl", &Recents::default());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].item.id, "b.md");
    // Nothing typed: the source's own order (the most recent first).
    let rows = engine.search("note:", &Recents::default());
    let ids: Vec<&str> = rows.iter().map(|r| r.item.id.as_str()).collect();
    assert_eq!(ids, ["b.md", "a.md"]);
}

#[test]
fn disabled_rows_come_last_and_recents_lead_an_empty_query() {
    let mut engine = Engine::new();
    engine.register(commands());
    let mut recents = Recents::default();
    recents.touch("WORKSPACE_SPLIT_RIGHT");
    let rows = engine.search("", &recents);
    let ids: Vec<&str> = rows.iter().map(|r| r.item.id.as_str()).collect();
    assert_eq!(ids, ["WORKSPACE_SPLIT_RIGHT", "WORKSPACE_CLOSE_TAB", "THEME_PICK"]);
    let rows = engine.search("t", &recents);
    assert_eq!(rows.last().map(|r| r.item.id.as_str()), Some("THEME_PICK"));
}

#[test]
fn a_source_of_the_same_id_replaces_the_old_one_in_place() {
    let mut engine = Engine::new();
    engine.register(commands());
    engine.register(notes());
    engine.register(FnSource::new("commands", "Mine", |_| Vec::new()));
    assert_eq!(engine.ids(), ["commands", "note"]);
    assert_eq!(engine.title("commands"), Some("Mine"));
    assert!(engine.remove("note"));
    assert!(!engine.remove("note"));
    assert_eq!(notes().title(), "Notes");
}
