//! The `THEME_*` actions (feature `actions`): the store driven from
//! anywhere the Action List reaches — ⌘K, keys, MCP, the command line.
//!
//! | id | effect | does |
//! |---|---|---|
//! | `THEME_LIST` | read-only | every theme: id, label, appearance, source (`extension:<id>` for an extension's), problems |
//! | `THEME_GET` | read-only | the choice, the appearance, the theme showing, the revision, and its tokens (all, or the names and `prefix-*`es asked for) |
//! | `THEME_SET` | mutating | the pick: `one`, or `light` / `dark` halves, and/or `appearance` |
//! | `THEME_PREVIEW` | mutating | shows a theme over everything until `THEME_PREVIEW_END` |
//! | `THEME_PREVIEW_END` | mutating | ends the preview |
//! | `THEME_TOKEN_SET` | mutating | a token overridden (a literal or an expression) in a layer, the person's by default |
//! | `THEME_TOKEN_RESET` | mutating | one (or every) override of a layer removed, the person's by default |
//! | `THEME_RELOAD` | mutating | the theme files read again |
//! | `THEME_PROBLEMS` | read-only | what is wrong with the theme files, the choice and the token expressions |
//! | `THEME_INSPECT` | read-only | each token (all, or those under a prefix): its value, who set it, whether it is an expression |
//!
//! Every write answers whether anything that shows changed, and the
//! revision after it. None is destructive: each is undone by another.

// The facet derive's generated code trips the arithmetic lint.
#![allow(clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;

use architect::action::{ActionError, ActionList, ActionMeta, Effect};
use facet::Facet;

use crate::model::{Appearance, TOKEN_NAME_RULE, ThemeChoice, valid_token};
use crate::store::{LayerKind, ThemeStore};

pub mod ids {
    pub const LIST: &str = "THEME_LIST";
    pub const GET: &str = "THEME_GET";
    pub const SET: &str = "THEME_SET";
    pub const PREVIEW: &str = "THEME_PREVIEW";
    pub const PREVIEW_END: &str = "THEME_PREVIEW_END";
    pub const TOKEN_SET: &str = "THEME_TOKEN_SET";
    pub const TOKEN_RESET: &str = "THEME_TOKEN_RESET";
    pub const RELOAD: &str = "THEME_RELOAD";
    pub const PROBLEMS: &str = "THEME_PROBLEMS";
    pub const INSPECT: &str = "THEME_INSPECT";

    /// Every id [`super::register`] puts on a list.
    pub const ALL: &[&str] = &[
        LIST,
        GET,
        SET,
        PREVIEW,
        PREVIEW_END,
        TOKEN_SET,
        TOKEN_RESET,
        RELOAD,
        PROBLEMS,
        INSPECT,
    ];
}

macro_rules! metas {
    ($($name:ident = $id:ident, $display:literal, $desc:literal;)*) => {
        $(static $name: ActionMeta = ActionMeta {
            id: ids::$id,
            trait_name: "",
            method_name: "",
            display_name: $display,
            description: $desc,
            category: "Appearance",
            group: "Theme",
            shortcut: "",
            toggleable: false,
            undo: false,
        };)*
    };
}

metas! {
    LIST = LIST, "Themes", "Every theme the app knows: its id, label, appearance (light or dark), source (builtin, file, memory, or extension:<id> for one a loaded extension contributes) and problems.";
    GET = GET, "Current theme", "The choice (one theme, or a light and a dark), the appearance (system, light, dark), the theme showing, the revision, and its resolved tokens: all, or only the `tokens` named (a `prefix-*` matches every name starting `prefix-`).";
    SET = SET, "Set the theme", "Picks the theme: `one` always shows it; `light` / `dark` set that half of the pair (the other half stays). `appearance` is system, light or dark. Answers whether anything changed and the revision.";
    PREVIEW = PREVIEW, "Preview a theme", "Shows theme `id` over every other choice until THEME_PREVIEW_END; nothing is kept.";
    PREVIEW_END = PREVIEW_END, "End the theme preview", "Ends THEME_PREVIEW: the chosen theme shows again.";
    TOKEN_SET = TOKEN_SET, "Set a theme token", "Overrides token `name` (as THEME_GET names it: `primary`, `ink-3`, `radius`, an app's `arrange.item.edge`) with `value` in `layer` (app, org, person, device, workspace, pick, preview; person when left out), over every theme. A value is any CSS value, or an expression over other tokens: `$border`, `mix($border, $foreground, 0.2)`, `alpha($ink, 0.3)`.";
    TOKEN_RESET = TOKEN_RESET, "Reset a theme token", "Removes `layer`'s (the person's when left out) override of token `name`, or every one when `name` is left out.";
    RELOAD = RELOAD, "Reload themes", "Reads the theme files again. A file that no longer reads keeps its last good theme and is reported (THEME_PROBLEMS).";
    PROBLEMS = PROBLEMS, "Theme problems", "What is wrong: theme files that do not read (file, field, message), missing or looping parents, a chosen theme that is not there, token expressions that name no token or loop (drawn magenta).";
    INSPECT = INSPECT, "Inspect theme tokens", "Every token of the theme showing (or those whose name starts with `prefix`, such as `arrange.`): its value, its origin (derived, theme, layer), the layer and the theme or layer id that set it, and the expression when it is one.";
}

