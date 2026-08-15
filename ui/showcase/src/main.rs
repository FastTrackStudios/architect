//! architect-ui Showcase — visual verification of all components.
//! Run: `dx serve --platform desktop`

use dioxus::desktop::{tao::window::WindowBuilder, Config};
use dioxus::prelude::*;
use architect_ui::showcase::Showcase;

fn main() {
    unsafe {
        std::env::set_var("GTK_THEME", "Adwaita:dark");
    }
    unsafe {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }

    let cfg = Config::new()
        .with_window(
            WindowBuilder::new()
                .with_title("architect-ui Showcase")
                .with_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(1400.0, 900.0)),
        )
        .with_menu(None);

    LaunchBuilder::desktop().with_cfg(cfg).launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Stylesheet { href: asset!("/assets/tailwind.css") }
        Showcase { renderer: "Desktop".to_string() }
    }
}
