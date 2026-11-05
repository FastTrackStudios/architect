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

use architect::action::{ActionCaller, ActionError, ActionMeta, Effect, RawJson};
use facet::Facet;
use serde_json::{Value, json};

use crate::extensions::Extensions;
use crate::toolbar::ToolbarDecl;
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
        Ok(())
    }
}
