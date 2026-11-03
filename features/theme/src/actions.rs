//! The `THEME_*` actions (feature `actions`): the store driven from
//! anywhere the Action List reaches — ⌘K, keys, MCP, the command line.
//!
//! | id | effect | does |
//! |---|---|---|
//! | `THEME_LIST` | read-only | every theme: id, label, appearance, source, problems |
//! | `THEME_GET` | read-only | the choice, the appearance, the theme showing, the revision, and its tokens (all, or the names and `prefix-*`es asked for) |
//! | `THEME_SET` | mutating | the pick: `one`, or `light` / `dark` halves, and/or `appearance` |
//! | `THEME_PREVIEW` | mutating | shows a theme over everything until `THEME_PREVIEW_END` |
//! | `THEME_PREVIEW_END` | mutating | ends the preview |
//! | `THEME_TOKEN_SET` | mutating | a token overridden in the person's layer |
//! | `THEME_TOKEN_RESET` | mutating | one (or every) override of the person's layer removed |
//! | `THEME_RELOAD` | mutating | the theme files read again |
//! | `THEME_PROBLEMS` | read-only | what is wrong with the theme files and the choice |
//!
//! Every write answers whether anything that shows changed, and the
//! revision after it. None is destructive: each is undone by another.

// The facet derive's generated code trips the arithmetic lint.
#![allow(clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;

use architect::action::{ActionError, ActionList, ActionMeta, Effect};
use facet::Facet;

use crate::model::{Appearance, ThemeChoice, valid_token};
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
    LIST = LIST, "Themes", "Every theme the app knows: its id, label, appearance (light or dark), source (builtin, file, extension) and problems.";
    GET = GET, "Current theme", "The choice (one theme, or a light and a dark), the appearance (system, light, dark), the theme showing, the revision, and its resolved tokens: all, or only the `tokens` named (a `prefix-*` matches every name starting `prefix-`).";
    SET = SET, "Set the theme", "Picks the theme: `one` always shows it; `light` / `dark` set that half of the pair (the other half stays). `appearance` is system, light or dark. Answers whether anything changed and the revision.";
    PREVIEW = PREVIEW, "Preview a theme", "Shows theme `id` over every other choice until THEME_PREVIEW_END; nothing is kept.";
    PREVIEW_END = PREVIEW_END, "End the theme preview", "Ends THEME_PREVIEW: the chosen theme shows again.";
    TOKEN_SET = TOKEN_SET, "Set a theme token", "Overrides token `name` (as THEME_GET names it: `primary`, `ink-3`, `radius`) with `value` (any CSS value) in the person's layer, over every theme.";
    TOKEN_RESET = TOKEN_RESET, "Reset a theme token", "Removes the person's override of token `name`, or every one when `name` is left out.";
    RELOAD = RELOAD, "Reload themes", "Reads the theme files again. A file that no longer reads keeps its last good theme and is reported (THEME_PROBLEMS).";
    PROBLEMS = PROBLEMS, "Theme problems", "What is wrong: theme files that do not read (file, field, message), missing or looping parents, a chosen theme that is not there.";
}

/// One theme, as `THEME_LIST` answers it.
#[derive(Debug, Clone, Facet)]
pub struct ThemeItem {
    pub id: String,
    pub label: String,
    /// `light` or `dark`; unset when its parent is missing.
    pub appearance: Option<String>,
    /// `builtin`, `file` or `extension`.
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
}

#[derive(Debug, Clone, Default, Facet)]
pub struct TokenResetArgs {
    /// Every override when left out.
    #[facet(default)]
    pub name: Option<String>,
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
                    source: t.source.kind().to_owned(),
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
        if !valid_token(&args.name) {
            return Err(format!(
                "{:?}: a token name is lowercase letters, digits and `-`",
                args.name
            ));
        }
        if args.value.trim().is_empty() {
            return Err("an empty value: THEME_TOKEN_RESET removes an override".to_owned());
        }
        let did = s.set_token(LayerKind::Person, &args.name, args.value.trim());
        Ok(changed(&s, did))
    })?;
    let s = store.clone();
    list.register(
        &TOKEN_RESET,
        Effect::Mutating,
        move |args: TokenResetArgs| {
            let did = s.reset_token(LayerKind::Person, args.name.as_deref());
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
    Ok(())
}
