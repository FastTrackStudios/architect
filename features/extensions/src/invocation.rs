//! An action's invocation as the JSON an extension is handed (its
//! `invoke`'s `invocation`): the same from every host.

use architect::action::{ActionCaller, Invocation, InvocationSource};
use serde_json::Value;

/// An invocation as JSON (what `ctx` is, and what a process is sent):
/// `{"source": "app"}` for none.
#[must_use]
pub fn invocation_json(invocation: Option<&Invocation>) -> Value {
    let Some(i) = invocation else {
        return serde_json::json!({ "source": InvocationSource::App.name() });
    };
    let mut out = serde_json::Map::new();
    out.insert("source".into(), Value::from(i.source.name()));
    for (name, field) in [
        ("target", &i.target),
        ("id", &i.id),
        ("window", &i.window),
        ("pane", &i.pane),
        ("view", &i.view),
        ("selection", &i.selection),
        ("url", &i.url),
        ("path", &i.path),
        ("event", &i.event),
        ("detail", &i.detail),
    ] {
        if let Some(v) = field {
            out.insert(name.into(), Value::from(v.as_str()));
        }
    }
    Value::Object(out)
}

/// What an action is handed as its context: its invocation
/// ([`invocation_json`]) and, when an extension made the call, that
/// extension's id as `caller`, from the call's principal (the host's to
/// say, never the extension's arguments). A service checks whose call it
/// is by it (`architect.playhead`: an extension moves its own source only).
#[must_use]
pub fn caller_json(caller: &ActionCaller) -> Value {
    let mut out = invocation_json(caller.invocation());
    if let (Some(id), Value::Object(map)) =
        (caller.principal().and_then(|p| p.extension_id()), &mut out)
    {
        map.insert("caller".into(), Value::from(id));
    }
    out
}