/// One theme, as `THEME_LIST` answers it.
#[derive(Debug, Clone, Facet)]
pub struct ThemeItem {
    pub id: String,
    pub label: String,
    /// `light` or `dark`; unset when its parent is missing.
    pub appearance: Option<String>,
    /// `builtin`, `file`, `memory`, or `extension:<id>` (the extension
    /// that contributes it).
    pub source: String,
    /// The file (or extension and file) it was read from.
    pub origin: String,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, Facet)]
pub struct ListOut {
    pub themes: Vec<ThemeItem>,
    /// The theme showing.
    pub showing: String,
}

#[derive(Debug, Clone, Default, Facet)]
pub struct GetArgs {
    /// Token names, or `prefix-*`; every token when left out.
    #[facet(default)]
    pub tokens: Option<Vec<String>>,
}

/// The choice: `one`, or `light` and `dark`.
#[derive(Debug, Clone, Facet)]
pub struct ChoiceOut {
    pub one: Option<String>,
    pub light: Option<String>,
    pub dark: Option<String>,
}

#[derive(Debug, Clone, Facet)]
pub struct GetOut {
    pub choice: ChoiceOut,
    /// `system`, `light` or `dark`.
    pub appearance: String,
    /// The OS's, as the host last said.
    pub system: String,
    /// The scheme showing.
    pub scheme: String,
    /// The theme showing.
    pub theme: String,
    /// What shows in light and in dark.
    pub light: String,
    pub dark: String,
    pub revision: u64,
    pub tokens: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Facet)]
pub struct SetArgs {
    /// One theme, whatever the appearance.
    #[facet(default)]
    pub one: Option<String>,
    /// The theme light shows.
    #[facet(default)]
    pub light: Option<String>,
    /// The theme dark shows.
    #[facet(default)]
    pub dark: Option<String>,
    #[facet(default)]
    pub appearance: Option<Appearance>,
}

#[derive(Debug, Clone, Facet)]
pub struct IdArgs {
    pub id: String,
}

#[derive(Debug, Clone, Facet)]
pub struct TokenSetArgs {
    pub name: String,
    pub value: String,
    /// The layer (`person` when left out).
    #[facet(default)]
    pub layer: Option<String>,
}

#[derive(Debug, Clone, Default, Facet)]
pub struct TokenResetArgs {
    /// Every override when left out.
    #[facet(default)]
    pub name: Option<String>,
    /// The layer (`person` when left out).
    #[facet(default)]
    pub layer: Option<String>,
}

#[derive(Debug, Clone, Default, Facet)]
pub struct InspectArgs {
    /// Only the tokens whose name starts with it (`arrange.`, `ink`).
    #[facet(default)]
    pub prefix: Option<String>,
}

/// One token, as `THEME_INSPECT` answers it.
#[derive(Debug, Clone, Facet)]
pub struct TokenOut {
    pub name: String,
    /// The final value (an expression's, evaluated).
    pub value: String,
    /// `derived`, `theme` or `layer`.
    pub origin: String,
    /// The layer that set it, or whose choice shows the theme that did.
    pub layer: Option<String>,
    /// The theme (or the layer) that set it.
    pub id: String,
    pub reference: bool,
    /// The expression as written, when it is one.
    pub expression: Option<String>,
}

#[derive(Debug, Clone, Facet)]
pub struct InspectOut {
    pub theme: String,
    pub revision: u64,
    pub tokens: Vec<TokenOut>,
}

/// A write's answer.
#[derive(Debug, Clone, Copy, Facet)]
pub struct Changed {
    /// Whether anything that shows changed.
    pub changed: bool,
    /// The revision after it.
    pub revision: u64,
}

#[derive(Debug, Clone, Facet)]
pub struct ProblemOut {
    pub theme: String,
    pub source: String,
    pub field: Option<String>,
    pub message: String,
    /// All of it in one line, for a person.
    pub text: String,
}

