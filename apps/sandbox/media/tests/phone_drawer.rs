//! A phone's left drawer in the Architect app (#383): opened by the
//! sidebar's command, it holds the navigator, and a note pressed in it
//! opens in the page and puts the drawer away.

#![allow(clippy::panic)]

#[test]
fn a_note_pressed_in_a_phones_drawer_opens() {
    let app = architect_sandbox_media::architect_app();
    let source = r#"id phone-drawer
title "phone drawer"
view {width 402, height 874}
steps (
    {id vault, do @run{id LAB_GO, args {tab vault}}, done {check ({id LAB_STATE, has {tab vault}}), timeout 5}}
    {id open, do @run{id COMMANDS_RUN, args {id SHELL_SIDEBAR_TOGGLE}}, done {check ({id LAB_STATE, has {drawer true}}), timeout 5}}
    {id navigator, target @css"[data-drawer-page] .nv-file", done {timeout 5}}
    {id press, target @css"[data-drawer-page] .nv-file", do @click{}, done {check ({id APP_STATUS, has {workspace {focused_view "vault.note"}}}), timeout 5}}
    {id away, done {check ({id LAB_STATE, has {drawer false}}), timeout 5}}
)
"#;
    let scenario = architect_guide::load(source).unwrap_or_else(|e| panic!("{e}"));
    if let Err(e) = architect_guide::run::run(&app, &scenario, None) {
        panic!("{e}");
    }
}
