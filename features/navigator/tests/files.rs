//! File operations and templates, as Notebook Navigator's (#339): notes,
//! folders, canvases and bases made (from templates, numbered, with
//! prompts), renamed, moved, copied, duplicated, put in the trash and
//! merged; drops on folders and among the shortcuts; driven through the
//! navigator's state over a source in memory, its events taken in as the
//! view does.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use architect_navigator::list::Clock;
use architect_navigator::state::{Drag, Made, MergeSeparator, NewKind, NewNote, Reader};
use architect_navigator::template::TemplateCommand;
use architect_navigator::{
    FileOp, MemorySource, NavTarget, NavigatorSettings, NavigatorSource, NavigatorState, NoteFile,
    OpenIn, Outcome, Pane, PeriodKind, Shortcut, SourceEvent, Subscription, VaultIndex,
};
use chrono::NaiveDate;

/// 2026-11-05 14:30 UTC.
const NOW: i64 = 1_793_889_000_000;

fn vault() -> MemorySource {
    MemorySource::new("Lab")
        .with_file(NoteFile::new("Projects/Plan.md", NOW, NOW))
        .with_file(NoteFile::new("Projects/Alpha/Spec.md", NOW, NOW))
        .with_file(NoteFile::new("Projects/Alpha/Alpha.md", NOW, NOW))
        .with_file(NoteFile::new("Inbox/Idea.md", NOW, NOW))
        .with_file(NoteFile::new("Inbox/Meeting 07.md", NOW, NOW))
        .with_file(NoteFile::new("Inbox/photo.png", NOW, NOW))
        .with_file(NoteFile::new("Templates/Project.md", NOW, NOW))
        .with_file(NoteFile::new("Templates/Meeting.md", NOW, NOW))
        .with_file(NoteFile::new("Templates/Daily.md", NOW, NOW))
        .with_text(
            "Projects/Plan.md",
            "---\ntags: [plan]\n---\n# Plan\n\nSteps.\n",
        )
        .with_text("Inbox/Idea.md", "---\nx: 1\n---\nAn idea.\n")
        .with_text(
            "Templates/Project.md",
            "---\nfolder: \"{{folder}}\"\n---\n# {{title}}\n\nMade {{date}}.\n{{cursor}}",
        )
        .with_text(
            "Templates/Meeting.md",
            "# {{prompt:Client}} on {{date:YYYY-MM-DD}}\n",
        )
        .with_text(
            "Templates/Daily.md",
            "# {{date:dddd D MMMM}}\nYesterday: [[{{yesterday}}]]\n",
        )
}

/// A navigator over `src` with its texts read, and the source's events
/// kept to take in.
struct Rig {
    src: MemorySource,
    s: NavigatorState,
    events: Arc<Mutex<Vec<SourceEvent>>>,
    _sub: Subscription,
}

