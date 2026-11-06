//! The vault profiles' editor, a page of its own on the settings page.
//!
//! Notebook Navigator's `EditVaultProfilesModal`: each profile's name,
//! what it hides, its file visibility and banner; one added, used,
//! removed. It edits the navigator on show ([`use_profiles_page`]), and
//! is opened from the profile menu ([`open_profiles_page`]).

// `Navigator` is a `Copy` handle of signals, taken as the components take it.
#![allow(clippy::large_types_passed_by_value)]

use std::cell::Cell;

use architect_settings::Section;
use dioxus::prelude::*;

use super::Navigator;
use crate::hidden::FileVisibility;
use crate::profiles::VaultProfile;

/// The settings page's id for it.
pub const SECTION: &str = "navigator-profiles";

thread_local! {
    /// The navigator the page edits: the one on show.
    static SHOWN: Cell<Option<Navigator>> = const { Cell::new(None) };
    /// The settings page it opens on.
    static SETTINGS: std::cell::RefCell<Option<architect_settings::SettingsHandle>> =
        const { std::cell::RefCell::new(None) };
}

/// Lets "Vault profiles" be opened on the settings page while `nav` is on
/// show ([`open_profiles_page`]); the page is there from the first time it
/// is opened until the view goes.
pub fn use_profiles_page(nav: Navigator) {
    let handle = architect_settings::try_use_settings();
    use_hook(|| {
        SHOWN.with(|s| s.set(Some(nav)));
        SETTINGS.with(|s| *s.borrow_mut() = handle);
    });
    use_drop(move || {
        SHOWN.with(|s| s.set(None));
        SETTINGS.with(|s| *s.borrow_mut() = None);
        if let Some(h) = handle {
            let (mut registry, mut pages) = (h.registry, h.pages);
            registry.write().remove(SECTION);
            pages.write().remove(SECTION);
        }
    });
}

/// Opens the vault profiles' editor on the settings page ("Edit
/// profiles…", `NAVIGATOR_PROFILE`'s `edit`). Whether a settings page was
/// there to open it on.
pub fn open_profiles_page() -> bool {
    let Some(h) = SETTINGS.with(|s| *s.borrow()) else {
        return false;
    };
    if h.registry.peek().section(SECTION).is_none() {
        let (mut registry, mut pages) = (h.registry, h.pages);
        let section = Section::new(SECTION, "Vault profiles")
            .icon("users")
            .group("");
        if registry.write().register(section).is_err() {
            return false;
        }
        pages.write().insert(SECTION.to_owned(), ProfilesPage);
    }
    h.show(Some(SECTION));
    true
}

/// A profile's lists, as the page writes them: each a comma list.
const LISTS: &[(&str, &str, &str)] = &[
    (
        "folders",
        "Hidden folders",
        "archive, temp*, *_old, /projects/*/drafts",
    ),
    ("tags", "Hidden tags", "archive, temp*, *draft"),
    (
        "names",
        "Hidden notes and files",
        "*.excalidraw.md, Templates/*",
    ),
    (
        "properties",
        "Hidden by frontmatter",
        "draft, status=archived",
    ),
    ("file_tags", "Hidden by tag", "private"),
    ("excluded", "Kept out of the lists above", "Journal"),
];

fn list_of(p: &VaultProfile, which: &str) -> String {
    let l = match which {
        "folders" => &p.hidden_folders,
        "tags" => &p.hidden_tags,
        "names" => &p.hidden_file_names,
        "properties" => &p.hidden_file_properties,
        "file_tags" => &p.hidden_file_tags,
        _ => &p.descendant_excluded,
    };
    l.join(", ")
}

fn set_list(p: &mut VaultProfile, which: &str, text: &str) {
    let items: Vec<String> = text
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect();
    match which {
        "folders" => p.hidden_folders = items,
        "tags" => p.hidden_tags = items,
        "names" => p.hidden_file_names = items,
        "properties" => p.hidden_file_properties = items,
        "file_tags" => p.hidden_file_tags = items,
        _ => p.descendant_excluded = items,
    }
}

