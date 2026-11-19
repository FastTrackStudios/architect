//! What the viewer may do in the app as it is open now, and the controls
//! that follow from it.
//!
//! A share link is this same app, opened as a guest: what a guest cannot
//! do is a permission, never another app (`guide/shares.md`). The shell's
//! own controls are held to that here. Each control says what it
//! [`Needs`]; the viewer's [`Permits`] say whether it shows. A guest has
//! no vault to browse or search and writes nothing, so "Files", "Open…",
//! "Vault", "Search" and "New note" are not drawn for one, and are drawn
//! again the moment the same person signs in.

/// What a control needs of the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Needs {
    /// Nothing: every viewer has it (the palette, settings, the player).
    Nothing,
    /// A vault of the viewer's to browse, search and open notes in.
    Vault,
    /// To write to that vault (a new note).
    Write,
}

/// What the viewer has, as the app is open now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permits {
    /// A vault of their own on show (the demo's, an org's).
    pub vault: bool,
    /// They may write to it.
    pub write: bool,
}

impl Permits {
    /// A member on their org's vaults, or the demo on its own.
    pub const MEMBER: Self = Self {
        vault: true,
        write: true,
    };
    /// A share link's guest: the link's notes, as the node hands them,
    /// read-only; no vault.
    pub const GUEST: Self = Self {
        vault: false,
        write: false,
    };

    /// Whether a control that needs `needs` shows.
    #[must_use]
    pub const fn allows(self, needs: Needs) -> bool {
        match needs {
            Needs::Nothing => true,
            Needs::Vault => self.vault,
            Needs::Write => self.vault && self.write,
        }
    }
}

/// What one of the web app's own bottom tabs needs, by its id
/// (`app::favourites::own_set`): all three are the viewer's vault.
#[must_use]
pub fn own_tab(id: &str) -> Needs {
    match id {
        "vault" | "files" | "search" => Needs::Vault,
        _ => Needs::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guest_has_no_control_that_needs_a_vault_or_a_write() {
        for needs in [Needs::Vault, Needs::Write] {
            assert!(!Permits::GUEST.allows(needs), "{needs:?}");
            assert!(Permits::MEMBER.allows(needs), "{needs:?}");
        }
        // What every viewer has: the palette, settings, the player.
        assert!(Permits::GUEST.allows(Needs::Nothing));
        // A read-only member (a vault, no writes) browses and does not write.
        let reader = Permits {
            vault: true,
            write: false,
        };
        assert!(reader.allows(Needs::Vault) && !reader.allows(Needs::Write));
    }

    #[test]
    fn the_apps_own_tabs_are_the_viewers_vault() {
        for id in ["vault", "files", "search"] {
            assert_eq!(own_tab(id), Needs::Vault);
            assert!(!Permits::GUEST.allows(own_tab(id)));
        }
        // A note type's declared favourite (an extension's view) is not the
        // app's to gate here: its extension's capabilities are.
        assert_eq!(own_tab("setlist"), Needs::Nothing);
    }
}
