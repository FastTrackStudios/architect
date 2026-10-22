//! The processor's behaviour, moved with it from studio's `input` crate.

use architect_input::{
    InputCommand, InputContext, InputEvent, InputProcessor, KeyChord, KeyCode, KeyEvent, KeyTrie,
    ModeDefinition, ModeId, Modifiers, WhenExpr, parse_key_sequence, which_key,
};

type Proc = InputProcessor;

const fn key_event(key: KeyCode) -> InputEvent {
    InputEvent::Key(KeyEvent {
        key,
        modifiers: Modifiers::NONE,
    })
}

fn char_event(ch: char) -> InputEvent {
    key_event(KeyCode::char(ch))
}

fn chord(ch: char) -> KeyChord {
    KeyChord::plain(KeyCode::char(ch))
}

fn mode(id: ModeId, name: &str) -> ModeDefinition {
    let tag = id.as_str().to_owned();
    ModeDefinition::new(id, name)
        .with_on_enter(vec![format!("mode.{tag}.enter")])
        .with_on_exit(vec![format!("mode.{tag}.exit")])
}

fn make_processor() -> Proc {
    let mut proc = Proc::new();
    proc.add_mode(mode(ModeId::normal(), "NORMAL"));
    proc.add_mode(mode(ModeId::insert(), "INSERT").with_passthrough_text(true));
    proc.add_mode(mode(ModeId::visual(), "VISUAL"));

    let mut normal = KeyTrie::new();
    normal.bind(chord('j'), "cursor.down".into());
    normal.bind(chord('k'), "cursor.up".into());
    normal.bind_mode_switch(chord('i'), ModeId::insert());
    normal.bind_mode_switch(chord('v'), ModeId::visual());
    normal.bind_sequence(&[chord('g'), chord('g')], "cursor.top".into());
    normal.bind_sequence(&[chord('g'), chord('e')], "cursor.end".into());
    proc.set_keymap(ModeId::normal(), normal);
    proc.set_keymap(ModeId::insert(), KeyTrie::new());
    proc
}

fn ids(commands: &[InputCommand]) -> Vec<&str> {
    commands
        .iter()
        .filter_map(|cmd| cmd.action().map(String::as_str))
        .collect()
}

fn is_pending(commands: &[InputCommand], want: &str) -> bool {
    matches!(commands.first(), Some(InputCommand::Pending { display }) if display == want)
}

#[test]
fn test_single_key_action_normal_mode() {
    let mut proc = make_processor();
    let commands = proc.process(char_event('j'), &InputContext::new());
    assert_eq!(ids(&commands), vec!["cursor.down"]);
    assert_eq!(commands.len(), 1);
    assert!(!proc.needs_timeout());
}

#[test]
fn test_sticky_prefix_rewinds_to_anchor_while_held() {
    let mut proc = make_processor();
    let ctx = InputContext::new();

    assert!(is_pending(&proc.process(char_event('g'), &ctx), "g"));

    let commands = proc.process(char_event('e'), &ctx);
    assert!(!proc.notify_key_release(&chord('e')));
    assert_eq!(ids(&commands), vec!["cursor.end"]);
    assert!(proc.needs_timeout(), "rewound to the anchor, not cleared");
    assert_eq!(proc.pending_display(), Some("g".to_owned()));
    assert_eq!(proc.pending_keys(), &[chord('g')]);

    let commands = proc.process(char_event('e'), &ctx);
    assert!(!proc.notify_key_release(&chord('e')));
    assert_eq!(ids(&commands), vec!["cursor.end"]);

    assert!(proc.notify_key_release(&chord('g')), "hide the overlay");
    assert!(!proc.needs_timeout());
    assert_eq!(proc.pending_display(), None);
}

#[test]
fn test_held_anchor_auto_repeat_is_suppressed() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    assert!(is_pending(&proc.process(char_event('g'), &ctx), "g"));

    let commands = proc.process(char_event('g'), &ctx);
    assert!(
        ids(&commands).is_empty(),
        "anchor auto-repeat must not fire `g g`"
    );
    assert_eq!(proc.pending_display(), Some("g".to_owned()));
}

