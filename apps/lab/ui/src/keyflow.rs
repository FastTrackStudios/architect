//! Keyflow integration exercise in the lab's workspace.
use architect_workspace::HostData;
use architect_workspace::paint::peniko::Color;
use dioxus::prelude::*;
use keyflow::engraver::renderer::ChartTheme;
use keyflow::syntax::highlighting::Theme as HighlightTheme;
use keyflow_ui::native_chart_editor::NativeChartEditor;
use keyflow_ui::paint_view::{ChartSnapshot, chart_mount};

const SOURCE: &str = "120bpm 4/4 #C\n\nvs\nC F G C\n";

#[component]
pub fn KeyflowPane() -> Element {
    let setup = use_hook(|| {
        keyflow::parse(SOURCE).ok().map(|chart| {
            let data = HostData::new(ChartSnapshot {
                chart,
                revision: 0,
                playhead_secs: None,
                theme: None,
            });
            let mount = chart_mount(data.clone());
            (data, mount)
        })
    });
    let mut dark = use_signal(|| false);
    let Some((data, mount)) = setup else {
        return rsx! { div { "Could not parse the sample chart" } };
    };
    let update_data = data.clone();
    let wake = mount.wake.clone();
    rsx! {
        div { style: "display:flex; flex-direction:column; height:100%; min-height:0;",
            button {
                "aria-label": "Toggle chart theme",
                onclick: move |_| {
                    let next = !dark();
                    dark.set(next);
                    let theme = if next { dark_theme() } else { ChartTheme::default() };
                    update_data.update(|snapshot| snapshot.theme = Some(theme));
                    wake.wake();
                },
                if dark() { "Light chart" } else { "Dark chart" }
            }
            div { style: "flex:1; min-height:0;",
                NativeChartEditor {
                    initial: SOURCE.to_string(),
                    data,
                    chart: mount,
                    highlight_theme: if dark() { HighlightTheme::default_dark() } else { HighlightTheme::default_light() },
                    editor_style: (if dark() { "--code-bg:#191e2d; --background:#191e2d; --foreground:#f5f0dc; --ink:#f5f0dc;" } else { "--code-bg:#fff; --background:#fff; --foreground:#18202c; --ink:#18202c;" }).to_string(),
                    on_change: move |_| {},
                }
            }
        }
    }
}

fn dark_theme() -> ChartTheme {
    let mut theme = ChartTheme::default();
    theme.render.background_color = Color::from_rgb8(25, 30, 45);
    theme.render.foreground_color = Color::from_rgb8(245, 240, 220);
    theme.render.staff_color = Color::from_rgb8(210, 210, 205);
    theme
}