#[derive(Debug, Clone, Facet)]
pub struct ProblemsOut {
    pub problems: Vec<ProblemOut>,
}

fn changed(store: &ThemeStore, changed: bool) -> Changed {
    Changed {
        changed,
        revision: store.revision(),
    }
}

/// Whether `name` is asked for by `wanted` (names, or `prefix-*`).
fn wanted(name: &str, wanted: &[String]) -> bool {
    wanted.iter().any(|w| {
        w.strip_suffix('*')
            .map_or(w == name, |prefix| name.starts_with(prefix))
    })
}

/// `THEME_GET`'s answer for `store`, its tokens filtered by `tokens`.
#[must_use]
pub fn get(store: &ThemeStore, tokens: Option<&[String]>) -> GetOut {
    let composed = store.composed();
    let current = store.current();
    let (one, light, dark) = match &composed.choice {
        ThemeChoice::One(id) => (Some(id.clone()), None, None),
        ThemeChoice::Pair { light, dark } => (None, Some(light.clone()), Some(dark.clone())),
    };
    GetOut {
        choice: ChoiceOut { one, light, dark },
        appearance: match composed.appearance {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
        .to_owned(),
        system: store.system().as_str().to_owned(),
        scheme: current.scheme().as_str().to_owned(),
        theme: current.id().to_owned(),
        light: composed.choice.pick(crate::Scheme::Light).to_owned(),
        dark: composed.choice.pick(crate::Scheme::Dark).to_owned(),
        revision: current.revision(),
        tokens: current
            .tokens()
            .iter()
            .filter(|(k, _)| tokens.is_none_or(|w| wanted(k, w)))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

/// `THEME_INSPECT`'s answer for `store`: every token under `prefix`.
#[must_use]
pub fn inspect(store: &ThemeStore, prefix: Option<&str>) -> InspectOut {
    let current = store.current();
    let tokens = current
        .tokens()
        .iter()
        .filter(|(k, _)| prefix.is_none_or(|p| k.starts_with(p)))
        .map(|(name, value)| {
            let source = current.source(name);
            TokenOut {
                name: name.clone(),
                value: value.clone(),
                origin: source.map_or("derived", |s| s.origin.as_str()).to_owned(),
                layer: source.and_then(|s| s.layer).map(|l| l.as_str().to_owned()),
                id: source.map_or_else(|| current.id().to_owned(), |s| s.id.clone()),
                reference: source.is_some_and(crate::TokenSource::is_reference),
                expression: source.and_then(|s| s.expression.clone()),
            }
        })
        .collect();
    InspectOut {
        theme: current.id().to_owned(),
        revision: current.revision(),
        tokens,
    }
}

/// A token action's `layer`: the person's when left out; never the
/// built-in one (architect's own).
fn layer_arg(layer: Option<&str>) -> Result<LayerKind, String> {
    let Some(name) = layer else {
        return Ok(LayerKind::Person);
    };
    match LayerKind::parse(name) {
        Some(LayerKind::Builtin) => Err("the builtin layer is architect's own".to_owned()),
        Some(LayerKind::Extension) => {
            Err("the extension layer is what loaded extensions contribute".to_owned())
        }
        Some(kind) => Ok(kind),
        None => Err(format!(
            "{name:?} is not a layer (app, org, person, device, workspace, pick, preview)"
        )),
    }
}

/// `THEME_TOKEN_SET` on `store`.
pub fn token_set(store: &ThemeStore, args: &TokenSetArgs) -> Result<Changed, String> {
    if !valid_token(&args.name) {
        return Err(format!("{:?}: {TOKEN_NAME_RULE}", args.name));
    }
    let value = args.value.trim();
    if value.is_empty() {
        return Err("an empty value: THEME_TOKEN_RESET removes an override".to_owned());
    }
    crate::expr::parse(value)?;
    let layer = layer_arg(args.layer.as_deref())?;
    let did = store.set_token(layer, &args.name, value);
    Ok(changed(store, did))
}

/// `THEME_SET` on `store`.
pub fn set(store: &ThemeStore, args: &SetArgs) -> Result<Changed, String> {
    if args.one.is_none()
        && args.light.is_none()
        && args.dark.is_none()
        && args.appearance.is_none()
    {
        return Err("give `one`, `light`, `dark` or `appearance`".to_owned());
    }
    if args.one.is_some() && (args.light.is_some() || args.dark.is_some()) {
        return Err("`one`, or `light` / `dark`: not both".to_owned());
    }
    let registry = store.registry();
    for id in [&args.one, &args.light, &args.dark].into_iter().flatten() {
        if !registry.contains(id) {
            return Err(format!("no theme {id:?} (THEME_LIST has them)"));
        }
    }
    let now = store.composed().choice;
    let choice = match (&args.one, &args.light, &args.dark) {
        (Some(one), _, _) => Some(ThemeChoice::One(one.clone())),
        (None, None, None) => None,
        (None, light, dark) => Some(ThemeChoice::pair(
            light
                .clone()
                .unwrap_or_else(|| now.pick(crate::Scheme::Light).to_owned()),
            dark.clone()
                .unwrap_or_else(|| now.pick(crate::Scheme::Dark).to_owned()),
        )),
    };
    let did = store.update(LayerKind::Pick, |layer| {
        if let Some(choice) = choice {
            layer.choice = Some(choice);
        }
        if let Some(appearance) = args.appearance {
            layer.appearance = Some(appearance);
        }
    });
    Ok(changed(store, did))
}

/// Puts the `THEME_*` actions on `list`, driving `store`.
pub fn register(list: &ActionList, store: &ThemeStore) -> Result<(), ActionError> {
    let s = store.clone();
    list.register(&LIST, Effect::ReadOnly, move |(): ()| {
        Ok(ListOut {
            showing: s.current().id().to_owned(),
            themes: s
                .themes()
                .into_iter()
                .map(|t| ThemeItem {
                    id: t.id,
                    label: t.label,
                    appearance: t.scheme.map(|s| s.as_str().to_owned()),
                    source: t.source.label(),
                    origin: t.source.to_string(),
                    problems: t.problems,
                })
                .collect(),
        })
    })?;
    let s = store.clone();
    list.register(&GET, Effect::ReadOnly, move |args: GetArgs| {
        Ok(get(&s, args.tokens.as_deref()))
    })?;
    let s = store.clone();
    list.register(&SET, Effect::Mutating, move |args: SetArgs| set(&s, &args))?;
    let s = store.clone();
    list.register(&PREVIEW, Effect::Mutating, move |args: IdArgs| {
        let did = s.preview(&args.id)?;
        Ok(changed(&s, did))
    })?;
    let s = store.clone();
    list.register(&PREVIEW_END, Effect::Mutating, move |(): ()| {
        let did = s.end_preview();
        Ok(changed(&s, did))
    })?;
    let s = store.clone();
    list.register(&TOKEN_SET, Effect::Mutating, move |args: TokenSetArgs| {
        token_set(&s, &args)
    })?;
    let s = store.clone();
    list.register(
        &TOKEN_RESET,
        Effect::Mutating,
        move |args: TokenResetArgs| {
            let layer = layer_arg(args.layer.as_deref())?;
            let did = s.reset_token(layer, args.name.as_deref());
            Ok(changed(&s, did))
        },
    )?;
    let s = store.clone();
    list.register(&RELOAD, Effect::Mutating, move |(): ()| {
        let did = s.reload();
        Ok(changed(&s, did))
    })?;
    let s = store.clone();
    list.register(&PROBLEMS, Effect::ReadOnly, move |(): ()| {
        Ok(ProblemsOut {
            problems: s
                .problems()
                .into_iter()
                .map(|p| ProblemOut {
                    text: p.to_string(),
                    theme: p.theme,
                    source: p.source,
                    field: p.field,
                    message: p.message,
                })
                .collect(),
        })
    })?;
    let s = store.clone();
    list.register(&INSPECT, Effect::ReadOnly, move |args: InspectArgs| {
        Ok(inspect(&s, args.prefix.as_deref()))
    })?;
    report(list, store);
    Ok(())
}

/// The theme's part of `APP_STATUS`: its `theme` section (the choice,
/// the theme and scheme showing, the revision) and its problems, each
/// with its file and field (`architect::action::app_status`).
pub fn report(list: &ActionList, store: &ThemeStore) {
    let s = store.clone();
    list.status().section(STATUS_SECTION, move || {
        let mut now = get(&s, Some(&[] as &[String]));
        now.tokens.clear();
        facet_json::to_string(&now).unwrap_or_else(|_| "{}".to_owned())
    });
    let s = store.clone();
    list.problems().source(STATUS_SECTION, move || {
        s.problems()
            .into_iter()
            .map(|p| {
                let mut out = architect::action::Problem::new(
                    STATUS_SECTION,
                    format!("{}: {}", p.theme, p.message),
                );
                if p.source != "builtin" && p.source != "memory" {
                    out = out.file(p.source);
                }
                if let Some(field) = p.field {
                    out = out.field(field);
                }
                out
            })
            .collect()
    });
}

/// The theme's section of `APP_STATUS`, and its problems' source.
pub const STATUS_SECTION: &str = "theme";
