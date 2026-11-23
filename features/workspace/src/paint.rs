//! Views that paint a scene each frame: `architect_scene::paint`, and
//! what the workspace's panes tell one mounted in them ([`workspace_chrome`]).

pub use architect_scene::paint::*;

#[cfg(feature = "paint-ui")]
mod chrome {
    use architect_shell::ShellTheme;
    use dioxus::prelude::*;

    use super::{Chrome, PaneChrome};
    use crate::{PaneId, ThemeTokens};

    /// The shell's theme as a view's tokens.
    ///
    /// By the names a resolved theme gives them (`architect_theme`): its fields with `_` as `-`
    /// (`"ink-2"`), the accent trio as `"brand"`, `"brand-ink"`,
    /// `"brand-soft"` (`"accent"` is architect-ui's hover wash). What a pane
    /// gets when the app has a shell but no theme store.
    #[must_use]
    pub fn shell_tokens(theme: &ShellTheme) -> ThemeTokens {
        [
            ("desk", &theme.desk),
            ("sheet", &theme.sheet),
            ("title-bar", &theme.title_bar),
            ("pill", &theme.pill),
            ("rule", &theme.rule),
            ("active", &theme.active),
            ("ink", &theme.ink),
            ("ink-2", &theme.ink_2),
            ("ink-3", &theme.ink_3),
            ("brand", &theme.accent),
            ("brand-ink", &theme.accent_ink),
            ("danger", &theme.danger),
            ("scrim", &theme.scrim),
            ("brand-soft", &theme.accent_soft),
            ("font", &theme.font),
        ]
        .into_iter()
        .fold(ThemeTokens::default(), |t, (name, value)| {
            t.with(name, value.to_string())
        })
    }

    /// The tokens a pane sees now: the app's theme store's whole resolved
    /// theme when there is one, else the shell's colours, else none. Read in
    /// a render, the component renders again when the theme changes.
    #[must_use]
    pub fn pane_tokens() -> ThemeTokens {
        if let Some(store) = architect_theme::try_use_theme() {
            return ThemeTokens::from(&*store.resolved().read());
        }
        architect_shell::try_use_shell()
            .map(|s| shell_tokens(&s.theme))
            .unwrap_or_default()
    }

    /// A pane's [`Chrome`]: its id as the workspace has it now, and
    /// [`pane_tokens`].
    #[must_use]
    pub fn workspace_chrome(pane: Signal<PaneId>) -> Chrome {
        struct Pane(Signal<PaneId>);
        impl PaneChrome for Pane {
            fn pane(&self) -> PaneId {
                self.0.read().clone()
            }
            fn tokens(&self) -> ThemeTokens {
                pane_tokens()
            }
        }
        Chrome::new(Pane(pane))
    }
}

#[cfg(feature = "paint-ui")]
pub use chrome::{pane_tokens, shell_tokens, workspace_chrome};

#[cfg(all(test, feature = "paint-ui"))]
mod document_tests;
