//! The settings application: a rail, and a working surface.
//!
//! # Two shells, on purpose
//!
//! Signing in is one decision, so it gets one centred card — nothing
//! else on the screen, nothing else to do. Everything after signing in
//! is *managing* things, which means moving between them, so it gets a
//! persistent rail instead. A settings page that made you go back to a
//! menu between every task would be the card shell used where it does
//! not fit.
//!
//! # The rail shows standing, not routes
//!
//! It carries the person's name, the organizations they belong to with
//! their role in each, and — only for a server administrator — the
//! server itself. That is the thing this product is about: which
//! account you are holding and which rooms you are in. A plain list of
//! page names would have been the generic answer and would have told
//! nobody anything they did not already know.

use architect_auth::{AuthStorage, CurrentSession, ListOrganizations};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse as _, Response};
use dioxus::prelude::*;

use crate::UiState;
use crate::chrome::STYLE;
use crate::page::token_of;

/// One organization, as the rail shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct NavOrg {
    pub id: String,
    pub name: String,
    pub role: String,
}

/// Everything the rail needs.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Nav {
    /// The path being rendered, so one item can mark itself current.
    pub current: String,
    pub name: String,
    pub email: String,
    pub is_admin: bool,
    pub is_guest: bool,
    pub orgs: Vec<NavOrg>,
}

impl Nav {
    /// Read the rail's contents for the signed-in person.
    ///
    /// Two queries: the session, and the organizations. Admin is read
    /// off the user rather than asked for, because every admin route
    /// checks properly on its own — this only decides whether a link
    /// is worth showing.
    pub async fn build<S>(state: &UiState<S>, headers: &HeaderMap, current: &str) -> Option<Self>
    where
        S: AuthStorage,
    {
        let token = token_of(headers, &state.cookie)?;
        let session = state
            .auth
            .current_session(CurrentSession {
                token: token.clone(),
            })
            .await
            .ok()?;
        let orgs = state
            .auth
            .list_organizations(ListOrganizations {
                session_token: token,
            })
            .await
            .unwrap_or_default();

        Some(Self {
            current: current.to_owned(),
            name: session
                .user
                .name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .or_else(|| session.user.username.clone())
                .unwrap_or_else(|| {
                    if crate::guest::is_guest(&session.user) {
                        "Guest".to_owned()
                    } else {
                        "Unnamed".to_owned()
                    }
                }),
            email: session.user.email.clone().unwrap_or_default(),
            is_admin: session.user.role.as_deref() == Some("admin"),
            is_guest: crate::guest::is_guest(&session.user),
            orgs: orgs
                .into_iter()
                .map(|bundle| NavOrg {
                    id: bundle.organization.id.to_string(),
                    name: bundle.organization.name,
                    role: bundle.membership.role,
                })
                .collect(),
        })
    }
}

/// Render a settings page: rail on the left, `body` on the right.
pub fn document(title: &str, heading: &str, blurb: &str, nav: &Nav, body: Element) -> Response {
    let rendered = dioxus_ssr::render_element(rsx! {
        head {
            meta { charset: "utf-8" }
            meta { name: "viewport", content: "width=device-width, initial-scale=1" }
            title { "{title} · FastTrackStudio" }
            style { {STYLE} }
        }
        body { class: "app",
            div { class: "app-frame",
                Rail { nav: nav.clone() }
                main { class: "sheet",
                    div { class: "sheet-head",
                        h1 { "{heading}" }
                        if !blurb.is_empty() {
                            p { class: "sub", "{blurb}" }
                        }
                    }
                    div { class: "sheet-body", {body} }
                }
            }
        }
    });
    Html(format!(
        "<!doctype html>\n<html lang=\"en\">{rendered}</html>"
    ))
    .into_response()
}

/// As [`document`], with a script in the page — the passkey pages.
pub fn document_with_script(
    title: &str,
    heading: &str,
    blurb: &str,
    nav: &Nav,
    script: &str,
    body: Element,
) -> Response {
    let rendered = dioxus_ssr::render_element(rsx! {
        head {
            meta { charset: "utf-8" }
            meta { name: "viewport", content: "width=device-width, initial-scale=1" }
            title { "{title} · FastTrackStudio" }
            style { {STYLE} }
        }
        body { class: "app",
            script { dangerous_inner_html: "{script}" }
            div { class: "app-frame",
                Rail { nav: nav.clone() }
                main { class: "sheet",
                    div { class: "sheet-head",
                        h1 { "{heading}" }
                        if !blurb.is_empty() {
                            p { class: "sub", "{blurb}" }
                        }
                    }
                    div { class: "sheet-body", {body} }
                }
            }
        }
    });
    Html(format!(
        "<!doctype html>\n<html lang=\"en\">{rendered}</html>"
    ))
    .into_response()
}

/// The account pages, in the order somebody works down them.
const ACCOUNT: [(&str, &str); 7] = [
    ("/account/profile", "Profile"),
    ("/account", "Linked accounts"),
    ("/account/passkeys", "Passkeys"),
    ("/account/two-factor", "Two-factor"),
    ("/account/phone", "Phone"),
    ("/account/sessions", "Sessions"),
    ("/account/api-keys", "API keys"),
];

#[component]
fn Rail(nav: Nav) -> Element {
    rsx! {
        nav { class: "rail", aria_label: "Settings",
            a { class: "wordmark", href: "/account/profile", "FastTrackStudio" }

            div { class: "rail-who",
                strong { "{nav.name}" }
                if nav.is_guest {
                    span { "Not saved yet" }
                } else if !nav.email.is_empty() {
                    span { "{nav.email}" }
                }
            }

            div { class: "rail-group",
                for (href, label) in ACCOUNT {
                    RailLink { href: href.to_string(), label: label.to_string(), current: nav.current.clone() }
                }
            }

            div { class: "rail-group",
                p { class: "rail-label", "Organizations" }
                for org in nav.orgs.iter() {
                    a {
                        href: "/orgs/{org.id}",
                        aria_current: if nav.current == format!("/orgs/{}", org.id) { "page" } else { "" },
                        span { "{org.name}" }
                        span { class: "role", "{org.role}" }
                    }
                }
                RailLink {
                    href: "/orgs".to_string(),
                    label: if nav.orgs.is_empty() { "Make one".to_string() } else { "All organizations".to_string() },
                    current: nav.current.clone(),
                }
            }

            if nav.is_admin {
                div { class: "rail-group",
                    p { class: "rail-label", "Server" }
                    RailLink { href: "/admin/users".to_string(), label: "Users".to_string(), current: nav.current.clone() }
                }
            }

            div { class: "rail-group rail-foot",
                RailLink { href: "/account/switch".to_string(), label: "Switch account".to_string(), current: nav.current }
                form { method: "post", action: "/account/sign-out",
                    button { r#type: "submit", class: "link", "Sign out" }
                }
            }
        }
    }
}

#[component]
fn RailLink(href: String, label: String, current: String) -> Element {
    rsx! {
        a {
            href: "{href}",
            // Exact, not a prefix: `/account` is its own page and must
            // not light up on every `/account/…` route beneath it.
            aria_current: if current == href { "page" } else { "" },
            "{label}"
        }
    }
}
