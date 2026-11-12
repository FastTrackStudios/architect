//! The site's sheet (`assets/site.css`) as old browsers read it:
//! Safari 16.0 and 16.1 have no `color-mix()`, so every declaration of
//! the player's and the playlist's that mixes a colour comes after a
//! plain one of the same property (the older browser keeps the plain
//! one); and the small icon buttons a finger taps are 44 px square.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

const SHEET: &str = include_str!("../assets/site.css");

/// The sheet from the player's part on (the share pages' own rules).
fn players_part() -> &'static str {
    let at = SHEET
        .find("/* The player (#511)")
        .expect("the player's part");
    &SHEET[at..]
}

/// Each rule's body (`{ … }`, nested at-rules flattened), with its selector.
fn rules(css: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    let mut selector_from = 0_usize;
    for (i, c) in css.char_indices() {
        match c {
            '{' => {
                if depth == 0 || !css[selector_from..i].trim_start().starts_with('@') {
                    start = i + 1;
                }
                depth += 1;
                if depth == 1 || !css[selector_from..i].trim().is_empty() {
                    out.push((css[selector_from..i].trim().to_owned(), String::new()));
                }
                selector_from = i + 1;
            }
            '}' => {
                if let Some(last) = out.last_mut() {
                    if last.1.is_empty() {
                        css[start..i].clone_into(&mut last.1);
                    }
                }
                depth = depth.saturating_sub(1);
                selector_from = i + 1;
            }
            _ => {}
        }
    }
    out
}

#[test]
fn every_mixed_colour_has_a_plain_one_before_it() {
    let mut missing = Vec::new();
    for (selector, body) in rules(players_part()) {
        let decls: Vec<(String, String)> = body
            .split(';')
            .filter_map(|d| d.split_once(':'))
            .map(|(p, v)| (p.trim().to_owned(), v.trim().to_owned()))
            .collect();
        for (i, (prop, value)) in decls.iter().enumerate() {
            if value.contains("color-mix(") {
                let plain = decls[..i]
                    .iter()
                    .any(|(p, v)| p == prop && !v.contains("color-mix("));
                if !plain {
                    missing.push(format!("{selector} {{ {prop}: {value} }}"));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "no plain value first (Safari 16.0/16.1):\n{}",
        missing.join("\n")
    );
}

#[test]
fn the_icon_buttons_a_finger_taps_are_44_px() {
    let part = players_part();
    for selector in [
        ".site-now-lyrics, .site-now-page {",
        "[data-playlist] [data-track-more] {",
        ".site-player .site-player-opt, .site-now .site-player-opt {",
    ] {
        let at = part
            .find(selector)
            .unwrap_or_else(|| panic!("{selector} is in the sheet"));
        let body = &part[at..at + part[at..].find('}').unwrap()];
        assert!(
            body.contains("width: 44px") && body.contains("height: 44px"),
            "{selector}: {body}"
        );
    }
    // No narrower override on a phone.
    assert!(!part.contains("[data-track-open] { width: 36px; }"));
}

/// A playlist's block keeps `overflow: auto` (the box a block is drawn in,
/// inline and `!important`), so nothing of a row may reach past it: a
/// negative margin on the row's menu button scrolled every phone's list
/// sideways (a bar under the list, white in a dark theme).
#[test]
fn nothing_of_a_row_reaches_past_its_block() {
    for (selector, body) in rules(players_part()) {
        if !selector.contains("[data-playlist]") {
            continue;
        }
        for decl in body.split(';') {
            let Some((prop, value)) = decl.split_once(':') else {
                continue;
            };
            let prop = prop.trim();
            let horizontal = matches!(prop, "margin-right" | "margin-left" | "margin" | "margin-inline" | "margin-inline-end");
            assert!(
                !(horizontal && value.contains('-')),
                "{selector} {{ {prop}:{value} }}"
            );
        }
    }
}
