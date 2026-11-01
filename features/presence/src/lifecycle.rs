//! The window's life in an app that can run without it.
//!
//! Shown, or in the background (the window hidden, no Dock icon, the tray
//! item left), and what each event does to it. Pure: the platform carries
//! the [`Effect`]s out, so the rules are tested without a window.
//!
//! - A `--background` launch starts in the background.
//! - Closing the window hides it when background mode is on, and the Dock
//!   icon goes with it; off, the window closes and the app quits.
//! - A Dock click, the tray's Open, a second launch: the window comes back
//!   where it was, with the Dock icon, in front.
//! - Quit quits, whatever is on show.

/// Where the app is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Its window is on screen (or minimised), its icon in the Dock.
    Window,
    /// No window, no Dock icon: the tray item is the way back.
    Background,
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The app started; `background` for a login launch.
    Launched {
        background: bool,
    },
    /// The person closed the window.
    CloseRequested,
    /// The Dock icon was clicked; `visible` when a window is on screen.
    Reopen {
        visible: bool,
    },
    /// Show the window: the tray's Open, a second launch, an action.
    Open,
    /// Close the window and keep running, whatever the setting says.
    Hide,
    /// The person turned background mode on or off.
    KeepRunning(bool),
    Quit,
}

/// What the platform does about it, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    ShowWindow,
    HideWindow,
    /// The Dock icon (and the app's place in ⌘Tab) on or off.
    DockIcon(bool),
    /// The window to the front, the app active.
    Focus,
    /// Let the window close as it would have (the app quits with its last
    /// window).
    Close,
    /// End the app.
    Quit,
}

/// The state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lifecycle {
    pub place: Place,
    /// Closing the window keeps the app running.
    pub keep_running: bool,
}

impl Lifecycle {
    #[must_use]
    pub const fn new(keep_running: bool) -> Self {
        Self {
            place: Place::Window,
            keep_running,
        }
    }

    /// What `event` does, the state moved on.
    pub fn handle(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::Launched { background: false } => {
                self.place = Place::Window;
                vec![Effect::DockIcon(true)]
            }
            Event::CloseRequested if self.keep_running => self.go_background(),
            Event::CloseRequested => vec![Effect::Close],
            Event::Launched { background: true } | Event::Hide => self.go_background(),
            // A click on the Dock icon with the window on screen: AppKit
            // brings it forward (and out of the Dock, if minimised).
            Event::Reopen { visible: true } if self.place == Place::Window => Vec::new(),
            Event::Reopen { .. } | Event::Open => match self.place {
                Place::Background => {
                    self.place = Place::Window;
                    vec![Effect::DockIcon(true), Effect::ShowWindow, Effect::Focus]
                }
                Place::Window => vec![Effect::ShowWindow, Effect::Focus],
            },
            Event::KeepRunning(on) => {
                self.keep_running = on;
                Vec::new()
            }
            Event::Quit => vec![Effect::Quit],
        }
    }

    fn go_background(&mut self) -> Vec<Effect> {
        self.place = Place::Background;
        vec![Effect::HideWindow, Effect::DockIcon(false)]
    }
}