#[test]
fn test_sticky_prefix_suppresses_auto_repeat() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    proc.process(char_event('g'), &ctx);
    assert_eq!(
        ids(&proc.process(char_event('e'), &ctx)),
        vec!["cursor.end"]
    );
    assert!(ids(&proc.process(char_event('e'), &ctx)).is_empty());
}

#[test]
fn test_bare_hold_release_keeps_overlay_visible() {
    let mut proc = make_processor();
    proc.process(char_event('g'), &InputContext::new());
    assert!(!proc.notify_key_release(&chord('g')));
}

#[test]
fn test_two_key_sequence_with_pending() {
    let mut proc = make_processor();
    let ctx = InputContext::new();

    let commands = proc.process(char_event('g'), &ctx);
    assert_eq!(commands.len(), 1);
    assert!(is_pending(&commands, "g"));
    assert!(proc.needs_timeout());

    // Released before the second press: a vim double-tap, not a held prefix.
    proc.notify_key_release(&chord('g'));

    assert_eq!(
        ids(&proc.process(char_event('g'), &ctx)),
        vec!["cursor.top"]
    );
    assert!(!proc.needs_timeout());
    assert_eq!(proc.pending_display(), None);
}

#[test]
fn test_mode_switch_via_keybinding() {
    let mut proc = make_processor();
    let commands = proc.process(char_event('i'), &InputContext::new());
    assert_eq!(proc.current_mode(), &ModeId::insert());
    assert!(
        matches!(commands.first(), Some(InputCommand::SwitchMode(m)) if *m == ModeId::insert())
    );
    assert_eq!(
        ids(&commands),
        vec!["mode.normal.exit", "mode.insert.enter"]
    );
}

#[test]
fn test_insert_mode_passthrough() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    proc.process(char_event('i'), &ctx);

    let commands = proc.process(char_event('h'), &ctx);
    assert_eq!(commands, vec![InputCommand::InsertText("h".into())]);
    let commands = proc.process(char_event('e'), &ctx);
    assert_eq!(commands, vec![InputCommand::InsertText("e".into())]);
}

#[test]
fn test_escape_cancels_pending_and_returns_to_normal() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    assert!(is_pending(&proc.process(char_event('g'), &ctx), "g"));

    assert!(proc.process(key_event(KeyCode::Escape), &ctx).is_empty());
    assert!(!proc.needs_timeout());
    assert_eq!(proc.current_mode(), &ModeId::normal());
}

#[test]
fn test_escape_binding_runs_when_idle() {
    let mut proc = make_processor();
    let mut normal = KeyTrie::new();
    normal.bind(KeyChord::plain(KeyCode::Escape), "selection.clear".into());
    proc.set_keymap(ModeId::normal(), normal);

    let commands = proc.process(key_event(KeyCode::Escape), &InputContext::new());
    assert_eq!(ids(&commands), vec!["selection.clear"]);
    assert_eq!(proc.current_mode(), &ModeId::normal());
}

#[test]
fn test_escape_exits_insert_to_normal() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    proc.process(char_event('i'), &ctx);

    let commands = proc.process(key_event(KeyCode::Escape), &ctx);
    assert_eq!(proc.current_mode(), &ModeId::normal());
    assert_eq!(
        ids(&commands),
        vec!["mode.insert.exit", "mode.normal.enter"]
    );
}

#[test]
fn test_new_starts_in_normal_mode() {
    let proc = Proc::new();
    assert_eq!(proc.current_mode(), &ModeId::normal());
    assert!(!proc.needs_timeout());
    assert_eq!(proc.pending_display(), None);
}

#[test]
fn test_unmatched_key_in_normal_mode() {
    let mut proc = make_processor();
    let commands = proc.process(char_event('z'), &InputContext::new());
    assert!(matches!(commands.as_slice(), [InputCommand::Unhandled(_)]));
}

