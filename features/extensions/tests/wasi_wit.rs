//! The `architect:extension` world (`wit/extension.wit`) is hand-written
//! beside what it mirrors; this keeps them from drifting apart:
//!
//! - its `types` are the remote-view wire's (`wire::Edit`, `Value`,
//!   `ViewEvent`, …): case for case, field for field, as their facet
//!   shapes say;
//! - its `host` interface covers the host service: each typed function an
//!   op of `hostcall::HostRequest`, and `call` for all of them;
//! - every method of the process protocol (`proto::ProcessExtension`'s
//!   vox descriptor) has its place in the world, or is named here as not
//!   yet one of a component's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;

use architect_extensions::hostcall::HostRequest;
use architect_extensions::wire;
use facet::{Facet, Shape, Type, UserType};
use wit_parser::{Resolve, TypeDefKind, TypeId};

fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c == '_' {
            out.push('-');
        } else if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

struct World {
    resolve: Resolve,
    types: BTreeMap<String, TypeId>,
}

fn world() -> World {
    let mut resolve = Resolve::new();
    let (pkg, _) = resolve
        .push_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("wit"))
        .unwrap();
    let iface = *resolve
        .packages
        .get(pkg)
        .expect("the package just pushed")
        .interfaces
        .get("types")
        .expect("a types interface");
    let types = resolve
        .interfaces
        .get(iface)
        .expect("the types interface")
        .types
        .iter()
        .map(|(n, t)| (n.clone(), *t))
        .collect();
    World { resolve, types }
}

impl World {
    fn kind(&self, name: &str) -> &TypeDefKind {
        let id = self
            .types
            .get(name)
            .unwrap_or_else(|| panic!("the world has no type {name}"));
        &self
            .resolve
            .types
            .get(*id)
            .expect("a type of the world")
            .kind
    }

    fn fields_of(&self, ty: &wit_parser::Type) -> Option<Vec<String>> {
        let wit_parser::Type::Id(id) = ty else {
            return None;
        };
        match &self
            .resolve
            .types
            .get(*id)
            .expect("a type of the world")
            .kind
        {
            TypeDefKind::Record(r) => Some(r.fields.iter().map(|f| f.name.clone()).collect()),
            _ => None,
        }
    }
}

fn field_names(fields: &[facet::Field]) -> Vec<String> {
    fields.iter().map(|f| kebab(f.name)).collect()
}

/// `shape`, an enum, has the variant `name` of the world, case for case:
/// no fields is no payload, one is the payload, more are a record's.
fn same_variant(w: &World, shape: &Shape, name: &str) {
    let Type::User(UserType::Enum(e)) = shape.ty else {
        panic!("{name}: not an enum on the wire");
    };
    let TypeDefKind::Variant(v) = w.kind(name) else {
        panic!("{name}: not a variant in the world");
    };
    let wire: Vec<String> = e.variants.iter().map(|v| kebab(v.name)).collect();
    let world: Vec<String> = v.cases.iter().map(|c| c.name.clone()).collect();
    assert_eq!(wire, world, "{name}'s cases");
    for (wv, case) in e.variants.iter().zip(&v.cases) {
        let fields = wv.data.fields;
        match (fields.len(), &case.ty) {
            (0, None) | (1, Some(_)) => {}
            (_, Some(ty)) if fields.len() > 1 => {
                assert_eq!(
                    w.fields_of(ty),
                    Some(field_names(fields)),
                    "{name}.{}'s fields",
                    case.name
                );
            }
            (n, ty) => panic!(
                "{name}.{}: {n} fields on the wire, {ty:?} in the world",
                case.name
            ),
        }
    }
}

fn same_record(w: &World, shape: &Shape, name: &str) {
    let Type::User(UserType::Struct(s)) = shape.ty else {
        panic!("{name}: not a struct on the wire");
    };
    let TypeDefKind::Record(r) = w.kind(name) else {
        panic!("{name}: not a record in the world");
    };
    let world: Vec<String> = r.fields.iter().map(|f| f.name.clone()).collect();
    assert_eq!(field_names(s.fields), world, "{name}'s fields");
}

