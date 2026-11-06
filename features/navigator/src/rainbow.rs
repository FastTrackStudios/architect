//! The navigation rainbow (Notebook Navigator's `utils/navigationRainbow.ts`):
//! rows coloured along their siblings, a gradient from a first colour to
//! a last, where a row has no colour of its own.
//!
//! Each section (folders, tags, properties) is on or off. The scope says
//! which sibling groups get a gradient: the top level's (`root`), the
//! level below it (`child`), or every group (`all`); with the first two,
//! a row below takes its coloured ancestor's colour, as NN's inherited
//! colours do. The gradient runs through the hue, the shorter way round
//! (`transitionStyle` `hue`).

// Colour math: channels in 0..=1, hues in degrees, a few rows' counts.
#![allow(clippy::many_single_char_names, clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;

use crate::settings::{NavigatorSettings, RainbowMode, RainbowScope};
use crate::tree::{NavRow, NavRowKind};

/// `#rrggbb` (or `#rgb`) as red, green and blue in 0..1.
fn rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.trim().strip_prefix('#')?;
    let h: String = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect()
    } else {
        h.to_owned()
    };
    if h.len() != 6 {
        return None;
    }
    let p = |i: usize| {
        u8::from_str_radix(h.get(i..i + 2)?, 16)
            .ok()
            .map(|v| f64::from(v) / 255.0)
    };
    Some((p(0)?, p(2)?, p(4)?))
}

fn hsl((r, g, b): (f64, f64, f64)) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = f64::midpoint(max, min);
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < f64::EPSILON {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f64::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hex_of((h, s, l): (f64, f64, f64)) -> String {
    let c = (1.0 - 2.0f64.mul_add(l, -1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp {
        v if v < 1.0 => (c, x, 0.0),
        v if v < 2.0 => (x, c, 0.0),
        v if v < 3.0 => (0.0, c, x),
        v if v < 4.0 => (0.0, x, c),
        v if v < 5.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    // In 0..=255 by construction.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions
    )]
    let byte = |v: f64| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
}

/// The colour `t` (0..=1) of the way from `a` to `b`, through the hue the
/// shorter way.
#[must_use]
pub fn between(a: &str, b: &str, t: f64) -> Option<String> {
    let (ha, sa, la) = hsl(rgb(a)?);
    let (hb, sb, lb) = hsl(rgb(b)?);
    let mut dh = hb - ha;
    if dh > 180.0 {
        dh -= 360.0;
    } else if dh < -180.0 {
        dh += 360.0;
    }
    Some(hex_of((
        dh.mul_add(t, ha),
        (sb - sa).mul_add(t, sa),
        (lb - la).mul_add(t, la),
    )))
}

/// A row's section, and its parent's and its own ids within it.
fn place(row: &NavRow) -> Option<(&'static str, String, String)> {
    let parent = |k: &str| {
        k.rsplit_once('/')
            .map_or_else(String::new, |(p, _)| p.to_owned())
    };
    match &row.kind {
        NavRowKind::Folder { path, .. } if !path.is_empty() => {
            Some(("folder", parent(path), path.clone()))
        }
        NavRowKind::Tag { key, .. } if !crate::tags::is_virtual(key) => {
            Some(("tag", parent(key), key.clone()))
        }
        NavRowKind::Property { key, value, .. } if key != crate::properties::ROOT => {
            Some(value.as_ref().map_or_else(
                || ("property", String::new(), key.clone()),
                |v| ("property", key.clone(), format!("{key}={v}")),
            ))
        }
        _ => None,
    }
}

/// Each coloured row's colour, by its key ([`NavRow::key`]).
#[must_use]
pub fn colours(rows: &[NavRow], s: &NavigatorSettings) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if s.rainbow == RainbowMode::Off {
        return out;
    }
    let on = |section: &str| match section {
        "folder" => s.rainbow_folders,
        "tag" => s.rainbow_tags,
        _ => s.rainbow_properties,
    };
    // Sibling groups, in the rows' order, and each id's depth (its top
    // level 0) and row key.
    let mut groups: BTreeMap<(&str, String), Vec<String>> = BTreeMap::new();
    let mut depth: BTreeMap<(&str, String), usize> = BTreeMap::new();
    let mut key_of: BTreeMap<(&str, String), String> = BTreeMap::new();
    let mut parent_of: BTreeMap<(&str, String), String> = BTreeMap::new();
    for r in rows {
        let Some((section, parent, id)) = place(r) else {
            continue;
        };
        if !on(section) {
            continue;
        }
        let d = if parent.is_empty() {
            0
        } else {
            depth.get(&(section, parent.clone())).map_or(0, |d| d + 1)
        };
        depth.insert((section, id.clone()), d);
        key_of.insert((section, id.clone()), r.key.clone());
        parent_of.insert((section, id.clone()), parent.clone());
        groups.entry((section, parent)).or_default().push(id);
    }
    let wanted = |d: usize| match s.rainbow_scope {
        RainbowScope::Root => d == 0,
        RainbowScope::Child => d == 1,
        RainbowScope::All => true,
    };
    let mut own: BTreeMap<(&str, String), String> = BTreeMap::new();
    for ((section, _), ids) in &groups {
        let Some(first) = ids.first() else { continue };
        let d = depth.get(&(*section, first.clone())).copied().unwrap_or(0);
        if !wanted(d) {
            continue;
        }
        let n = ids.len();
        for (i, id) in ids.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
            let t = if n > 1 {
                i as f64 / (n - 1) as f64
            } else {
                0.0
            };
            if let Some(c) = between(&s.rainbow_first, &s.rainbow_last, t) {
                own.insert((*section, id.clone()), c);
            }
        }
    }
    for ((section, id), key) in &key_of {
        // Its own, else (root and child scopes) its nearest coloured
        // ancestor's.
        let mut at = id.clone();
        let colour = loop {
            if let Some(c) = own.get(&(*section, at.clone())) {
                break Some(c.clone());
            }
            if s.rainbow_scope == RainbowScope::All {
                break None;
            }
            match parent_of.get(&(*section, at.clone())) {
                Some(p) if !p.is_empty() => at.clone_from(p),
                _ => break None,
            }
        };
        if let Some(c) = colour {
            out.insert(key.clone(), c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gradient_runs_the_short_way_round_the_hue() {
        assert_eq!(
            between("#ef4444", "#8b5cf6", 0.0).as_deref(),
            Some("#ef4444")
        );
        assert_eq!(
            between("#ef4444", "#8b5cf6", 1.0).as_deref(),
            Some("#8b5cf6")
        );
        // Red to violet goes through magenta, not green.
        let mid = between("#ff0000", "#8000ff", 0.5).unwrap();
        let (h, _, _) = hsl(rgb(&mid).unwrap());
        assert!(h > 270.0 || h < 30.0, "{mid} {h}");
        assert_eq!(between("nope", "#fff", 0.5), None);
    }
}
