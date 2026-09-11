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
use architect_ui::components::{
    Sidebar, SidebarContent, SidebarFooter, SidebarGroup, SidebarGroupLabel, SidebarHeader,
    SidebarMenu, SidebarMenuButton, SidebarMenuButtonRenderAs, SidebarMenuItem, SidebarProvider,
};

use crate::assets;
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

/// Render a settings page: rail on the left, `body` on the right.
pub fn document(title: &str, heading: &str, blurb: &str, nav: &Nav, body: Element) -> Response {
    shell(title, heading, blurb, nav, None, body)
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
    shell(title, heading, blurb, nav, Some(script), body)
}

/// The one page shape these settings screens share.
///
/// The stylesheet is linked rather than inlined: it carries the whole
/// design system, and re-sending it on every navigation within a
/// settings app is exactly the cost the rail is meant to avoid. See
/// [`crate::assets`] for why that is safe to cache.
fn shell(
    title: &str,
    heading: &str,
    blurb: &str,
    nav: &Nav,
    script: Option<&str>,
    body: Element,
) -> Response {
    let stylesheet = assets::stylesheet_path();
    let script = script.unwrap_or_default().to_owned();
    let rendered = dioxus_ssr::render_element(rsx! {
        head {
            meta { charset: "utf-8" }
            meta { name: "viewport", content: "width=device-width, initial-scale=1" }
            title { "{title} · FastTrackStudio" }
            link { rel: "stylesheet", href: "{stylesheet}" }
        }
        body { class: "bg-background text-foreground antialiased",
            if !script.is_empty() {
                script { dangerous_inner_html: "{script}" }
            }
            SidebarProvider { class: "flex-col md:flex-row",
                Rail { nav: nav.clone() }
                // `main` is the working surface, and the tests scope to
                // it — the rail repeats organizations and "Sign out", so
                // a page-wide lookup matches twice.
                main { class: "min-w-0 flex-1 px-5 py-10 sm:px-8 lg:px-12",
                    div { class: "mx-auto flex w-full max-w-3xl flex-col gap-8",
                        header { class: "flex flex-col gap-1",
                            h1 { class: "text-2xl font-semibold tracking-tight", "{heading}" }
                            if !blurb.is_empty() {
                                p { class: "text-sm text-muted-foreground", "{blurb}" }
                            }
                        }
                        div { class: "flex flex-col gap-6", {body} }
                    }
                }
            }
        }
    });
    Html(format!(
        "<!doctype html>\n<html lang=\"en\">{rendered}</html>"
    ))
    .into_response()
}

#[component]
fn Rail(nav: Nav) -> Element {
    rsx! {
        // Wide: a column down the left. Narrow: a strip that scrolls
        // sideways above the page — the same links, none of them lost,
        // and no script to make it happen.
        Sidebar {
            class: "w-full shrink-0 md:min-h-screen md:w-64 md:border-r",
            SidebarHeader { class: "hidden gap-4 p-4 md:flex",
                a {
                    class: "text-xs font-semibold uppercase tracking-[0.18em] text-sidebar-foreground",
                    href: "/account/profile",
                    "FastTrackStudio"
                }
                div { class: "flex flex-col",
                    strong { class: "truncate text-sm font-medium", "{nav.name}" }
                    if nav.is_guest {
                        span { class: "text-xs text-muted-foreground", "Not saved yet" }
                    } else if !nav.email.is_empty() {
                        span { class: "truncate text-xs text-muted-foreground", "{nav.email}" }
                    }
                }
            }

            SidebarContent { class: "flex-row gap-1 overflow-x-auto px-2 md:flex-col md:gap-2 md:overflow-x-hidden",
                SidebarGroup { class: "flex-row md:flex-col",
                    SidebarMenu { class: "flex-row md:flex-col",
                        for (href, label) in ACCOUNT {
                            RailLink {
                                href: href.to_string(),
                                label: label.to_string(),
                                current: nav.current.clone(),
                            }
                        }
                    }
                }

                SidebarGroup { class: "flex-row md:flex-col",
                    SidebarGroupLabel { class: "hidden md:flex", "Organizations" }
                    SidebarMenu { class: "flex-row md:flex-col",
                        for org in nav.orgs.iter() {
                            SidebarMenuItem {
                                SidebarMenuButton {
                                    render_as: SidebarMenuButtonRenderAs::Anchor {
                                        href: format!("/orgs/{}", org.id),
                                    },
                                    is_active: nav.current == format!("/orgs/{}", org.id),
                                    span { class: "truncate", "{org.name}" }
                                    span { class: "ml-auto text-xs text-muted-foreground", "{org.role}" }
                                }
                            }
                        }
                        RailLink {
                            href: "/orgs".to_string(),
                            label: if nav.orgs.is_empty() { "Make one".to_string() } else { "All organizations".to_string() },
                            current: nav.current.clone(),
                        }
                    }
                }

                SidebarGroup { class: "ml-auto flex-row md:hidden",
                    SidebarMenu { class: "flex-row",
                        RailLink {
                            href: "/account/switch".to_string(),
                            label: "Switch account".to_string(),
                            current: nav.current.clone(),
                        }
                        SidebarMenuItem {
                            form { method: "post", action: "/account/sign-out",
                                button {
                                    class: "whitespace-nowrap rounded-lg px-3 py-2 text-sm text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                                    r#type: "submit",
                                    "Sign out"
                                }
                            }
                        }
                    }
                }

                if nav.is_admin {
                    SidebarGroup { class: "flex-row md:flex-col",
                        SidebarGroupLabel { class: "hidden md:flex", "Server" }
                        SidebarMenu { class: "flex-row md:flex-col",
                            RailLink {
                                href: "/admin/users".to_string(),
                                label: "Users".to_string(),
                                current: nav.current.clone(),
                            }
                        }
                    }
                }
            }

            SidebarFooter { class: "hidden px-2 pb-4 md:flex",
                SidebarMenu {
                    RailLink {
                        href: "/account/switch".to_string(),
                        label: "Switch account".to_string(),
                        current: nav.current,
                    }
                    SidebarMenuItem {
                        form { method: "post", action: "/account/sign-out",
                            button {
                                class: "w-full rounded-lg px-3 py-2 text-left text-sm text-muted-foreground underline-offset-4 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground hover:underline",
                                r#type: "submit",
                                "Sign out"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RailLink(href: String, label: String, current: String) -> Element {
    rsx! {
        SidebarMenuItem {
            SidebarMenuButton {
                render_as: SidebarMenuButtonRenderAs::Anchor { href: href.clone() },
                // Exact, not a prefix: `/account` is its own page and must
                // not light up on every `/account/…` route beneath it.
                is_active: current == href,
                class: "whitespace-nowrap",
                "{label}"
            }
        }
    }
}
