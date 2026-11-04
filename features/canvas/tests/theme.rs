//! `ImGui` windows follow the app's theme: each style colour from its
//! token (`Style::from_theme`, the table on `Col`), restyled on every
//! switch (`Session::set_theme`), a script's `PushStyleColor` over it, and
//! the script reading the theme itself (`theme.color`). The window's
//! shapes are what the host paints, so checking their colours is the
//! headless pixel check.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use architect::action::{ActionCaller, ActionList};
use architect_canvas::imgui::{Col, Input, Style};
use architect_canvas::luau::Session;
use architect_canvas::{Color, Shape, Size};
use architect_scripting::luau::Runtime;
use architect_theme::{Appearance, LayerKind, Resolved, Rgba, Scheme, ThemeStore};
use futures::executor::block_on;
use serde_json::Value;

fn packed(r: &Resolved, name: &str) -> Color {
    Color(r.color(name).packed())
}

fn mixed(r: &Resolved, a: &str, b: &str, t: f32) -> Color {
    Color(r.color(a).mix(r.color(b), t).packed())
}

fn store(scheme: Scheme) -> ThemeStore {
    let store = ThemeStore::default();
    store.set_system(scheme);
    store
}

#[test]
fn the_style_maps_a_dark_and_a_light_theme_token_by_token() {
    for scheme in [Scheme::Dark, Scheme::Light] {
        let r = store(scheme).current();
        let s = Style::from_theme(&r);
        let want = [
            (Col::Text, packed(&r, "ink")),
            (Col::TextDisabled, packed(&r, "ink-3")),
            (Col::WindowBg, packed(&r, "sheet")),
            (Col::FrameBg, packed(&r, "pill")),
            (Col::FrameBgHovered, mixed(&r, "pill", "ink", 0.08)),
            (Col::FrameBgActive, mixed(&r, "pill", "ink", 0.16)),
            (Col::Button, mixed(&r, "pill", "brand", 0.18)),
            (Col::ButtonHovered, mixed(&r, "pill", "brand", 0.32)),
            (Col::ButtonActive, mixed(&r, "pill", "brand", 0.5)),
            (Col::SliderGrab, packed(&r, "brand")),
            (Col::SliderGrabActive, mixed(&r, "brand", "ink", 0.3)),
            (Col::CheckMark, packed(&r, "brand")),
            (Col::Separator, packed(&r, "rule")),
            (Col::PlotHistogram, packed(&r, "chart-1")),
        ];
        assert_eq!(want.len(), Col::ALL.len(), "every colour is checked");
        for (col, colour) in want {
            assert_eq!(s.color(col), colour, "{scheme:?}: {}", col.name());
        }
    }
    // Light ground under dark text, and the other way round.
    let dark = Style::from_theme(&store(Scheme::Dark).current());
    let light = Style::from_theme(&store(Scheme::Light).current());
    let lum = |c: Color| {
        let [r, g, b, _] = c.channels();
        Rgba::rgb(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        )
        .luminance()
    };
    assert!(lum(dark.window_bg) < lum(dark.text));
    assert!(lum(light.window_bg) > lum(light.text));
}

#[test]
fn a_themes_imgui_token_wins_over_the_derived_colour() {
    let store = ThemeStore::default();
    store.set_token(LayerKind::Person, "imgui.button", "#123456");
    store.set_token(
        LayerKind::Person,
        "imgui.window-bg",
        "mix($sheet, $danger, 0.5)",
    );
    let r = store.current();
    let s = Style::from_theme(&r);
    assert_eq!(s.button, Color(0x1234_56FF));
    assert_eq!(s.window_bg, packed(&r, "imgui.window-bg"));
    assert_eq!(Col::from_index(21), Some(Col::Button));
    assert_eq!(Col::from_index(3), None);
}

const SCRIPT: &str = r"
local ImGui = require 'imgui' '0.10'
local ctx = ImGui.CreateContext('t')
local function loop()
  local visible = ImGui.Begin(ctx, 'W')
  if visible then
    ImGui.TextColored(ctx, theme.color('danger'), 'Danger, from the theme')
    ImGui.PushStyleColor(ctx, ImGui.Col_Button, 0x00FF00FF)
    ImGui.Button(ctx, 'Green')
    ImGui.PopStyleColor(ctx)
    ImGui.Button(ctx, 'Themed')
    ImGui.End(ctx)
  end
  defer(loop)
end
loop()
";

fn input() -> Input {
    Input {
        sizes: [("t/W".to_owned(), Size::new(300.0, 200.0))].into(),
        ..Input::default()
    }
}

/// The window's ground, the danger text's colour, and the two buttons'.
fn drawn(s: &Session) -> (Color, Color, Vec<Color>) {
    let w = s.output().window("t/W").expect("the window").clone();
    let shapes = w.draw.shapes();
    let Shape::Rect { color: ground, .. } = &shapes[0] else {
        panic!("the ground is drawn first: {shapes:?}")
    };
    let text = shapes
        .iter()
        .find_map(|s| match s {
            Shape::Text { text, color, .. } if text.starts_with("Danger") => Some(*color),
            _ => None,
        })
        .unwrap();
    let buttons = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Rect {
                color, fill: true, ..
            } if *color != *ground => Some(*color),
            _ => None,
        })
        .collect();
    (*ground, text, buttons)
}

#[test]
fn a_window_follows_a_theme_switch_and_a_push_holds_over_it() {
    let store = store(Scheme::Dark);
    let (report, session) = block_on(Session::start_themed(
        &Runtime::new(ActionList::new()),
        &ActionCaller::app(),
        SCRIPT,
        Value::Null,
        "T",
        &store.current(),
    ));
    report.result.unwrap();
    let mut session = session.unwrap();
    block_on(session.frame(input())).result.unwrap();
    let dark = store.current();
    let (ground, text, buttons) = drawn(&session);
    assert_eq!(ground, packed(&dark, "sheet"));
    assert_eq!(text, packed(&dark, "danger"));
    assert_eq!(
        buttons,
        vec![Color(0x00FF_00FF), mixed(&dark, "pill", "brand", 0.18)],
        "the pushed colour, then the theme's again after the pop"
    );

    // THEME_SET's effect: the light half shows. The host hands it over
    // and draws a frame; everything follows.
    store.update(LayerKind::Pick, |l| l.appearance = Some(Appearance::Light));
    let light = store.current();
    assert!(session.set_theme(&light));
    assert!(
        !session.set_theme(&light),
        "the same theme again is no change"
    );
    block_on(session.frame(input())).result.unwrap();
    let (ground, text, buttons) = drawn(&session);
    assert_ne!(ground, packed(&dark, "sheet"));
    assert_eq!(ground, packed(&light, "sheet"));
    assert_eq!(text, packed(&light, "danger"));
    assert_eq!(
        buttons,
        vec![Color(0x00FF_00FF), mixed(&light, "pill", "brand", 0.18)]
    );
}
