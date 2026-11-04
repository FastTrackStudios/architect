//! The toolbars on the Action List (feature `host`): what a visual
//! editor, a script or a model reads and writes them with. Put on the
//! list by [`Extensions::install_contribution_actions`].
//!
//! | action | what it does |
//! |---|---|
//! | `TOOLBAR_LIST` | every toolbar in force (its key, owner, where it comes from, place, title, file), the folder of a person's files, the armed action |
//! | `TOOLBAR_GET` | toolbar `{id}` (its key) whole, as its file has it |
//! | `TOOLBAR_SAVE` | writes `{id, toolbar}` (JSON or TOML text) as the person's file: their copy of an extension's, or their own |
//! | `TOOLBAR_RESET` | takes away the person's file for `{id}`: an extension's comes back |
//! | `TOOLBAR_ARM` / `TOOLBAR_DISARM` | arms `{action}` (its buttons show it armed), or disarms |
//! | `TOOLBAR_EDIT` | the editor on or off (`{on}`; a toggle) |
//! | `TOOLBAR_SELECT` | shows the settings of `{id, at}` (an item) or `{id}` (the toolbar), or none |
//! | `TOOLBAR_NEW` | a new toolbar of the person's own, empty: `{title, where, id}` |
//! | `TOOLBAR_RENAME` / `TOOLBAR_DUPLICATE` / `TOOLBAR_DELETE` | renames, copies (as the person's own), deletes `{id}` |
//! | `TOOLBAR_SHOW` | shows `{id}` at `{where}`, whichever showed there hidden: which toolbar a place shows |
//! | `TOOLBAR_INSERT` | puts `{item}` in `{id}` at `{at}` (the end when none) |
//! | `TOOLBAR_REMOVE` | takes away `{id}`'s item `{at}` |
//! | `TOOLBAR_MOVE` | moves `{id}`'s item `{from}` to `{to}`, of toolbar `{into}` when given |
//! | `TOOLBAR_SET` | sets fields of `{id}`'s item `{at}`, or (no `at`) of the toolbar: `{set: {…}}` |
//! | `TOOLBAR_GROUP` / `TOOLBAR_UNGROUP` | `{count}` items from `{at}` as one group, or a group back into its buttons |
//! | `TOOLBAR_BACKUPS` | the backups of `{id}`'s file, the newest first |
//!
//! Each edit is written at once (the editor's own wait [`DEBOUNCE`] for
//! the next). An edit run from a toolbar item's context menu (target
//! `toolbar-item`, id `<key>#<at>`) needs no arguments: it edits that item.
//!
//! [`DEBOUNCE`]: crate::toolbars::DEBOUNCE

use std::sync::Arc;

use architect::action::{
    ActionCaller, ActionError, ActionFuture, ActionInfo, ActionList, ActionMeta, Effect, RawJson,
};
use facet::Facet;
use serde_json::{Value, json};

use crate::extensions::Extensions;
use crate::host::Host;
use crate::toolbar::ToolbarDecl;
use crate::toolbar_edit::{self as edit, At};
use crate::toolbars::Toolbar;

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Toolbars",
        method_name: id,
        display_name: name,
        description,
        category: "Toolbars",
        group: "Toolbars",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static LIST: ActionMeta = meta(
    "TOOLBAR_LIST",
    "Toolbars",
    "Every toolbar in force: its key (`<extension>/<id>`, `user/<id>`), its owner, where it comes \
     from (`extension`, `override`: a person's copy in its place, `person`: their own), where it \
     is (`dock:right`, `float`, `window`, `pane`, `view:<kind>`), its title and file; the folder \
     a person's toolbar files are in, and the action armed now.",
);
static GET: ActionMeta = meta(
    "TOOLBAR_GET",
    "Toolbar: read",
    "Toolbar `id` (its key) whole, as its file has it (`toolbar`: id, title, where, items with \
     their kind, action, label, icon, colors, right_click, menu, items, view, width, newline), \
     and where it comes from.",
);
static SAVE: ActionMeta = meta(
    "TOOLBAR_SAVE",
    "Toolbar: save",
    "Writes `toolbar` (JSON or TOML text of one toolbar) as the person's file under key `id`: \
     an extension's key makes it their copy, used in its place; another id a toolbar of their \
     own. Checked first (a place there is, each item whole); shown at once. Answers its file.",
);
static RESET: ActionMeta = meta(
    "TOOLBAR_RESET",
    "Toolbar: reset",
    "Takes away the person's file for toolbar `id`: their copy of an extension's goes (the \
     extension's comes back), their own toolbar goes. Answers whether there was one.",
);
static ARM: ActionMeta = meta(
    "TOOLBAR_ARM",
    "Arm an action",
    "Arms action `action` (one at a time): its toolbar buttons show it armed until it is \
     disarmed. What an armed action does next is the app's.",
);
static DISARM: ActionMeta = meta(
    "TOOLBAR_DISARM",
    "Disarm",
    "Disarms the armed action. Answers which it was.",
);

