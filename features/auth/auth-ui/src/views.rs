//! The markup for the organization pages.
//!
//! Separated from the handlers so that file reads as "what happens"
//! and this one as "what it looks like". The row types are the seam:
//! a handler reduces the engine's types to exactly what a page shows,
//! which is what keeps the components free of `Option` juggling.

use dioxus::prelude::*;
use uuid::Uuid;

use crate::page::Flash;
use crate::profile::FlashLine;

/// The roles a person can be given from these pages.
///
/// Not every role the engine will honour — an organization may define
/// its own through `create_organization_role` — but the three the
/// default permission matrix knows, which is what a dropdown can
/// meaningfully offer.
pub(crate) const ROLES: [(&str, &str); 3] = [
    ("owner", "Owner — full control, including deleting"),
    ("admin", "Admin — manage members, teams and invitations"),
    ("member", "Member — read access"),
];

#[derive(Clone, PartialEq, Eq)]
pub struct OrgRow {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub role: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct MemberRow {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct TeamRow {
    pub id: Uuid,
    pub name: String,
    pub members: Vec<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct InvitationRow {
    pub id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub expires: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LinkRow {
    pub id: Uuid,
    pub label: String,
    pub role: String,
    pub used: i32,
    pub remaining: String,
    pub usable: bool,
}

#[component]
pub fn OrgsView(rows: Vec<OrgRow>, flash: Option<Flash>) -> Element {
    rsx! {
        h1 { "Organizations" }
        p { class: "sub", "The organizations this account belongs to." }
        FlashLine { flash }

        if rows.is_empty() {
            p { class: "hint",
                "You are not in any organization yet. Make one below, or follow an invite link somebody sends you."
            }
        }
        ul { class: "orgs",
            for row in rows.iter() {
                li { key: "{row.id}", class: "org",
                    a { href: "/orgs/{row.id}",
                        strong { "{row.name}" }
                        span { class: "handle", "/{row.slug}" }
                    }
                    span { class: "tag", "{row.role}" }
                }
            }
        }

        h2 { "New organization" }
        form { method: "post", action: "/orgs", class: "stack",
            label { r#for: "org-name", "Name" }
            input { id: "org-name", name: "name", required: true, placeholder: "Acme Records" }

            label { r#for: "org-slug", "URL slug" }
            input { id: "org-slug", name: "slug", placeholder: "acme-records" }
            p { class: "hint", "Leave blank to build one from the name." }

            button { r#type: "submit", "Create organization" }
        }

        p { class: "alt",
            a { href: "/account/profile", "Profile" }
            " · "
            a { href: "/account", "Linked accounts" }
            " · "
            a { href: "/account/passkeys", "Passkeys" }
            " · "
            a { href: "/account/two-factor", "Two-factor" }
            " · "
            a { href: "/account/phone", "Phone" }
            " · "
            a { href: "/account/sessions", "Sessions" }
            " · "
            a { href: "/account/api-keys", "API keys" }
            " · "
            a { href: "/account/switch", "Accounts" }
            " · "
            a { href: "/orgs", "Organizations" }
        }
    }
}

#[component]
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub fn OrgView(
    id: Uuid,
    name: String,
    slug: String,
    logo: String,
    my_role: String,
    is_owner: bool,
    can_invite: bool,
    members: Vec<MemberRow>,
    teams: Vec<TeamRow>,
    invitations: Vec<InvitationRow>,
    links: Vec<LinkRow>,
    minted: Option<String>,
    flash: Option<Flash>,
) -> Element {
    rsx! {
        h1 { "{name}" }
        p { class: "sub", "/{slug} · you are {my_role}" }
        FlashLine { flash }

        // Shown once, right after minting. There is no second chance:
        // only the hash is stored, so if this is not copied now the
        // link has to be made again.
        if let Some(url) = minted {
            div { class: "minted",
                p { class: "ok", role: "status", "Copy this now — it is not shown again." }
                input { class: "mono", readonly: true, value: "{url}", "aria-label": "Invite URL" }
            }
        }

        h2 { "Members" }
        table { class: "grid",
            thead {
                tr {
                    th { "Person" }
                    th { "Email" }
                    th { "Role" }
                    th { }
                }
            }
            tbody {
                for member in members.iter() {
                    tr { key: "{member.user_id}",
                        td { "{member.name}" }
                        td { class: "mono", "{member.email}" }
                        td {
                            if can_invite {
                                form { method: "post", action: "/orgs/{id}/members/role", class: "inline",
                                    input { r#type: "hidden", name: "user_id", value: "{member.user_id}" }
                                    select { name: "role", "aria-label": "Role for {member.name}",
                                        for (value, label) in ROLES {
                                            option { value: "{value}", selected: member.role == value, "{label}" }
                                        }
                                    }
                                    button { r#type: "submit", class: "link", "Set" }
                                }
                            } else {
                                span { class: "tag", "{member.role}" }
                            }
                        }
                        td {
                            if can_invite {
                                form { method: "post", action: "/orgs/{id}/members/remove", class: "inline",
                                    input { r#type: "hidden", name: "user_id", value: "{member.user_id}" }
                                    button { r#type: "submit", class: "link", "Remove" }
                                }
                            }
                        }
                    }
                }
            }
        }

        h2 { "Teams" }
        if teams.is_empty() {
            p { class: "hint", "No teams yet. Teams group people inside the organization." }
        }
        ul { class: "teams",
            for team in teams.iter() {
                li { key: "{team.id}", class: "team",
                    div { class: "team-head",
                        strong { "{team.name}" }
                        if can_invite {
                            form { method: "post", action: "/orgs/{id}/teams/delete", class: "inline",
                                input { r#type: "hidden", name: "team_id", value: "{team.id}" }
                                button { r#type: "submit", class: "link", "Delete team" }
                            }
                        }
                    }
                    if team.members.is_empty() {
                        p { class: "hint", "Nobody in this team yet." }
                    } else {
                        p { class: "handle", "{team.members.join(\", \")}" }
                    }
                    if can_invite {
                        form { method: "post", action: "/orgs/{id}/teams/members", class: "inline",
                            input { r#type: "hidden", name: "team_id", value: "{team.id}" }
                            select { name: "user_id", "aria-label": "Add to {team.name}",
                                for member in members.iter() {
                                    option { value: "{member.user_id}", "{member.name}" }
                                }
                            }
                            button { r#type: "submit", class: "link", "Add" }
                        }
                    }
                }
            }
        }
        if can_invite {
            form { method: "post", action: "/orgs/{id}/teams", class: "stack",
                label { r#for: "team-name", "New team" }
                input { id: "team-name", name: "name", required: true, placeholder: "Mastering" }
                button { r#type: "submit", "Create team" }
            }
        }

        if can_invite {
            h2 { "Invite one person" }
            p { class: "hint",
                "Creates a link addressed to one person. It can be accepted once."
            }
            form { method: "post", action: "/orgs/{id}/invitations", class: "stack",
                label { r#for: "invite-email", "Email address" }
                input { id: "invite-email", name: "email", r#type: "email", required: true }
                label { r#for: "invite-role", "Role" }
                select { id: "invite-role", name: "role",
                    for (value, label) in ROLES {
                        option { value: "{value}", selected: value == "member", "{label}" }
                    }
                }
                button { r#type: "submit", "Create invitation" }
            }

            if !invitations.is_empty() {
                table { class: "grid",
                    thead {
                        tr {
                            th { "Invited" }
                            th { "Role" }
                            th { "Status" }
                            th { "Expires" }
                            th { }
                        }
                    }
                    tbody {
                        for invitation in invitations.iter() {
                            tr { key: "{invitation.id}",
                                td { class: "mono", "{invitation.email}" }
                                td { "{invitation.role}" }
                                td { span { class: "tag", "{invitation.status}" } }
                                td { class: "mono", "{invitation.expires}" }
                                td {
                                    if invitation.status == "pending" {
                                        form { method: "post", action: "/orgs/{id}/invitations/cancel", class: "inline",
                                            input { r#type: "hidden", name: "id", value: "{invitation.id}" }
                                            button { r#type: "submit", class: "link", "Cancel" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            h2 { "Invite links" }
            p { class: "hint",
                "One URL that anyone can follow to join. Give it a limit if you are pasting it somewhere public."
            }
            form { method: "post", action: "/orgs/{id}/links", class: "stack",
                label { r#for: "link-label", "Label" }
                input { id: "link-label", name: "label", placeholder: "Launch week" }

                label { r#for: "link-role", "Role for everyone who joins" }
                select { id: "link-role", name: "role",
                    for (value, label) in ROLES {
                        option { value: "{value}", selected: value == "member", "{label}" }
                    }
                }

                label { r#for: "link-max", "Maximum uses" }
                input { id: "link-max", name: "max_uses", r#type: "number", min: "1", placeholder: "unlimited" }

                label { r#for: "link-days", "Expires after (days)" }
                input { id: "link-days", name: "expires_days", r#type: "number", min: "1", placeholder: "never" }

                button { r#type: "submit", "Create invite link" }
            }

            if !links.is_empty() {
                table { class: "grid",
                    thead {
                        tr {
                            th { "Label" }
                            th { "Role" }
                            th { "Used" }
                            th { "Left" }
                            th { }
                        }
                    }
                    tbody {
                        for link in links.iter() {
                            tr { key: "{link.id}",
                                td {
                                    "{link.label}"
                                    if !link.usable {
                                        span { class: "tag", "closed" }
                                    }
                                }
                                td { "{link.role}" }
                                td { class: "mono", "{link.used}" }
                                td { class: "mono", "{link.remaining}" }
                                td {
                                    if link.usable {
                                        form { method: "post", action: "/orgs/{id}/links/revoke", class: "inline",
                                            input { r#type: "hidden", name: "id", value: "{link.id}" }
                                            button { r#type: "submit", class: "link", "Revoke" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        h2 { "Settings" }
        if is_owner {
            form { method: "post", action: "/orgs/{id}", class: "stack",
                label { r#for: "settings-name", "Name" }
                input { id: "settings-name", name: "name", value: "{name}", required: true }

                label { r#for: "settings-slug", "URL slug" }
                input { id: "settings-slug", name: "slug", value: "{slug}", required: true }

                label { r#for: "settings-logo", "Logo URL" }
                input { id: "settings-logo", name: "logo", value: "{logo}", r#type: "url", placeholder: "https://…" }

                button { r#type: "submit", "Save" }
            }
        }

        p { class: "alt",
            form { method: "post", action: "/orgs/{id}/leave", class: "inline",
                button { r#type: "submit", class: "link", "Leave this organization" }
            }
            if is_owner {
                " · "
                form { method: "post", action: "/orgs/{id}/delete", class: "inline",
                    button { r#type: "submit", class: "link danger", "Delete organization" }
                }
            }
            " · "
            a { href: "/orgs", "All organizations" }
        }
    }
}

#[component]
pub fn InvitationView(
    id: Uuid,
    token: String,
    organization: String,
    role: String,
    email: String,
    signed_in: bool,
    error: Option<String>,
) -> Element {
    rsx! {
        h1 { "Join {organization}" }
        p { class: "sub", "Invited as {role}, to {email}." }
        if let Some(message) = error {
            p { class: "error", role: "alert", "{message}" }
        }
        if signed_in {
            form { method: "post", action: "/invite/{id}/accept", class: "stack",
                input { r#type: "hidden", name: "token", value: "{token}" }
                button { r#type: "submit", "Accept invitation" }
            }
        } else {
            p { class: "hint",
                "Sign in or create an account with this address, and you will come straight back here."
            }
            form { method: "post", action: "/invite/{id}/accept", class: "stack",
                input { r#type: "hidden", name: "token", value: "{token}" }
                button { r#type: "submit", "Sign in and accept" }
            }
        }
        p { class: "alt",
            form { method: "post", action: "/invite/{id}/reject", class: "inline",
                input { r#type: "hidden", name: "token", value: "{token}" }
                button { r#type: "submit", class: "link", "No thanks" }
            }
        }
    }
}

#[component]
pub fn JoinView(
    token: String,
    organization: String,
    role: String,
    signed_in: bool,
    error: Option<String>,
) -> Element {
    rsx! {
        h1 { "Join {organization}" }
        p { class: "sub", "You will join as {role}." }
        if let Some(message) = error {
            p { class: "error", role: "alert", "{message}" }
        }
        if !signed_in {
            p { class: "hint",
                "Sign in or create an account, and you will come straight back here."
            }
        }
        form { method: "post", action: "/join", class: "stack",
            input { r#type: "hidden", name: "token", value: "{token}" }
            button { r#type: "submit",
                if signed_in { "Join {organization}" } else { "Sign in and join" }
            }
        }
    }
}

/// What a bad, spent, revoked or expired token gets.
///
/// One page for all four, and it names none of them: telling somebody
/// holding a guessed token that it was "expired" rather than "wrong"
/// confirms it once existed.
#[component]
pub fn DeadEnd() -> Element {
    rsx! {
        h1 { "This link is not usable" }
        p { class: "sub",
            "It may have been used up, turned off, or it may never have been valid. Ask whoever sent it for a new one."
        }
        p { class: "alt", a { href: "/orgs", "Your organizations" } }
    }
}

#[component]
pub fn Declined() -> Element {
    rsx! {
        h1 { "Invitation declined" }
        p { class: "sub", "Nothing has been shared with you, and the invitation is closed." }
    }
}
