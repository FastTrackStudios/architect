//! The window as the shell sees it: its size measured from the window, and
//! — on a desk standing in for a device — that device's touch and insets,
//! with its status bar, camera housing and home indicator drawn where the
//! real one has them, so spacing is judged against them.

use architect_shell::{Insets, Screen, WindowChrome};
use dioxus::prelude::*;
use dioxus_native::winit::event::WindowEvent;
use dioxus_native::winit::window::Window;

/// What the window stands for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Device {
    /// What it is: an iPhone or iPad running the lab (its own safe area,
    /// read from the window: the app has Blitz lay the page out edge to
    /// edge, `BLITZ_SAFE_AREA=0`), or a desk window as a desk.
    Native,
    /// An iPhone 17 Pro held upright (402 × 874).
    IPhone,
    /// The same on its side, housing on the left (874 × 402).
    IPhoneSide,
    /// An 11" iPad Pro upright (834 × 1194).
    IPad,
    /// The same on its side (1194 × 834).
    IPadSide,
}

impl Device {
    pub const ALL: [Self; 5] = [
        Self::Native,
        Self::IPhone,
        Self::IPhoneSide,
        Self::IPad,
        Self::IPadSide,
    ];

    /// The window size it wants, in points (`None`: leave it).
    #[must_use]
    pub const fn size(self) -> Option<(f64, f64)> {
        match self {
            Self::Native => None,
            Self::IPhone => Some((402.0, 874.0)),
            Self::IPhoneSide => Some((874.0, 402.0)),
            Self::IPad => Some((834.0, 1194.0)),
            Self::IPadSide => Some((1194.0, 834.0)),
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Native => "This window",
            Self::IPhone => "iPhone",
            Self::IPhoneSide => "iPhone, sideways",
            Self::IPad => "iPad",
            Self::IPadSide => "iPad, sideways",
        }
    }