fn rig(settings: NavigatorSettings) -> Rig {
    let src = vault();
    let mut s = NavigatorState::new(
        VaultIndex::read(&src),
        settings,
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    let read = src.clone();
    s.set_reader(Reader::new(move |p| read.read_text(p)));
    s.set_view(Pane::Navigation, 2000.0);
    s.set_view(Pane::Files, 600.0);
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let sub = src.subscribe(Box::new(move |e| sink.lock().unwrap().push(e.clone())));
    Rig {
        src,
        s,
        events,
        _sub: sub,
    }
}

impl Rig {
    /// Writes `out` and takes in what the source said, as the view does.
    fn write(&mut self, out: Outcome) -> Option<(String, OpenIn)> {
        let Outcome::Write { ops, open } = out else {
            panic!("not a write: {out:?}");
        };
        self.src.write(&ops).unwrap();
        let events: Vec<SourceEvent> = std::mem::take(&mut *self.events.lock().unwrap());
        for e in &events {
            self.s.apply(e);
        }
        self.s.after_write(&ops);
        open
    }

    fn done(&mut self, made: Made) -> (String, Option<(usize, usize)>) {
        let Made::Done { out, cursor, .. } = made else {
            panic!("asks: {made:?}");
        };
        let (path, _) = self.write(out).unwrap();
        (path, cursor)
    }

    fn text(&self, path: &str) -> String {
        self.src.read_text(path).unwrap_or_default()
    }

    fn has(&self, path: &str) -> bool {
        self.s.index().file(path).is_some()
    }
}

fn note(folder: &str) -> NewNote {
    NewNote {
        folder: folder.to_owned(),
        ..NewNote::default()
    }
}

#[test]
fn new_notes_are_untitled_numbered_and_canvases_and_bases_have_their_text() {
    let mut r = rig(NavigatorSettings::default());
    let (first, _) = r.done(r.s.new_note(&note("Inbox")).unwrap());
    let (second, _) = r.done(r.s.new_note(&note("Inbox")).unwrap());
    assert_eq!(
        (first.as_str(), second.as_str()),
        ("Inbox/Untitled.md", "Inbox/Untitled 1.md")
    );
    let canvas = NewNote {
        kind: NewKind::Canvas,
        ..note("")
    };
    let (made, _) = r.done(r.s.new_note(&canvas).unwrap());
    assert_eq!(made, "Untitled.canvas");
    assert_eq!(r.text(&made), "{}");
    let base = NewNote {
        kind: NewKind::Base,
        ..note("")
    };
    let (made, _) = r.done(r.s.new_note(&base).unwrap());
    assert!(r.text(&made).starts_with("views:"));
    // A name typed: checked, its extension not doubled, not taken.
    let named = NewNote {
        name: Some("Ideas.md".to_owned()),
        ..note("Inbox")
    };
    let (made, _) = r.done(r.s.new_note(&named).unwrap());
    assert_eq!(made, "Inbox/Ideas.md");
    assert!(r.s.new_note(&named).is_err(), "taken");
    let bad = NewNote {
        name: Some("a:b".to_owned()),
        ..note("Inbox")
    };
    assert!(r.s.new_note(&bad).is_err());
    assert!(r.s.new_note(&note("Nowhere")).is_err());
}

#[test]
fn a_folder_template_is_taken_below_it_and_its_tokens_written() {
    let mut r = rig(NavigatorSettings::default());
    r.s.edit_templates(|t| t.set_folder("Projects", "Templates/Project.md", true));
    let (p, cursor) = r.done(r.s.new_note(&note("Projects/Alpha")).unwrap());
    assert_eq!(p, "Projects/Alpha/Untitled.md");
    assert_eq!(
        r.text(&p),
        "---\nfolder: \"Alpha\"\n---\n# Untitled\n\nMade Nov 5, 2026.\n"
    );
    assert_eq!(cursor, Some((6, 0)));
    // Not outside it; and `no_template` makes an empty note.
    let (q, _) = r.done(r.s.new_note(&note("Inbox")).unwrap());
    assert_eq!(r.text(&q), "");
    let plain = NewNote {
        no_template: true,
        ..note("Projects")
    };
    let (e, _) = r.done(r.s.new_note(&plain).unwrap());
    assert_eq!(r.text(&e), "");
    // The template's path follows it moved.
    let out =
        r.s.move_files(&["Templates/Project.md".to_owned()], "Inbox")
            .unwrap();
    r.write(out);
    assert_eq!(
        r.s.template_for("Projects", None).as_deref(),
        Some("Inbox/Project.md")
    );
}

#[test]
fn prompts_are_asked_first_and_a_command_numbers_its_notes() {
    let mut r = rig(NavigatorSettings::default());
    r.s.edit_templates(|t| {
        t.commands.push(TemplateCommand {
            id: "m1".to_owned(),
            name: "Meeting".to_owned(),
            template: "Templates/Meeting.md".to_owned(),
            name_format: "Meeting {{number:00}}".to_owned(),
            location: architect_navigator::template::CommandLocation::Folder,
            folder: "Inbox".to_owned(),
            icon: String::new(),
            placement: architect_navigator::template::CommandPlacement::None,
        });
    });
    let asked =
        r.s.run_template_command("Meeting", BTreeMap::new(), OpenIn::Here)
            .unwrap();
    assert_eq!(asked, Made::Ask(vec!["Client".to_owned()]));
    let answers: BTreeMap<String, String> = [("Client".to_owned(), "Acme".to_owned())].into();
    let (p, _) = r.done(
        r.s.run_template_command("m1", answers.clone(), OpenIn::Here)
            .unwrap(),
    );
    assert_eq!(p, "Inbox/Meeting 08.md", "one past the highest, Meeting 07");
    assert_eq!(r.text(&p), "# Acme on 2026-11-05\n");
    let (q, _) = r.done(
        r.s.run_template_command("1", answers, OpenIn::Here)
            .unwrap(),
    );
    assert_eq!(q, "Inbox/Meeting 09.md");
    assert!(
        r.s.run_template_command("nope", BTreeMap::new(), OpenIn::Here)
            .is_err()
    );
}

#[test]
fn a_periodic_note_is_made_from_its_template_for_its_day() {
    let mut r = rig(NavigatorSettings {
        daily_template: "Templates/Daily.md".to_owned(),
        daily_pattern: "YYYY-MM-DD".to_owned(),
        periodic_folder: "Journal".to_owned(),
        ..NavigatorSettings::default()
    });
    let day = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
    let out =
        r.s.open_periodic(PeriodKind::Day, day, OpenIn::Here)
            .unwrap();
    let (p, _) = r.write(out).unwrap();
    assert_eq!(p, "Journal/2026-11-03.md");
    assert_eq!(
        r.text(&p),
        "# Tuesday 3 November\nYesterday: [[2026-11-02]]\n",
        "{{date}} and its neighbours in the file name's pattern, for its day"
    );
}

#[test]
fn renames_keep_the_extension_and_folders_take_their_folder_note() {
    let mut r = rig(NavigatorSettings {
        folder_notes: true,
        ..NavigatorSettings::default()
    });
    let out = r.s.rename_file("Inbox/photo.png", "Sunset").unwrap();
    r.write(out);
    assert!(r.has("Inbox/Sunset.png"));
    let out = r.s.rename_file("Inbox/Idea.md", "Idea #2.md").unwrap();
    r.write(out);
    assert!(r.has("Inbox/Idea #2.md"), "a warning, not an error");
    assert!(r.s.rename_file("Inbox/Sunset.png", "a/b").is_err());
    assert!(
        r.s.rename_file("Inbox/Sunset.png", "Meeting 07.md").is_ok(),
        "png beside md"
    );
    assert!(
        r.s.rename_file("Inbox/Idea #2.md", "meeting 07").is_err(),
        "taken, any case"
    );
    // The folder, its folder note with it, and what was open and selected.
    r.s.expand_all();
    r.s.select_folder("Projects/Alpha");
    let out = r.s.rename_folder("Projects/Alpha", "Beta").unwrap();
    r.write(out);
    assert!(r.has("Projects/Beta/Beta.md"));
    assert!(r.has("Projects/Beta/Spec.md"));
    assert_eq!(r.s.target(), NavTarget::Folder("Projects/Beta".to_owned()));
    assert!(r.s.rename_folder("", "X").is_err());
    assert!(
        r.s.rename_folder("Projects/Beta", "Inbox").is_ok(),
        "a sibling's name elsewhere"
    );
    assert!(r.s.rename_folder("Projects", "Inbox").is_err(), "taken");
}

#[test]
fn files_move_copy_and_duplicate_and_folders_move_by_dropping() {
    let mut r = rig(NavigatorSettings::default());
    let notes = vec!["Inbox/Idea.md".to_owned(), "Projects/Plan.md".to_owned()];
    let out =
        r.s.drop_on(
            &Drag::Notes(notes.clone()),
            &NavTarget::Folder("Projects/Alpha".to_owned()),
            true,
        )
        .unwrap();
    r.write(out);
    assert!(
        r.has("Projects/Alpha/Idea.md") && r.has("Inbox/Idea.md"),
        "copied"
    );
    assert_eq!(
        r.text("Projects/Alpha/Idea.md"),
        "---\nx: 1\n---\nAn idea.\n"
    );
    // Copied again: numbered.
    let out = r.s.copy_files(&notes[..1], "Projects/Alpha").unwrap();
    r.write(out);
    assert!(r.has("Projects/Alpha/Idea 1.md"));
    // Moved: those in the folder already are left.
    let out =
        r.s.drop_on(
            &Drag::Notes(notes),
            &NavTarget::Folder("Inbox".to_owned()),
            false,
        )
        .unwrap();
    r.write(out);
    assert!(r.has("Inbox/Plan.md") && !r.has("Projects/Plan.md"));
    assert!(
        r.s.move_files(&["Inbox/Plan.md".to_owned()], "Inbox")
            .is_err()
    );
    // Duplicated beside itself, and opened.
    let out = r.s.duplicate("Inbox/Plan.md").unwrap();
    let open = r.write(out);
    assert_eq!(open, Some(("Inbox/Plan 1.md".to_owned(), OpenIn::Here)));
    let out = r.s.duplicate_folder("Inbox").unwrap();
    r.write(out);
    assert!(r.has("Inbox 1/Plan 1.md"));
    // A folder dropped on a folder; never into itself.
    let alpha = Drag::Folder("Projects/Alpha".to_owned());
    assert!(
        r.s.drop_on(
            &alpha,
            &NavTarget::Folder("Projects/Alpha".to_owned()),
            false
        )
        .is_err()
    );
    let out =
        r.s.drop_on(&alpha, &NavTarget::Folder("Inbox".to_owned()), false)
            .unwrap();
    r.write(out);
    assert!(r.has("Inbox/Alpha/Spec.md") && !r.has("Projects/Alpha/Spec.md"));
    assert!(
        r.s.drop_on(
            &Drag::Folder("Inbox".to_owned()),
            &NavTarget::Tag("x".to_owned()),
            false
        )
        .is_err()
    );
}

#[test]
fn the_trash_takes_files_and_folders_once_and_merging_joins_notes() {
    let mut r = rig(NavigatorSettings::default());
    let out =
        r.s.trash(&[
            "Projects".to_owned(),
            "Projects/Plan.md".to_owned(),
            "Inbox/photo.png".to_owned(),
        ])
        .unwrap();
    let Outcome::Write { ops, .. } = &out else {
        panic!()
    };
    assert_eq!(
        ops,
        &[
            FileOp::Trash("Projects".to_owned()),
            FileOp::Trash("Inbox/photo.png".to_owned())
        ],
        "what is inside a folder going goes with it"
    );
    assert!(
        r.s.trash_question(&["Projects".to_owned()])
            .contains("3 files")
    );
    r.write(out);
    assert!(!r.has("Projects/Plan.md") && !r.s.index().has_folder("Projects"));
    assert_eq!(r.src.trashed(), ["Projects", "Inbox/photo.png"]);
    assert!(r.s.trash(&[String::new()]).is_err(), "not the root");

    let mut r = rig(NavigatorSettings::default());
    let paths = vec!["Projects/Plan.md".to_owned(), "Inbox/Idea.md".to_owned()];
    assert!(
        r.s.merge(&paths[..1], "M", None, MergeSeparator::BlankLine, false)
            .is_err()
    );
    let out =
        r.s.merge(&paths, "Merged", None, MergeSeparator::Heading, true)
            .unwrap();
    let open = r.write(out);
    assert_eq!(open, Some(("Projects/Merged.md".to_owned(), OpenIn::Here)));
    assert_eq!(
        r.text("Projects/Merged.md"),
        "---\ntags: [plan]\n---\n# Plan\n\nSteps.\n\n# Idea\n\nAn idea.\n"
    );
    assert!(
        !r.has("Projects/Plan.md") && !r.has("Inbox/Idea.md"),
        "to the trash"
    );
}

#[test]
fn new_folders_open_all_and_drops_among_the_shortcuts() {
    let mut r = rig(NavigatorSettings {
        include_descendants: true,
        ..NavigatorSettings::default()
    });
    let out = r.s.new_folder("Projects", "Gamma").unwrap();
    r.write(out);
    assert!(r.s.index().has_folder("Projects/Gamma"));
    assert!(
        r.s.new_folder("Projects", "gamma").is_err(),
        "taken, any case"
    );
    assert!(r.s.new_folder("Projects", ".hidden").is_err());
    r.s.select_folder("Projects");
    assert_eq!(r.s.open_all().unwrap().len(), 3);
    // Shortcuts: a note dropped among them is added there; one dragged
    // moves.
    r.s.add_shortcut(Shortcut::note("Inbox/Idea.md"), None);
    r.s.add_shortcut(Shortcut::to(&NavTarget::Folder("Inbox".to_owned())), None);
    assert!(r.s.drop_on_shortcuts(&Drag::Notes(vec!["Projects/Plan.md".to_owned()]), 1));
    let order = |s: &NavigatorState| -> Vec<String> {
        s.profile()
            .shortcuts
            .iter()
            .map(|x| x.target.clone())
            .collect()
    };
    assert_eq!(order(&r.s), ["Inbox/Idea.md", "Projects/Plan.md", "Inbox"]);
    assert!(r.s.drop_on_shortcuts(&Drag::Shortcut(2), 0));
    assert_eq!(order(&r.s), ["Inbox", "Inbox/Idea.md", "Projects/Plan.md"]);
    assert!(!r.s.drop_on_shortcuts(&Drag::Shortcut(1), 1), "where it is");
    // A folder held under a drag opens.
    r.s.collapse_all();
    assert!(r.s.spring_open(&architect_navigator::NavTarget::Folder("Projects".into())));
    assert!(
        !r.s.spring_open(&architect_navigator::NavTarget::Folder("Projects".into())),
        "open already"
    );
}