const FIELD: &str = "height:28px; padding:0 8px; border-radius:6px; border:1px solid var(--nv-divider); \
                     background:transparent; color:var(--nv-text); font-size:13px; min-width:0; flex:1 1 auto;";
const BUTTON: &str = "height:28px; padding:0 10px; border-radius:6px; border:1px solid var(--nv-divider); \
                      background:transparent; color:var(--nv-text); cursor:pointer; font-size:12px;";

/// The page.
#[component]
fn ProfilesPage() -> Element {
    let Some(nav) = SHOWN.with(Cell::get) else {
        return rsx! {
            div { style: "padding:16px; font-size:13px; opacity:0.7;", "Open the navigator to edit its vault profiles." }
        };
    };
    // The navigator's colours, as its view has them.
    let vars = architect_theme::use_themed::<super::NavigatorColours>()
        .read()
        .vars();
    let s = nav.state.read();
    // Collected, so the state is let go before the cards are drawn.
    #[allow(clippy::needless_collect)]
    let profiles: Vec<VaultProfile> = s.profiles().all().cloned().collect();
    let active = s.profile().id.clone();
    drop(s);
    let mut new_name = use_signal(String::new);
    rsx! {
        div { class: "nv-profiles", "data-testid": "nv-profiles", style: "{vars} display:flex; flex-direction:column; gap:14px; padding:16px; color:var(--nv-text);",
            div { style: "font-size:12px; color:var(--nv-muted);",
                "Each profile hides its own folders, tags and notes, and has its own banner, shortcuts and recent notes. Patterns: a name, prefix*, *suffix, or a /path with *."
            }
            for p in profiles {
                ProfileCard { key: "{p.id}", nav, active: p.id == active, profile: p }
            }
            div { style: "display:flex; gap:8px; align-items:center;",
                input { "data-testid": "nv-profile-new-name", placeholder: "New profile's name", value: "{new_name}", style: FIELD,
                    oninput: move |e| new_name.set(e.value()) }
                button { "data-testid": "nv-profile-add", style: BUTTON,
                    onclick: move |_| {
                        let name = new_name.peek().trim().to_owned();
                        if name.is_empty() {
                            return;
                        }
                        match nav.update(|s| s.add_profile(&name, false)) {
                            Ok(_) => new_name.set(String::new()),
                            Err(e) => nav.say(e),
                        }
                    },
                    "Add profile"
                }
            }
        }
    }
}

