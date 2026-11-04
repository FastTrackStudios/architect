//! `EXT_VIEWS` (feature `host`): how each extension view keeps up.
//!
//! Its frames' times against their budget, its slow frames, whether it
//! is busy now (`docs/content/architecture/extensions.md`, "Responsive
//! by design: views and workers"). The app measures every view it does not draw itself — a Luau window
//! (`architect_canvas::ui::use_script_canvas`), a process's remote view
//! (`crate::remote::RemoteView`) — into one [`Meters`], provided in
//! Dioxus context, and registers this action over the same one. The
//! same [`Meters`] tells its `architect_workspace::ViewReport` when a
//! view turns slow or busy: the seam an app wires to its status board.

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use architect_workspace::{Meters, ViewSummary};
use serde_json::{Value, json};

static VIEWS: ActionMeta = ActionMeta {
    id: "EXT_VIEWS",
    trait_name: "Extensions",
    method_name: "EXT_VIEWS",
    display_name: "Extension views' frame times",
    description: "Each extension view the app has measured — a Luau window, a process's view — \
                  with its frames' p50, p99 and slowest time against its budget, how many ran \
                  past it, and whether one is running past it now (the pane shows its last good \
                  frame, marked busy).",
    category: "Extensions",
    group: "Extensions",
    shortcut: "",
    toggleable: false,
    undo: false,
};

fn ms(d: std::time::Duration) -> f64 {
    (d.as_secs_f64() * 1000.0 * 100.0).round() / 100.0
}

/// One view's summary as JSON.
#[must_use]
pub fn summary_json(view: &str, s: &ViewSummary) -> Value {
    json!({
        "view": view,
        "frames": s.frames,
        "slow": s.slow,
        "p50_ms": ms(s.p50),
        "p99_ms": ms(s.p99),
        "max_ms": ms(s.max),
        "budget_ms": ms(s.budget),
        "busy": s.busy,
    })
}

/// Puts `EXT_VIEWS` on `list`, reading `meters`.
///
/// # Errors
///
/// Its id is taken.
pub fn install_view_actions(list: &ActionList, meters: Meters) -> Result<(), ActionError> {
    list.register_with(&VIEWS, Effect::ReadOnly, move |_: &ActionCaller, (): ()| {
        let views: Vec<Value> = meters
            .summaries()
            .iter()
            .map(|(name, s)| summary_json(name, s))
            .collect();
        Ok::<_, String>(RawJson::from(json!({ "views": views }).to_string()))
    })
}