#[test]
fn the_worlds_types_are_the_wires() {
    let w = world();
    same_variant(&w, wire::Edit::SHAPE, "edit");
    same_variant(&w, wire::Value::SHAPE, "value");
    same_variant(&w, wire::EventData::SHAPE, "event-data");
    same_record(&w, wire::ViewEvent::SHAPE, "view-event");
    same_record(&w, wire::Pointer::SHAPE, "pointer");
    same_record(&w, wire::PointerData::SHAPE, "pointer-data");
    same_record(&w, wire::TouchPoint::SHAPE, "touch-point");
    same_record(&w, wire::TouchData::SHAPE, "touch-data");
    same_record(&w, wire::WheelData::SHAPE, "wheel-data");
    same_record(&w, wire::ScrollData::SHAPE, "scroll-data");
    same_record(&w, wire::KeyData::SHAPE, "key-data");
    same_record(&w, wire::Mods::SHAPE, "mods");
    same_record(&w, wire::ThemeData::SHAPE, "theme-data");
    same_record(&w, wire::Token::SHAPE, "token");
}

fn functions(w: &World, interface: &str) -> Vec<String> {
    let pkg = w.resolve.packages.iter().next().unwrap().0;
    let id = *w
        .resolve
        .packages
        .get(pkg)
        .expect("the package")
        .interfaces
        .get(interface)
        .expect("the interface");
    w.resolve
        .interfaces
        .get(id)
        .expect("the interface")
        .functions
        .keys()
        .cloned()
        .collect()
}

