//! An [`Icon`] drawn (Notebook Navigator's `IconService.renderIcon`): a
//! Lucide icon from architect's table, an emoji as text, a vault SVG from
//! the source; an icon pack's, with no pack here, as a box named for it.

// `Navigator` is a `Copy` handle of signals, taken as the components take it.
#![allow(clippy::large_types_passed_by_value)]

use architect_shell::lucide::{LucideIcon, glyphs};
use dioxus::prelude::*;

use super::Navigator;
use crate::icons::Icon;

/// `icon`, `size` points square, in `colour` (else the text's).
#[component]
pub(super) fn IconView(nav: Navigator, icon: Icon, size: u32, colour: Option<String>) -> Element {
    let id = icon.id();
    match icon {
        Icon::Lucide(name) if glyphs(&name).is_some() => rsx! {
            span { class: "nv-icon", "data-icon": "{id}", style: "display:flex;",
                LucideIcon { name, size, color: colour }
            }
        },
        Icon::Emoji(e) => rsx! {
            span { class: "nv-icon nv-emoji", "data-icon": "{id}",
                style: "display:flex; align-items:center; justify-content:center; width:{size}px; height:{size}px; font-size:{size - 2}px; line-height:1;",
                "{e}"
            }
        },
        Icon::Vault(path) => match nav.source().resource_url(&path) {
            Some(url) => rsx! {
                img { class: "nv-icon", "data-icon": "{id}", src: "{url}", alt: "", width: "{size}", height: "{size}",
                    style: "display:block; width:{size}px; height:{size}px;" }
            },
            None => rsx! { Fallback { id, size, colour } },
        },
        _ => rsx! { Fallback { id, size, colour } },
    }
}

/// An icon this view cannot draw: a pack's (none installed) or a name
/// Lucide has not: a box, named in its tooltip.
#[component]
fn Fallback(id: String, size: u32, colour: Option<String>) -> Element {
    rsx! {
        span { class: "nv-icon nv-icon-missing", "data-icon": "{id}", title: "{id}", style: "display:flex;",
            LucideIcon { name: "package".to_owned(), size, color: colour }
        }
    }
}
