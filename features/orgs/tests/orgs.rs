//! The selection, capabilities by role, refresh and the cache.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_orgs::{Capabilities, Org, OrgId, Orgs, Role, Selection};

fn orgs() -> Orgs {
    Orgs::new(vec![
        Org::new("fts", "FastTrackStudio", Role::Admin).members(12),
        Org::new("hp", "Harbor & Pine", Role::Viewer).members(4),
        Org::new("me", "Personal", Role::Owner).personal(),
    ])
}

fn caps() -> Capabilities {
    Capabilities::new()
        .allow(&Role::Viewer, ["notes.read"])
        .allow(&Role::Member, ["notes.write"])
        .allow(&Role::Admin, ["members.invite"])
        .allow(&Role::Owner, ["org.delete"])
        .inherit()
}

#[test]
fn a_window_starts_in_the_personal_org_and_switches() {
    let mut o = orgs();
    assert_eq!(o.current().unwrap().name, "Personal");
    assert!(o.switch(Selection::One(OrgId::new("fts"))));
    assert_eq!(o.current().unwrap().name, "FastTrackStudio");
    assert!(!o.switch(Selection::One(OrgId::new("nope"))));
    assert!(o.switch(Selection::All));
    assert!(o.current().is_none());
}

#[test]
fn capabilities_follow_the_role_and_higher_roles_inherit() {
    let c = caps();
    let mut o = orgs();
    o.switch(Selection::One(OrgId::new("fts")));
    assert!(o.can(&c, "members.invite"));
    assert!(o.can(&c, "notes.write"));
    assert!(!o.can(&c, "org.delete"));
    o.switch(Selection::One(OrgId::new("hp")));
    assert!(o.can(&c, "notes.read"));
    assert!(!o.can(&c, "notes.write"));
    // In All, only what every org allows.
    o.switch(Selection::All);
    assert!(o.can(&c, "notes.read"));
    assert!(!o.can(&c, "notes.write"));
}

#[test]
fn a_grant_in_one_org_adds_to_the_role() {
    let mut c = caps();
    c.grant(&OrgId::new("hp"), "notes.write");
    let mut o = orgs();
    o.switch(Selection::One(OrgId::new("hp")));
    assert!(o.can(&c, "notes.write"));
}

#[test]
fn leaving_the_current_org_moves_the_window_home() {
    let mut o = orgs();
    o.switch(Selection::One(OrgId::new("hp")));
    o.refresh(vec![
        Org::new("fts", "FastTrackStudio", Role::Admin),
        Org::new("me", "Personal", Role::Owner).personal(),
    ]);
    assert_eq!(o.current().unwrap().name, "Personal");
}

#[test]
fn roles_parse_from_the_servers_names_and_the_cache_round_trips() {
    assert_eq!(Role::parse("ADMIN"), Role::Admin);
    assert_eq!(Role::parse("guest"), Role::Viewer);
    assert_eq!(Role::parse("billing"), Role::Other("billing".into()));
    let o = orgs();
    assert_eq!(Orgs::from_json(&o.to_json().unwrap()).unwrap(), o);
}