#[test]
fn test_timeout_expired_clears_pending() {
    let mut proc = make_processor();
    proc.process(char_event('g'), &InputContext::new());
    assert!(proc.needs_timeout());

    let commands = proc.timeout_expired();
    assert!(!proc.needs_timeout());
    assert_eq!(commands, vec![InputCommand::Unhandled(char_event('g'))]);
    assert!(proc.timeout_expired().is_empty(), "nothing left to expire");
}

#[test]
fn test_sequence_miss_second_key() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    proc.process(char_event('g'), &ctx);
    let commands = proc.process(char_event('z'), &ctx);
    assert!(!proc.needs_timeout());
    assert!(matches!(commands.as_slice(), [InputCommand::Unhandled(_)]));
}

#[test]
fn test_non_key_events_forwarded_as_unhandled() {
    let mut proc = make_processor();
    let commands = proc.process(InputEvent::FocusGained, &InputContext::new());
    assert_eq!(
        commands,
        vec![InputCommand::Unhandled(InputEvent::FocusGained)]
    );
}

#[test]
fn test_macro_record_and_playback_replays_actions() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    for ch in ['q', 'a', 'j', 'j', 'j', 'q', '@'] {
        proc.process(char_event(ch), &ctx);
    }
    let commands = proc.process(char_event('a'), &ctx);
    assert_eq!(
        ids(&commands),
        vec!["cursor.down", "cursor.down", "cursor.down"]
    );
}

#[test]
fn a_bound_q_is_the_bindings_not_a_macro() {
    let mut proc = make_processor();
    let mut normal = KeyTrie::new();
    normal.bind(chord('q'), "quantize".into());
    normal.bind(chord('j'), "cursor.down".into());
    proc.set_keymap(ModeId::normal(), normal);
    let ctx = InputContext::new();

    assert_eq!(ids(&proc.process(char_event('q'), &ctx)), vec!["quantize"]);
    assert_eq!(
        ids(&proc.process(char_event('j'), &ctx)),
        vec!["cursor.down"]
    );
}

#[test]
fn test_macro_count_prefix_repeats_playback() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    for ch in ['q', 'a', 'j', 'q', '3', '@'] {
        proc.process(char_event(ch), &ctx);
    }
    let commands = proc.process(char_event('a'), &ctx);
    assert_eq!(
        ids(&commands),
        vec!["cursor.down", "cursor.down", "cursor.down"]
    );
}

#[test]
fn test_macro_repeat_last_register_with_double_at() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    for ch in ['q', 'a', 'j', 'q', '@'] {
        proc.process(char_event(ch), &ctx);
    }
    assert_eq!(
        ids(&proc.process(char_event('a'), &ctx)),
        vec!["cursor.down"]
    );
    proc.process(char_event('@'), &ctx);
    assert_eq!(
        ids(&proc.process(char_event('@'), &ctx)),
        vec!["cursor.down"]
    );
}

#[test]
fn test_macro_does_not_record_playback() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    for ch in ['q', 'a', 'j', 'q', 'q', 'b', '@', 'a', 'q', '@'] {
        proc.process(char_event(ch), &ctx);
    }
    assert!(proc.process(char_event('b'), &ctx).is_empty());
}

#[test]
fn a_count_prefixes_the_next_action() {
    let mut proc = make_processor();
    let ctx = InputContext::new();
    assert!(proc.process(char_event('1'), &ctx).is_empty());
    assert!(proc.process(char_event('2'), &ctx).is_empty());
    let commands = proc.process(char_event('j'), &ctx);
    let Some(InputCommand::ActionWithArgs { action, args }) = commands.first() else {
        panic!("expected a counted action, got {commands:?}");
    };
    assert_eq!(action, "cursor.down");
    assert_eq!(args.count, Some(12));
}

