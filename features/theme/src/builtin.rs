//! architect's own themes, compiled in: the files under
//! `features/theme/themes/`, each a light or a dark half. `FastTrackStudio`
//! dark is the default.

use std::sync::OnceLock;

use crate::model::{Registry, Scheme, Source, Theme, ThemeFile};
use crate::resolve::{Resolved, resolve};

/// The theme a dark appearance shows when nothing chooses one.
pub const DEFAULT_DARK: &str = "fasttrackstudio-dark";
/// The theme a light appearance shows when nothing chooses one.
pub const DEFAULT_LIGHT: &str = "fasttrackstudio-light";

macro_rules! builtin {
    ($($id:literal),* $(,)?) => {
        /// Every built-in theme file: its id and its text.
        pub const FILES: &[(&str, &str)] = &[
            $(($id, include_str!(concat!("../themes/", $id, ".json"))),)*
        ];
    };
}

builtin![
    "default-dark",
    "default-light",
    "fasttrackstudio-dark",
    "fasttrackstudio-light",
    "modern-minimal-dark",
    "modern-minimal-light",
    "violet-bloom-dark",
    "violet-bloom-light",
    "t3-chat-dark",
    "t3-chat-light",
    "amethyst-haze-dark",
    "amethyst-haze-light",
    "doom-64-dark",
    "doom-64-light",
    "graphite-dark",
    "graphite-light",
    "supabase-dark",
    "supabase-light",
    "neo-brutalism-dark",
    "neo-brutalism-light",
    "catppuccin-dark",
    "catppuccin-light",
    "nord-dark",
    "nord-light",
    "dracula-dark",
    "dracula-light",
    "gruvbox-dark",
    "gruvbox-light",
    "solarized-dark",
    "solarized-light",
    "tokyo-night-dark",
    "tokyo-night-light",
    "rose-pine-dark",
    "rose-pine-light",
    "everforest-dark",
    "everforest-light",
    "zen-dark",
    "zen-light",
    "obsidian-dark",
    "obsidian-light",
];

impl Registry {
    /// architect's built-in themes. A file that does not read is one of
    /// [`Registry::problems`] (a test keeps there being none).
    #[must_use]
    pub fn builtin() -> Self {
        static BUILTIN: OnceLock<Registry> = OnceLock::new();
        BUILTIN
            .get_or_init(|| {
                let mut registry = Self::empty();
                for (id, text) in FILES {
                    match Theme::parse(id, text, Source::Builtin) {
                        Ok(theme) => registry.insert(theme),
                        Err(problems) => registry.report(problems),
                    }
                }
                registry
            })
            .clone()
    }
}

/// The built-in default for `scheme`: what shows when a chosen theme is
/// not there.
#[must_use]
pub fn fallback(scheme: Scheme) -> Theme {
    let id = match scheme {
        Scheme::Dark => DEFAULT_DARK,
        Scheme::Light => DEFAULT_LIGHT,
    };
    Registry::builtin().get(id).cloned().unwrap_or_else(|| {
        Theme::new(
            id,
            ThemeFile {
                appearance: Some(scheme),
                ..ThemeFile::default()
            },
            Source::Builtin,
        )
    })
}

/// [`DEFAULT_DARK`], resolved.
#[must_use]
pub fn default_resolved() -> Resolved {
    static DEFAULT: OnceLock<Resolved> = OnceLock::new();
    DEFAULT
        .get_or_init(|| resolve(&fallback(Scheme::Dark), &Registry::builtin()))
        .clone()
}