static EDIT: ActionMeta = ActionMeta {
    toggleable: true,
    ..meta(
        "TOOLBAR_EDIT",
        "Edit toolbars",
        "Turns the toolbar editor on or off (`on`; a toggle when none): + chips between items to \
         insert an action, a separator, a widget or a dropdown, items dragged to move them, a \
         press opening an item's settings.",
    )
};

/// `TOOLBAR_EDIT`'s arguments.
#[derive(Debug, Facet)]
pub struct EditArgs {
    pub on: Option<bool>,
}

/// `TOOLBAR_GET`'s and `TOOLBAR_RESET`'s arguments.
#[derive(Debug, Facet)]
pub struct KeyArgs {
    /// The toolbar's key (`fts.metronome/transport`, `user/mine`).
    pub id: String,
}

/// `TOOLBAR_SAVE`'s arguments.
#[derive(Debug, Facet)]
pub struct SaveArgs {
    pub id: String,
    /// One toolbar, as JSON or TOML text.
    pub toolbar: String,
}

/// `TOOLBAR_ARM`'s arguments.
#[derive(Debug, Facet)]
pub struct ArmArgs {
    pub action: String,
}

fn listed(t: &Toolbar) -> Value {
    json!({
        "key": t.key,
        "owner": t.owner,
        "source": t.source,
        "title": t.decl.title,
        "where": t.decl.at,
        "file": t.file.as_ref().map(|p| p.display().to_string()),
    })
}