/// One profile.
#[component]
fn ProfileCard(nav: Navigator, active: bool, profile: VaultProfile) -> Element {
    let id = profile.id.clone();
    let default = id == crate::profiles::DEFAULT_ID;
    let (i1, i2, i3, i4, i5, i6, i7) = (
        id.clone(),
        id.clone(),
        id.clone(),
        id.clone(),
        id.clone(),
        id.clone(),
        id.clone(),
    );
    let banner = profile.banner.clone().unwrap_or_default();
    let keys = profile.property_keys.clone().unwrap_or_default();
    let periodic = profile.periodic_folder.clone().unwrap_or_default();
    let vis = profile.file_visibility;
    let border = if active {
        "2px solid var(--nv-open)"
    } else {
        "1px solid var(--nv-divider)"
    };
    rsx! {
        div { class: "nv-profile", "data-profile": "{id}", "data-active": if active { "true" } else { "false" },
            style: "display:flex; flex-direction:column; gap:8px; padding:12px; border-radius:10px; border:{border};",
            div { style: "display:flex; gap:8px; align-items:center;",
                input { "aria-label": "Name", value: "{profile.name}", style: FIELD,
                    onchange: move |e| {
                        if let Err(err) = nav.update(|s| s.rename_profile(&i1, &e.value())) {
                            nav.say(err);
                        }
                    },
                }
                if active {
                    span { style: "font-size:12px; color:var(--nv-muted);", "In use" }
                } else {
                    button { style: BUTTON, "data-testid": "nv-profile-use",
                        onclick: move |_| {
                            if let Err(e) = nav.update(|s| s.select_profile(&i2)) {
                                nav.say(e);
                            }
                        },
                        "Use"
                    }
                }
                if !default {
                    button { style: BUTTON, "data-testid": "nv-profile-remove",
                        onclick: move |_| {
                            if let Err(e) = nav.update(|s| s.remove_profile(&i3)) {
                                nav.say(e);
                            }
                        },
                        "Remove"
                    }
                }
            }
            for (which, title, hint) in LISTS {
                ListField { key: "{which}", nav, id: id.clone(), which: (*which).to_owned(), title: (*title).to_owned(), hint: (*hint).to_owned(), value: list_of(&profile, which) }
            }
            div { style: "display:flex; gap:8px; align-items:center;",
                span { style: "flex:0 0 180px; font-size:12px;", "Files shown" }
                for (v, label) in [(FileVisibility::Documents, "Documents"), (FileVisibility::Supported, "Supported"), (FileVisibility::All, "All")] {
                    button { key: "{label}", style: if vis == v { format!("{BUTTON} font-weight:700; border-color:var(--nv-open);") } else { BUTTON.to_owned() },
                        "aria-pressed": if vis == v { "true" } else { "false" },
                        onclick: {
                            let id = i4.clone();
                            move |_| {
                                let _ = nav.update(|s| s.edit_profile_in(&id, |p| p.file_visibility = v));
                            }
                        },
                        "{label}"
                    }
                }
            }
            div { style: "display:flex; gap:8px; align-items:center;",
                span { style: "flex:0 0 180px; font-size:12px;", "Banner" }
                input { "aria-label": "Banner", placeholder: "A vault image or a URL", value: "{banner}", style: FIELD,
                    onchange: move |e| {
                        let v = e.value();
                        let _ = nav.update(|s| s.edit_profile_in(&i5, |p| {
                            p.banner = Some(v.trim().to_owned()).filter(|b| !b.is_empty());
                        }));
                    },
                }
            }
            // Over the settings' (`propertyKeys`, `periodicNotesFolder`).
            div { style: "display:flex; gap:8px; align-items:center;",
                span { style: "flex:0 0 180px; font-size:12px;", "Property keys" }
                input { "aria-label": "Property keys", placeholder: "The settings' (status, type)", value: "{keys}", style: FIELD,
                    onchange: move |e| {
                        let v = e.value();
                        let _ = nav.update(|s| s.edit_profile_in(&i6, |p| {
                            p.property_keys = Some(v.trim().to_owned()).filter(|b| !b.is_empty());
                        }));
                    },
                }
            }
            div { style: "display:flex; gap:8px; align-items:center;",
                span { style: "flex:0 0 180px; font-size:12px;", "Periodic notes folder" }
                input { "aria-label": "Periodic notes folder", placeholder: "The settings' (Journal)", value: "{periodic}", style: FIELD,
                    onchange: move |e| {
                        let v = e.value();
                        let _ = nav.update(|s| s.edit_profile_in(&i7, |p| {
                            p.periodic_folder = Some(v.trim().to_owned()).filter(|b| !b.is_empty());
                        }));
                    },
                }
            }
        }
    }
}

/// One of a profile's lists, a comma list.
#[component]
fn ListField(
    nav: Navigator,
    id: String,
    which: String,
    title: String,
    hint: String,
    value: String,
) -> Element {
    rsx! {
        div { style: "display:flex; gap:8px; align-items:center;",
            span { style: "flex:0 0 180px; font-size:12px;", "{title}" }
            input { "aria-label": "{title}", "data-list": "{which}", placeholder: "{hint}", value: "{value}", style: FIELD,
                onchange: move |e| {
                    let text = e.value();
                    let _ = nav.update(|s| s.edit_profile_in(&id, |p| set_list(p, &which, &text)));
                },
            }
        }
    }
}