#[test]
fn an_operator_and_motion_compose_into_one_action() {
    let config = architect_input::KeymapConfig::from_json_str(
        r#"{ "keymap": { "normal": { "d": "operator:delete", "w": "motion:word" } } }"#,
    )
    .unwrap();
    let mut proc = Proc::from_config(config).unwrap();
    let ctx = InputContext::new();
    assert!(proc.process(char_event('d'), &ctx).is_empty());
    let commands = proc.process(char_event('w'), &ctx);
    let Some(InputCommand::ActionWithArgs { action, args }) = commands.first() else {
        panic!("expected a composed action, got {commands:?}");
    };
    assert_eq!(action, "operator.delete");
    assert_eq!(args.motion.as_deref(), Some("word"));
}

#[test]
fn which_key_lists_the_live_continuations_of_the_prefix() {
    let tree = which_key::WhichKeyTreeDef {
        prefix: "z".into(),
        label: "Zoom".into(),
        case_insensitive: None,
        entries: vec![which_key::WhichKeyEntryDef {
            key: "t".into(),
            label: "Toggle tracks".into(),
            action: Some("ZOOM_TRACKS".into()),
            children: None,
        }],
    };
    let mut proc = make_processor();
    let mut base = KeyTrie::new();
    for (seq, action) in which_key::bindings(std::slice::from_ref(&tree)) {
        base.bind_sequence(&parse_key_sequence(&seq).unwrap(), action);
    }
    proc.set_keymap(ModeId::normal(), base);
    let mut layer = KeyTrie::new();
    layer.bind_sequence(&parse_key_sequence("z e").unwrap(), "ZOOM_EDITOR".into());
    proc.add_context_keymap(ModeId::normal(), WhenExpr::parse("context:editor"), layer);
    let labels = which_key::labels(&[tree]);

    let mut ctx = InputContext::new();
    assert!(proc.which_key(&ctx, &labels).is_none(), "nothing typed yet");
    proc.process(char_event('z'), &ctx);

    let popup = proc.which_key(&ctx, &labels).unwrap();
    assert_eq!(popup.title.as_deref(), Some("Zoom"));
    let keys: Vec<String> = popup.entries.iter().map(|e| e.chord.to_string()).collect();
    assert_eq!(keys, vec!["t"], "the editor layer is not live");

    ctx.set_var("context", "editor");
    let popup = proc.which_key(&ctx, &labels).unwrap();
    let keys: Vec<String> = popup.entries.iter().map(|e| e.chord.to_string()).collect();
    assert_eq!(keys, vec!["e", "t"]);
    let editor = popup.entries.first().unwrap();
    assert_eq!(editor.action().map(String::as_str), Some("ZOOM_EDITOR"));
    assert_eq!(editor.label, None, "no label: the app shows the id");
}

/// An app's own action id type works as long as it is built from the
/// keymap's strings.
#[test]
fn a_typed_action_id_works() {
    #[derive(Debug, Clone, PartialEq)]
    enum Act {
        Save,
        Other(String),
    }
    impl From<String> for Act {
        fn from(id: String) -> Self {
            if id == "FILE_SAVE" {
                Self::Save
            } else {
                Self::Other(id)
            }
        }
    }
    let config = architect_input::KeymapConfig::from_json_str(
        r#"{ "keymap": { "normal": { "Ctrl+s": "FILE_SAVE", "x": "NOPE" } } }"#,
    )
    .unwrap();
    let mut proc: InputProcessor<Act> = InputProcessor::from_config(config).unwrap();
    let ctx = InputContext::new();
    let save = proc.process(
        InputEvent::Key(KeyEvent {
            key: KeyCode::char('s'),
            modifiers: Modifiers::CTRL,
        }),
        &ctx,
    );
    assert_eq!(save, vec![InputCommand::Action(Act::Save)]);
    let other = proc.process(char_event('x'), &ctx);
    assert_eq!(other, vec![InputCommand::Action(Act::Other("NOPE".into()))]);
}