impl Extensions {
    /// Puts `TOOLBAR_*` on the host's list.
    ///
    /// # Errors
    ///
    /// One is there already.
    pub fn install_toolbar_actions(&self) -> Result<(), ActionError> {
        let list = self.host().list().clone();
        let me = self.clone();
        list.register_with(&LIST, Effect::ReadOnly, move |_: &ActionCaller, (): ()| {
            let host = me.host();
            let bars = host.toolbars();
            let all: Vec<Value> = bars.all(host.contributions()).iter().map(listed).collect();
            Ok::<_, String>(RawJson::from(
                json!({
                    "toolbars": all,
                    "folder": bars.dir().map(|d| d.display().to_string()),
                    "armed": bars.armed(),
                })
                .to_string(),
            ))
        })?;
        let me = self.clone();
        list.register_with(
            &GET,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: KeyArgs| {
                let host = me.host();
                let t = host
                    .toolbars()
                    .get(host.contributions(), &a.id)
                    .ok_or_else(|| format!("no toolbar {:?} (TOOLBAR_LIST lists them)", a.id))?;
                let mut out = listed(&t);
                let whole = serde_json::to_value(&t.decl).map_err(|e| e.to_string())?;
                if let Some(o) = out.as_object_mut() {
                    o.insert("toolbar".to_owned(), whole);
                }
                Ok::<_, String>(RawJson::from(out.to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &SAVE,
            Effect::Mutating,
            move |_: &ActionCaller, a: SaveArgs| {
                let decl: ToolbarDecl = serde_json::from_str(&a.toolbar)
                    .map_err(|e| e.to_string())
                    .or_else(|json| {
                        ToolbarDecl::parse(&a.toolbar).map_err(|toml| {
                            format!("not a toolbar as JSON ({json}) or TOML ({toml})")
                        })
                    })?;
                let file = me.host().toolbars().save(&a.id, &decl)?;
                Ok::<_, String>(RawJson::from(
                    json!({ "file": file.display().to_string() }).to_string(),
                ))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &RESET,
            Effect::Destructive,
            move |_: &ActionCaller, a: KeyArgs| {
                let gone = me.host().toolbars().reset(&a.id)?;
                Ok::<_, String>(RawJson::from(json!({ "reset": gone }).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &ARM,
            Effect::Mutating,
            move |_: &ActionCaller, a: ArmArgs| {
                if me.host().list().info(&a.action).is_none() {
                    return Err(format!("no action {}", a.action));
                }
                me.host().toolbars().arm(&a.action);
                Ok::<_, String>(RawJson::from(json!({ "armed": a.action }).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &DISARM,
            Effect::Mutating,
            move |_: &ActionCaller, (): ()| {
                let was = me.host().toolbars().disarm();
                Ok::<_, String>(RawJson::from(json!({ "disarmed": was }).to_string()))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &EDIT,
            Effect::Mutating,
            move |_: &ActionCaller, a: EditArgs| {
                let bars = me.host().toolbars().clone();
                let on = a.on.unwrap_or(!bars.editing());
                bars.set_editing(on);
                if !on {
                    bars.flush()?;
                }
                Ok::<_, String>(RawJson::from(json!({ "editing": on }).to_string()))
            },
        )?;
        let bars = self.host().toolbars().clone();
        list.set_checked("TOOLBAR_EDIT", move || bars.editing())?;
        install_edits(&list, &self.host())
    }
}

/// The toolbar and item an edit is about: its arguments' `id` and `at`
/// (`at_field` names it), or what its context menu was opened on
/// (`toolbar-item`, `<key>#<at>`; `toolbar`, `<key>`).
fn about(caller: &ActionCaller, args: &Value, at_field: &str) -> Result<(String, Option<At>), String> {
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_owned);
    let menu = caller.invocation().and_then(|i| match i.target.as_deref() {
        Some("toolbar-item") => i.id.as_deref().and_then(|id| id.split_once('#')).map(|(k, at)| (k.to_owned(), Some(at.to_owned()))),
        Some("toolbar") => i.id.clone().map(|k| (k, None)),
        _ => None,
    });
    let (key, at) = match (text("id"), menu) {
        (Some(key), _) => (key, text(at_field)),
        (None, Some((key, at))) => (key, text(at_field).or(at)),
        (None, None) => return Err("which toolbar: `id` (its key, TOOLBAR_LIST)".to_owned()),
    };
    let at = at.map(|a| At::parse(&a)).transpose()?;
    Ok((key, at))
}

fn needs(at: Option<At>, what: &str) -> Result<At, String> {
    at.ok_or_else(|| format!("which item: `{what}` (`3`, or `2.1` inside a group)"))
}

/// One edit action: its JSON in, JSON out, with who ran it.
fn edit_action(
    list: &ActionList,
    (id, name, description): (&'static str, &'static str, &'static str),
    effect: Effect,
    params: &Value,
    run: impl Fn(&ActionCaller, &Value) -> Result<Value, String> + Send + Sync + 'static,
) -> Result<(), ActionError> {
    let info = ActionInfo {
        id: id.to_owned(),
        display_name: name.to_owned(),
        description: description.to_owned(),
        category: "Toolbars".to_owned(),
        group: "Toolbars".to_owned(),
        shortcut: String::new(),
        effect,
        params_schema: Some(params.to_string()),
        result_schema: None,
        enabled: true,
        disabled_reason: None,
        checked: None,
    };
    let run = Arc::new(run);
    list.register_forwarded_with(
        &info,
        Arc::new(move |caller: &ActionCaller, args: &[u8]| {
            let args: Value = if args.iter().all(u8::is_ascii_whitespace) {
                Value::Null
            } else {
                serde_json::from_slice(args).map_err(|e| ActionError::BadArgs {
                    id: id.to_owned(),
                    message: e.to_string(),
                    expected: None,
                })?
            };
            let args = if args.is_null() { json!({}) } else { args };
            let out = run(caller, &args).map_err(|message| ActionError::Failed {
                id: id.to_owned(),
                message,
            })?;
            let out: ActionFuture = Box::pin(futures::future::ready(Ok(out.to_string().into_bytes())));
            Ok(out)
        }),
    )
}

fn object(props: Value) -> Value {
    json!({ "type": "object", "properties": props })
}

fn text(what: &str) -> Value {
    json!({ "type": "string", "description": what })
}

const KEY: &str = "the toolbar's key (`user/mine`, `fts.metronome/transport`); from a toolbar's \
                   context menu, that toolbar";
const AT: &str = "an item's place: `3`, or `2.1` (the second button of the group at 2); from an \
                  item's context menu, that item";

/// The fine-grained edits: one item, one field, at a time.
#[allow(clippy::too_many_lines)]
fn install_edits(list: &ActionList, host: &Host) -> Result<(), ActionError> {
    let h = host.clone();
    edit_action(
        list,
        (
            "TOOLBAR_SELECT",
            "Toolbar: settings",
            "Shows the settings of toolbar `id`'s item `at` (its label, icon, colours, right-click, \
             dropdown rows), or with no `at` the toolbar's own (title, place, sizes, spacing, \
             rounding, labels, colour preset); `close` true closes them.",
        ),
        Effect::Mutating,
        &object(json!({ "id": text(KEY), "at": text(AT), "close": { "type": "boolean" } })),
        move |caller, a| {
            let bars = h.toolbars();
            if a.get("close").and_then(Value::as_bool) == Some(true) {
                bars.select(None);
                return Ok(json!({ "selected": null }));
            }
            let (key, at) = about(caller, a, "at")?;
            if h.toolbars().get(h.contributions(), &key).is_none() {
                return Err(format!("no toolbar {key:?}"));
            }
            bars.set_editing(true);
            bars.select(Some((key.clone(), at.map(|a| a.to_string()))));
            Ok(json!({ "selected": { "id": key, "at": at.map(|a| a.to_string()) } }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        (
            "TOOLBAR_NEW",
            "New toolbar",
            "A new toolbar of the person's own, empty (`user/<id>`; `id` from `title` when none): \
             at `where` (`dock:right` when none; `dock:left|right|bottom`, `float`, `window`, \
             `pane`, `view:<kind>`, `hidden`). Answers its key.",
        ),
        Effect::Mutating,
        &object(json!({ "title": text("its name"), "where": text("where it goes"), "id": text("its id") })),
        move |_, a| {
            let s = |k: &str| a.get(k).and_then(Value::as_str);
            let key = h.toolbars().create(
                h.contributions(),
                s("id"),
                s("title").unwrap_or("Toolbar"),
                s("where").unwrap_or("dock:right"),
            )?;
            Ok(json!({ "key": key }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        ("TOOLBAR_RENAME", "Toolbar: rename", "Gives toolbar `id` the title `title`."),
        Effect::Mutating,
        &json!({ "type": "object", "required": ["title"], "properties": { "id": text(KEY), "title": text("its new name") } }),
        move |caller, a| {
            let (key, _) = about(caller, a, "at")?;
            let title = a
                .get("title")
                .and_then(Value::as_str)
                .filter(|t| !t.trim().is_empty())
                .ok_or("a title")?
                .trim()
                .to_owned();
            commit(&h, &key, |t| {
                t.title.clone_from(&title);
                Ok(())
            })?;
            Ok(json!({ "key": key, "title": title }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        (
            "TOOLBAR_DUPLICATE",
            "Toolbar: duplicate",
            "A copy of toolbar `id` as one of the person's own (`title`, or \"<its title> copy\"; \
             `to` its id), hidden until TOOLBAR_SHOW shows it. Answers its key.",
        ),
        Effect::Mutating,
        &object(json!({ "id": text(KEY), "to": text("the copy's id"), "title": text("the copy's name") })),
        move |caller, a| {
            let (key, _) = about(caller, a, "at")?;
            let s = |k: &str| a.get(k).and_then(Value::as_str);
            let copy = h.toolbars().duplicate(h.contributions(), &key, s("to"), s("title"))?;
            Ok(json!({ "key": copy }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        (
            "TOOLBAR_DELETE",
            "Toolbar: delete",
            "Deletes toolbar `id`, a person's own (its file kept as a backup); for an extension's, \
             the person's copy goes and the extension's comes back. An extension's own cannot be \
             deleted: TOOLBAR_SHOW another in its place.",
        ),
        Effect::Destructive,
        &object(json!({ "id": text(KEY) })),
        move |caller, a| {
            let (key, _) = about(caller, a, "at")?;
            let bars = h.toolbars();
            if bars.selected().is_some_and(|(k, _)| k == key) {
                bars.select(None);
            }
            if !bars.reset(&key)? {
                return Err(format!(
                    "{key} is an extension's, as it shipped: TOOLBAR_SHOW another in its place"
                ));
            }
            Ok(json!({ "deleted": key }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        (
            "TOOLBAR_SHOW",
            "Toolbar: show here",
            "Shows toolbar `id` at `where` (a dock, `float`, `window`, `pane`, `view:<kind>`); \
             whichever showed there is hidden: switches which toolbar a place shows. `where` \
             `hidden` hides it. Answers the ones hidden.",
        ),
        Effect::Mutating,
        &json!({ "type": "object", "required": ["where"], "properties": { "id": text(KEY), "where": text("the place") } }),
        move |caller, a| {
            let (key, _) = about(caller, a, "at")?;
            let at = a.get("where").and_then(Value::as_str).ok_or("`where`")?;
            let hidden = h.toolbars().show(h.contributions(), &key, at)?;
            h.toolbars().flush()?;
            Ok(json!({ "shown": key, "hidden": hidden }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        (
            "TOOLBAR_INSERT",
            "Toolbar: insert",
            "Puts `item` in toolbar `id` at place `at` (the end when none; `2.1` inside the group at \
             2): a button `{action, label, icon, image, colors, right_click, menu}`, \
             `{kind: separator}`, `{kind: widget, view: <extension>/<view>, width}`, a dropdown \
             `{label, menu: [...]}` or `{kind: group, items: [...]}`. Answers its place.",
        ),
        Effect::Mutating,
        &json!({ "type": "object", "required": ["item"], "properties": {
            "id": text(KEY), "at": text(AT), "item": { "type": "object", "description": "the item, as TOOLBAR_GET shows one" } } }),
        move |caller, a| {
            let (key, at) = about(caller, a, "at")?;
            let item = edit::item_from(a.get("item").ok_or("`item`")?)?;
            let at = commit(&h, &key, |t| edit::insert(t, at, item))?;
            Ok(json!({ "at": at.to_string() }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        ("TOOLBAR_REMOVE", "Toolbar: remove item", "Takes away toolbar `id`'s item `at` (a group left empty goes too)."),
        Effect::Mutating,
        &object(json!({ "id": text(KEY), "at": text(AT) })),
        move |caller, a| {
            let (key, at) = about(caller, a, "at")?;
            let at = needs(at, "at")?;
            let gone = commit(&h, &key, |t| edit::remove(t, at))?;
            Ok(json!({ "removed": serde_json::to_value(gone).unwrap_or(Value::Null) }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        (
            "TOOLBAR_MOVE",
            "Toolbar: move item",
            "Moves toolbar `id`'s item `from` to place `to` (as the toolbar is before the move; \
             `2.1` into the group at 2) of toolbar `into` (another toolbar; this one when none). \
             A whole group moves as one. Answers its new place.",
        ),
        Effect::Mutating,
        &json!({ "type": "object", "required": ["to"], "properties": {
            "id": text(KEY), "from": text(AT), "to": text("the place it goes"), "into": text("the toolbar it goes into") } }),
        move |caller, a| {
            let (key, from) = about(caller, a, "from")?;
            let from = needs(from, "from")?;
            let to = At::parse(a.get("to").and_then(Value::as_str).ok_or("`to`")?)?;
            let into = a.get("into").and_then(Value::as_str).unwrap_or(&key).to_owned();
            let bars = h.toolbars();
            let at = bars.move_between(h.contributions(), &key, from, &into, to)?;
            bars.flush()?;
            Ok(json!({ "into": into, "at": at.to_string() }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        (
            "TOOLBAR_SET",
            "Toolbar: set",
            "Sets the fields in `set` of toolbar `id`'s item `at` (label, icon, image, hide_label, \
             colors {fill, ink, on_fill, on_ink, armed_fill, armed_ink: theme tokens or CSS \
             colours}, right_click, menu, view, width, height, newline; `null` clears one), or with \
             no `at` of the toolbar itself (title, where, hide_labels, style {size, spacing, \
             rounding, preset}).",
        ),
        Effect::Mutating,
        &json!({ "type": "object", "required": ["set"], "properties": {
            "id": text(KEY), "at": text(AT), "set": { "type": "object", "description": "the fields to set" } } }),
        move |caller, a| {
            let (key, at) = about(caller, a, "at")?;
            let patch = a.get("set").ok_or("`set`")?.clone();
            commit(&h, &key, |t| match at {
                Some(at) => edit::set(t, at, &patch),
                None => edit::configure(t, &patch),
            })?;
            Ok(json!({ "key": key, "at": at.map(|a| a.to_string()) }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        (
            "TOOLBAR_GROUP",
            "Toolbar: group",
            "Draws `count` items of toolbar `id` from `at` (2 when none; buttons) as one group \
             labelled `label`.",
        ),
        Effect::Mutating,
        &object(json!({ "id": text(KEY), "at": text(AT), "count": { "type": "integer", "minimum": 1 }, "label": text("its label") })),
        move |caller, a| {
            let (key, at) = about(caller, a, "at")?;
            let at = needs(at, "at")?;
            let count = a
                .get("count")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(2);
            let label = a.get("label").and_then(Value::as_str).unwrap_or_default().to_owned();
            let at = commit(&h, &key, |t| edit::group(t, at, count, &label))?;
            Ok(json!({ "at": at.to_string() }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        ("TOOLBAR_UNGROUP", "Toolbar: ungroup", "The group at `at` of toolbar `id` back into its buttons."),
        Effect::Mutating,
        &object(json!({ "id": text(KEY), "at": text(AT) })),
        move |caller, a| {
            let (key, at) = about(caller, a, "at")?;
            let at = needs(at, "at")?;
            commit(&h, &key, |t| edit::ungroup(t, at))?;
            Ok(json!({ "key": key }))
        },
    )?;
    let h = host.clone();
    edit_action(
        list,
        (
            "TOOLBAR_BACKUPS",
            "Toolbar: backups",
            "The backups of toolbar `id`'s file, the newest first: each write keeps the one it \
             replaces, the last ten.",
        ),
        Effect::ReadOnly,
        &object(json!({ "id": text(KEY) })),
        move |caller, a| {
            let (key, _) = about(caller, a, "at")?;
            let all: Vec<String> = h
                .toolbars()
                .backups(&key)
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            Ok(json!({ "key": key, "backups": all }))
        },
    )?;
    Ok(())
}

/// [`crate::toolbars::Toolbars::edit`], written at once.
fn commit<T>(
    host: &Host,
    key: &str,
    f: impl FnOnce(&mut ToolbarDecl) -> Result<T, String>,
) -> Result<T, String> {
    let bars = host.toolbars();
    let (out, _) = bars.edit(host.contributions(), key, f)?;
    bars.flush()?;
    Ok(out)
}