    /// What the page keeps clear itself on that device (its safe area).
    #[must_use]
    pub const fn insets(self) -> Insets {
        match self {
            Self::Native => Insets {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
            Self::IPhone => Insets {
                top: 62.0,
                right: 0.0,
                bottom: 34.0,
                left: 0.0,
            },
            Self::IPhoneSide => Insets {
                top: 0.0,
                right: 62.0,
                bottom: 21.0,
                left: 62.0,
            },
            Self::IPad | Self::IPadSide => Insets {
                top: 24.0,
                right: 0.0,
                bottom: 20.0,
                left: 0.0,
            },
        }
    }

    const fn touch(self) -> bool {
        !matches!(self, Self::Native) || cfg!(any(target_os = "ios", target_os = "android"))
    }

    /// The device by its name (`iphone`, `iphone-side`, `ipad`,
    /// `ipad-side`); anything else is [`Device::Native`].
    #[must_use]
    pub fn named(name: &str) -> Self {
        match name {
            "iphone" => Self::IPhone,
            "iphone-side" => Self::IPhoneSide,
            "ipad" => Self::IPad,
            "ipad-side" => Self::IPadSide,
            _ => Self::Native,
        }
    }
}

fn measure(window: &dyn Window, device: Device) -> Option<Screen> {
    let px = window.surface_size();
    if px.width == 0 || px.height == 0 {
        return None;
    }
    let scale = window.scale_factor().max(0.1);
    let insets = if device == Device::Native && cfg!(any(target_os = "ios", target_os = "android"))
    {
        let safe = window.safe_area();
        Insets {
            top: f64::from(safe.top) / scale,
            right: f64::from(safe.right) / scale,
            bottom: f64::from(safe.bottom) / scale,
            left: f64::from(safe.left) / scale,
        }
    } else {
        device.insets()
    };
    Some(Screen {
        width: f64::from(px.width) / scale,
        height: f64::from(px.height) / scale,
        touch: device.touch(),
        insets,
    })
}

/// The window's [`Screen`], published as a `Signal<Screen>` for the lab.
///
/// Also publishes the [`Device`] it stands for (a `Signal<Device>`, which
/// the host may change: [`set_device`]). Kept current through a resize, a
/// change of scale and, on iOS (whose window takes its scene's size after
/// the first frame), a few reads a second.
#[must_use]
pub fn use_window_screen(first: Device) -> Signal<Screen> {
    let window = dioxus_native::use_window();
    let device = use_context_provider(|| Signal::new(first));
    let measured = window.clone();
    let mut screen = use_context_provider(move || {
        Signal::new(measure(&*measured, first).unwrap_or_else(|| Screen {
            width: 1194.0,
            height: 834.0,
            touch: false,
            insets: Insets::default(),
        }))
    });
    let update = move || {
        if let Some(now) = measure(&*window, *device.peek())
            && *screen.peek() != now
        {
            screen.set(now);
        }
    };
    let mut on_event = update.clone();
    dioxus_native::use_window_event(move |event, _| {
        if matches!(
            event,
            WindowEvent::SurfaceResized(_)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::RedrawRequested
        ) {
            on_event();
        }
    });
    // A change of device: its insets and touch, at once.
    let mut on_device = update.clone();
    use_effect(move || {
        let _ = device();
        on_device();
    });
    use_hook(move || {
        if cfg!(target_os = "ios") {
            let mut update = update.clone();
            spawn(async move {
                loop {
                    futures_timer::Delay::new(std::time::Duration::from_millis(300)).await;
                    update();
                }
            });
        }
    });
    screen
}

/// Stand the window in for `device`: its size, insets and touch.
pub fn set_device(device: Device) {
    let mut current = consume_context::<Signal<Device>>();
    current.set(device);
    if let Some((w, h)) = device.size() {
        let window = consume_context::<std::sync::Arc<dyn Window>>();
        let _ =
            window.request_surface_size(dioxus_native::winit::dpi::LogicalSize::new(w, h).into());
    }
}

/// What a stood-in phone has over the page: the status bar and the camera
/// housing at the top, the home indicator at the foot. Nothing on a real
/// device (it draws its own) or a desk.
#[component]
pub fn DeviceChrome() -> Element {
    let device = use_context::<Signal<Device>>()();
    let ink = "#e4e4e7";
    match device {
        Device::IPhone => rsx! {
            div { style: "position: absolute; top: 0; left: 0; width: 100%; height: 54px; pointer-events: none; \
                          display: flex; align-items: center; justify-content: space-between; padding: 0 34px 0 52px; \
                          box-sizing: border-box; color: {ink}; font-family: -apple-system, system-ui, sans-serif; \
                          font-size: 17px; font-weight: 600;",
                span { "9:41" }
                span { style: "font-size: 13px;", "●●● 100%" }
            }
            div { style: "position: absolute; top: 11px; left: 138px; width: 126px; height: 37px; border-radius: 19px; \
                          background: #000; pointer-events: none;" }
            HomeIndicator {}
        },
        Device::IPhoneSide => rsx! {
            div { style: "position: absolute; top: 138px; left: 11px; width: 37px; height: 126px; border-radius: 19px; \
                          background: #000; pointer-events: none;" }
            HomeIndicator {}
        },
        Device::IPad | Device::IPadSide => rsx! { HomeIndicator {} },
        Device::Native => rsx! {},
    }
}

#[component]
fn HomeIndicator() -> Element {
    rsx! {
        div { style: "position: absolute; bottom: 8px; left: 0; width: 100%; display: flex; justify-content: center; \
                      pointer-events: none;",
            div { style: "width: 140px; height: 5px; border-radius: 3px; background: #e4e4e7;" }
        }
    }
}

/// The window's own title bar, on a Mac.
///
/// The content runs under a transparent title bar (the window is made so:
/// `architect-desktop`'s `main`), the shell keeps the traffic lights' corner clear, and its empty
/// bar space moves and zooms the window. None in full screen (no lights)
/// and off a Mac (the system's title bar).
#[must_use]
pub fn use_window_chrome() -> Option<WindowChrome> {
    let window = dioxus_native::use_window();
    let chrome = use_hook(|| {
        let (drag, zoom) = (window.clone(), window.clone());
        WindowChrome::mac(
            move |()| {
                let _ = drag.drag_window();
            },
            move |()| zoom.set_maximized(!zoom.is_maximized()),
        )
    });
    // Whether the window is in full screen, followed through the window's
    // events: read only while drawing, it was still full screen at the
    // resize that leaves it, and the row stayed without its lights.
    let mut full = use_signal(|| window.fullscreen().is_some());
    let watched = window.clone();
    dioxus_native::use_window_event(move |event, _| {
        if matches!(
            event,
            WindowEvent::SurfaceResized(_)
                | WindowEvent::Moved(_)
                | WindowEvent::Focused(_)
                | WindowEvent::Occluded(_)
                | WindowEvent::RedrawRequested
        ) {
            let now = watched.fullscreen().is_some();
            if *full.peek() != now {
                full.set(now);
            }
        }
    });
    let full = full();
    // Read again whenever the screen changes, and put the traffic lights
    // back in the row: AppKit lays its title bar out again on a resize. In
    // full screen AppKit's own (the bar that slides down) is left alone.
    let _ = use_context::<Signal<Screen>>()();
    let device = use_context::<Signal<Device>>()();
    // A window standing in for a phone or a tablet has no traffic lights.
    #[cfg(target_os = "macos")]
    if !full {
        architect_shell::fit_traffic_lights(
            window.rwh_06_window_handle(),
            (device == Device::Native).then_some(WindowChrome::MAC_ROW),
            WindowChrome::MAC_LIGHTS_LEFT,
        );
    }
    (cfg!(target_os = "macos") && !full && device == Device::Native).then_some(chrome)
}
