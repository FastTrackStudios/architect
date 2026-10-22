//! Scroll gesture patterns and per-mode (or per-surface) binding tables.

use crate::config::{ConfigError, KeymapConfig, parse_scroll_pattern};
use crate::context::{InputContext, WhenExpr};
use crate::event::ScrollEvent;
use crate::key::Modifiers;

/// Which way a scroll pattern listens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxis {
    Any,
    Horizontal,
    Vertical,
}

/// A scroll gesture a binding matches: an axis and the held modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollPattern {
    pub axis: ScrollAxis,
    pub modifiers: Modifiers,
}

impl ScrollPattern {
    #[must_use]
    pub const fn new(axis: ScrollAxis, modifiers: Modifiers) -> Self {
        Self { axis, modifiers }
    }

    /// Whether `event` is this gesture. A gesture's axis is the larger of
    /// its two deltas (a tie reads as vertical).
    #[must_use]
    pub fn matches(&self, event: &ScrollEvent) -> bool {
        if self.modifiers != event.modifiers {
            return false;
        }
        match self.axis {
            ScrollAxis::Any => true,
            ScrollAxis::Horizontal => event.delta_x.abs() > event.delta_y.abs(),
            ScrollAxis::Vertical => event.delta_y.abs() >= event.delta_x.abs(),
        }
    }
}

/// The scroll bindings of one *surface* of a config.
///
/// A surface is a place gestures mean something (an arrange view, an
/// editor, a mixer); it is a key of [`KeymapConfig::scroll`], which the
/// processor otherwise reads as a mode name. A surface reads the shared
/// config directly instead of hand-rolling its own, so one place resolves
/// a wheel event to an action id.
///
/// A surface not in the config gives an empty table: "nothing bound
/// here", not an error.
pub fn table_for_surface<A: From<String> + Clone>(
    config: &KeymapConfig,
    surface: &str,
) -> Result<ScrollBindingTable<A>, ConfigError> {
    let mut table = ScrollBindingTable::new();
    for (pattern, action) in config.scroll.get(surface).into_iter().flatten() {
        table.insert(
            parse_scroll_pattern(pattern)?,
            WhenExpr::True,
            A::from(action.clone()),
        );
    }
    Ok(table)
}

/// One mode's scroll bindings, in order: the first match wins.
#[derive(Debug, Clone)]
pub struct ScrollBindingTable<A = String> {
    bindings: Vec<(ScrollPattern, WhenExpr, A)>,
}

impl<A> Default for ScrollBindingTable<A> {
    fn default() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }
}

impl<A: Clone> ScrollBindingTable<A> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, pattern: ScrollPattern, when: WhenExpr, action: A) {
        self.bindings.push((pattern, when, action));
    }

    /// The raw entries, in match order.
    #[must_use]
    pub fn bindings(&self) -> &[(ScrollPattern, WhenExpr, A)] {
        &self.bindings
    }

    /// The action of the first entry matching `event` whose clause holds.
    #[must_use]
    pub fn match_event(&self, event: &ScrollEvent, ctx: &InputContext) -> Option<A> {
        self.bindings
            .iter()
            .find(|(pattern, when, _)| pattern.matches(event) && when.evaluate(ctx))
            .map(|(_, _, action)| action.clone())
    }
}

#[cfg(test)]
mod surface_tests {
    use super::*;

    /// The scroll scheme studio ships (REAPER's own): plain wheel scrolls,
    /// Shift goes sideways, Alt zooms; the editor surface adds one gesture.
    const SCHEME: &str = r#"{
        "scroll": {
            "normal": {
                "ScrollY": "view.vscroll",
                "ScrollX": "view.hscroll",
                "Shift+Scroll": "view.hscroll",
                "Alt+Scroll": "view.zoom_v",
                "Alt+Shift+Scroll": "view.zoom_h",
                "Ctrl+Alt+Scroll": "view.zoom_both"
            },
            "editor": {
                "ScrollY": "view.vscroll",
                "ScrollX": "view.hscroll",
                "Shift+Scroll": "view.hscroll",
                "Ctrl+Shift+Scroll": "edit.nudge_time"
            }
        }
    }"#;

    fn mods(ctrl: bool, alt: bool, shift: bool) -> Modifiers {
        Modifiers {
            ctrl,
            alt,
            shift,
            ..Modifiers::NONE
        }
    }

    /// What the gesture resolves to on `surface`.
    fn resolve(surface: &str, delta_x: f64, delta_y: f64, modifiers: Modifiers) -> Option<String> {
        let cfg = KeymapConfig::from_json_str(SCHEME).expect("scheme parses");
        let table: ScrollBindingTable = table_for_surface(&cfg, surface).expect("surface table");
        let event = ScrollEvent {
            delta_x,
            delta_y,
            modifiers,
        };
        table.match_event(&event, &InputContext::new())
    }

    #[test]
    fn the_shared_scheme_is_the_reaper_scheme() {
        let cases = [
            (mods(false, false, false), "view.vscroll"),
            (mods(false, false, true), "view.hscroll"),
            (mods(false, true, false), "view.zoom_v"),
            (mods(false, true, true), "view.zoom_h"),
            (mods(true, true, false), "view.zoom_both"),
        ];
        for (m, want) in cases {
            assert_eq!(
                resolve("normal", 0.0, 10.0, m).as_deref(),
                Some(want),
                "{m:?}"
            );
        }
    }

    #[test]
    fn a_trackpad_swipe_scrolls_sideways_without_a_modifier() {
        assert_eq!(
            resolve("normal", 10.0, 0.0, mods(false, false, false)).as_deref(),
            Some("view.hscroll")
        );
    }

    #[test]
    fn a_surface_adds_its_own_gesture_without_changing_the_shared_ones() {
        assert_eq!(
            resolve("editor", 0.0, 10.0, mods(true, false, true)).as_deref(),
            Some("edit.nudge_time")
        );
        assert_eq!(resolve("normal", 0.0, 10.0, mods(true, false, true)), None);
    }

    #[test]
    fn an_unknown_surface_is_empty_rather_than_an_error() {
        let cfg = KeymapConfig::from_json_str(SCHEME).expect("scheme parses");
        let table: ScrollBindingTable = table_for_surface(&cfg, "mixer").expect("no error");
        assert!(table.bindings().is_empty());
    }
}
