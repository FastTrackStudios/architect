//! Views as an app and its extensions agree on them, with no UI: the view
//! kinds a pane can show ([`kinds`]: `dioxus`, `paint`, `web`, `process`
//! and any the app registers) and each view's frame meters ([`meter`]).
//!
//! The workspace (the panes that show views) and the extension runtime
//! (the extensions that declare them) both read these; neither owns the
//! other.

pub mod kinds;
pub mod meter;

pub use kinds::{
    Capabilities, Capability, DIOXUS, Hosts, KindError, KindId, KindSpec, PAINT, PROCESS, Part,
    Placement, Placements, ViewHost, ViewKinds, WEB,
};
pub use meter::{FRAME_BUDGET, Meters, ViewHealth, ViewMeter, ViewReport, ViewSummary};

/// The context-menu targets the workspace owns, `(kind, what it is)`:
/// what an app's extension host declares for it
/// (`architect_extensions`' `Host::declare_context_targets`).
pub const CONTEXT_TARGETS: [(&str, &str); 3] = [
    ("pane", "every pane's tab; its id the pane's"),
    ("tab", "a tab, past its view kind's rows; its id the pane's"),
    (
        "view",
        "the tabs of one view kind's panes: `view:<kind>`; its id the pane's",
    ),
];