#[test]
fn the_host_interface_is_the_host_service() {
    let w = world();
    let op = |json: &str| serde_json::from_str::<HostRequest>(json).unwrap().op();
    // Each typed import, the op it sends.
    let typed = [
        ("run-action", Some(op(r#"{"op":"run","action":"X"}"#))),
        ("setting", Some(op(r#"{"op":"setting","key":"k"}"#))),
        (
            "set-setting",
            Some(op(r#"{"op":"set_setting","key":"k","value":1}"#)),
        ),
        ("log", Some(op(r#"{"op":"log","text":"t"}"#))),
        (
            "call-service",
            Some(op(r#"{"op":"service","service":"a.b","method":"m"}"#)),
        ),
        ("notify", Some(op(r#"{"op":"notify","text":"t"}"#))),
        ("clipboard-read", Some(op(r#"{"op":"clipboard_read"}"#))),
        (
            "clipboard-write",
            Some(op(r#"{"op":"clipboard_write","text":"t"}"#)),
        ),
        ("pick", Some(op(r#"{"op":"pick"}"#))),
        ("prompt", Some(op(r#"{"op":"prompt"}"#))),
        ("menu", Some(op(r#"{"op":"menu"}"#))),
        (
            "decorate",
            Some(op(r#"{"op":"decorate","target":{"kind":"file","id":"a"}}"#)),
        ),
        (
            "subscribe",
            Some(op(r#"{"op":"subscribe","on":"doc.saved","action":"X"}"#)),
        ),
        ("unsubscribe", Some(op(r#"{"op":"unsubscribe","key":"k"}"#))),
        ("list-documents", Some(op(r#"{"op":"list_documents"}"#))),
        (
            "read-document",
            Some(op(r#"{"op":"read_document","path":"a"}"#)),
        ),
        (
            "create-document",
            Some(op(r#"{"op":"create_document","path":"a"}"#)),
        ),
        (
            "edit-document",
            Some(op(r#"{"op":"edit_document","path":"a","edits":[]}"#)),
        ),
        (
            "rename-document",
            Some(op(r#"{"op":"rename_document","path":"a","to":"b"}"#)),
        ),
        (
            "move-document",
            Some(op(r#"{"op":"move_document","path":"a","folder":"b"}"#)),
        ),
        (
            "delete-document",
            Some(op(r#"{"op":"delete_document","path":"a"}"#)),
        ),
        (
            "watch-documents",
            Some(op(r#"{"op":"watch_documents","action":"X"}"#)),
        ),
        ("editor-state", Some(op(r#"{"op":"editor_state"}"#))),
        (
            "edit-editor",
            Some(op(r#"{"op":"edit_editor","changes":[]}"#)),
        ),
        (
            "decorate-editor",
            Some(op(r#"{"op":"decorate_editor","decorations":[]}"#)),
        ),
        // The theme is the host's to send, no request (a process is sent it).
        (
            "fetch",
            Some(op(r#"{"op":"fetch","url":"https://a.example"}"#)),
        ),
        ("read-body", Some(op(r#"{"op":"read_body","stream":"s1"}"#))),
        (
            "close-body",
            Some(op(r#"{"op":"close_body","stream":"s1"}"#)),
        ),
        ("secret-set", Some(op(r#"{"op":"secret","name":"k"}"#))),
        ("ask-secret", Some(op(r#"{"op":"ask_secret","name":"k"}"#))),
        ("ports", Some(op(r#"{"op":"ports"}"#))),
        (
            "open-port",
            Some(op(r#"{"op":"open_port","kind":"midi","name":"a"}"#)),
        ),
        (
            "port-send",
            Some(op(r#"{"op":"port_send","port":"p","message":{}}"#)),
        ),
        (
            "port-receive",
            Some(op(r#"{"op":"port_receive","port":"p"}"#)),
        ),
        ("close-port", Some(op(r#"{"op":"close_port","port":"p"}"#))),
        ("theme", None),
        // The host's clock: a process reads its own.
        ("now", None),
        // Any request at all.
        ("call", None),
    ];
    let mut expected: Vec<String> = typed.iter().map(|(f, _)| (*f).to_owned()).collect();
    expected.sort();
    let mut got = functions(&w, "host");
    got.sort();
    assert_eq!(got, expected, "the host interface's functions");
    assert_eq!(
        typed.iter().filter_map(|(_, op)| *op).collect::<Vec<_>>(),
        [
            "run",
            "setting",
            "set_setting",
            "log",
            "service",
            "notify",
            "clipboard_read",
            "clipboard_write",
            "pick",
            "prompt",
            "menu",
            "decorate",
            "subscribe",
            "unsubscribe",
            "list_documents",
            "read_document",
            "create_document",
            "edit_document",
            "rename_document",
            "move_document",
            "delete_document",
            "watch_documents",
            "editor_state",
            "edit_editor",
            "decorate_editor",
            "fetch",
            "read_body",
            "close_body",
            "secret",
            "ask_secret",
            "ports",
            "open_port",
            "port_send",
            "port_receive",
            "close_port"
        ]
    );
}

#[test]
fn every_process_method_has_its_place_in_the_world() {
    let w = world();
    let guest = functions(&w, "guest");
    let host = functions(&w, "host");
    // A process's method, and where a component does the same.
    let place: BTreeMap<&str, &[&str]> = BTreeMap::from([
        ("hello", &["init", "actions", "views"][..]),
        ("run", &["invoke"][..]),
        ("run_in", &["invoke"][..]),
        ("event", &["on-event"][..]),
        ("close", &["close"][..]),
        ("view", &["render", "pending"][..]),
        // Asked for, not sent: the `theme` import.
        ("theme", &[][..]),
        // What it adds at run time: not yet a component's (#167).
        ("updates", &[][..]),
    ]);
    let descriptors = [
        architect_extensions::proto::process_extension_service_descriptor(),
        architect_extensions::proto::process_extension_stream_service_descriptor(),
    ];
    for d in descriptors {
        for m in d.methods {
            let at = place.get(m.method_name).unwrap_or_else(|| {
                panic!(
                    "{}.{} has no place in the architect:extension world: add it to \
                     wit/extension.wit and to this table",
                    d.service_name, m.method_name
                )
            });
            for f in *at {
                assert!(guest.iter().any(|g| g == f), "the guest exports no {f}");
            }
        }
    }
    assert!(host.iter().any(|h| h == "theme"));
    assert!(guest.iter().any(|g| g == "save-state"));
}

/// A component's workers speak the API a Luau worker's and a process's
/// work do (`architect_scripting::worker`, `architect_extensions::work`):
/// each of their calls, and where a component makes it.
#[test]
fn the_worker_interfaces_are_the_other_runners() {
    let w = world();
    let mut worker = functions(&w, "worker");
    worker.sort();
    assert_eq!(
        worker,
        [
            "cancel",
            "cancelled",
            "post",
            "progress",
            "receive",
            "spawn"
        ],
        "the worker import's functions"
    );
    let mut work = functions(&w, "work");
    work.sort();
    assert_eq!(
        work,
        ["on-done", "on-message", "on-progress", "run"],
        "the work export's functions"
    );
    // The other runners' calls, by the name a component has for each.
    let same: [(&str, &str); 9] = [
        // A Luau owner's `worker.spawn(name, args)`, a process's `spawn_work`.
        ("spawn", "worker"),
        // `w:post(msg)` / `worker.post(msg)`, `Work::post`.
        ("post", "worker"),
        // `w:cancel()`, `WorkHandle::cancel`.
        ("cancel", "worker"),
        // `worker.progress(done, total, note)`, `Work::progress`.
        ("progress", "worker"),
        // `worker.cancelled()`, `Work::cancelled`.
        ("cancelled", "worker"),
        // `worker.receive(ms)`.
        ("receive", "worker"),
        // `worker.on_message(w, fn)`, `on_progress`, `on_done`.
        ("on-message", "work"),
        ("on-progress", "work"),
        ("on-done", "work"),
    ];
    for (f, interface) in same {
        assert!(
            functions(&w, interface).iter().any(|g| g == f),
            "{interface} has no {f}"
        );
    }
}

/// A page's worker says the same names as the others
/// (`docs/content/architecture/pages.md`): a TSX page's handle in
/// camelCase, a Luau page's in `snake_case`, as their generated types
/// write them (`architect-pages`' `types/architect.d.ts` and
/// `architect.d.luau`, which its own `tests/types.rs` keeps equal to
/// what its runtimes give).
#[test]
fn the_page_runners_say_the_same_worker_names() {
    let w = world();
    let types = Path::new(env!("CARGO_MANIFEST_DIR")).join("../pages/types");
    let ts = std::fs::read_to_string(types.join("architect.d.ts")).unwrap();
    let luau = std::fs::read_to_string(types.join("architect.d.luau")).unwrap();
    let camel = |wit: &str| {
        let mut out = String::new();
        let mut up = false;
        for c in wit.chars() {
            if c == '-' {
                up = true;
            } else if up {
                out.push(c.to_ascii_uppercase());
                up = false;
            } else {
                out.push(c);
            }
        }
        out
    };
    // What an owner calls on its worker, by the world's names: the
    // `worker` import's and the `work` export's callbacks.
    let owner: [(&str, &str); 5] = [
        ("post", "worker"),
        ("cancel", "worker"),
        ("on-message", "work"),
        ("on-progress", "work"),
        ("on-done", "work"),
    ];
    for (f, interface) in owner {
        assert!(
            functions(&w, interface).iter().any(|g| g == f),
            "{interface} has no {f}"
        );
        // A Luau page's handle: `w:on_message(fn)`.
        let snake = f.replace('-', "_");
        assert!(
            luau.contains(&format!("\t{snake}: (self: Worker")),
            "architect.d.luau's Worker has no {snake}"
        );
        // A TSX page's handle: `w.onMessage(fn)`. Its workers say no
        // progress (a QuickJS worker posts messages only).
        if f != "on-progress" {
            let name = camel(f);
            assert!(
                ts.contains(&format!("  {name}(")),
                "architect.d.ts's Worker has no {name}"
            );
        }
    }
    // Both start one with `worker.spawn(file, args)`.
    assert!(functions(&w, "worker").iter().any(|g| g == "spawn"));
    assert!(ts.contains("spawn(file: string, args?: unknown): Worker;"));
    assert!(luau.contains("worker.spawn(file, args?)"));
}
