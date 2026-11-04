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
//! [`ViewProblems`] is that report for `APP_STATUS`: a view that is slow
//! or busy is a problem there until it draws within its budget again.
//! [`health_lines`] is an extension's views' health in words (the
//! manager's "Frame health").

use architect::action::{
    ActionCaller, ActionError, ActionList, ActionMeta, Effect, Problem, Problems, RawJson,
};
use architect_workspace::{Meters, ViewHealth, ViewReport, ViewSummary};
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

/// The key a view's problem is reported under in [`Problems`].
#[must_use]
pub fn problem_key(view: &str) -> String {
    format!("view:{view}")
}

/// A view's health as `APP_STATUS` problems (`Meters::with_report`).
///
/// A view turned slow or busy is reported under [`problem_key`] (source
/// `view`, field its name) with its timings; it is cleared when the view
/// draws within its budget again. Cheap to clone.
#[derive(Clone, Debug)]
pub struct ViewProblems {
    problems: Problems,
}

impl ViewProblems {
    /// Reporting into `problems` (an app's: `ActionList::problems`).
    #[must_use]
    pub const fn new(problems: Problems) -> Self {
        Self { problems }
    }

    /// Clears view `view`'s problem (it was unmounted, its extension
    /// unloaded).
    pub fn forget(&self, view: &str) {
        self.problems.report(problem_key(view), Vec::new());
    }
}

/// What a slow or busy view says, with its timings.
#[must_use]
pub fn health_message(view: &str, health: ViewHealth, s: &ViewSummary) -> Option<String> {
    let timings = format!(
        "p50 {} ms, p99 {} ms, {} of {} frames past its {} ms budget",
        ms(s.p50),
        ms(s.p99),
        s.slow,
        s.frames,
        ms(s.budget)
    );
    match health {
        ViewHealth::Slow(took) => Some(format!(
            "{view} is slow: a frame took {} ms ({timings})",
            ms(took)
        )),
        ViewHealth::Busy => Some(format!(
            "{view} is busy: a frame is running past its budget, its last good frame shows ({timings})"
        )),
        ViewHealth::Recovered => None,
    }
}

impl ViewReport for ViewProblems {
    fn report(&self, view: &str, health: ViewHealth, summary: &ViewSummary) {
        let found = health_message(view, health, summary)
            .map(|m| vec![Problem::new("view", m).field(view)])
            .unwrap_or_default();
        self.problems.report(problem_key(view), found);
    }
}

/// The views of extension `extension` that `meters` has measured, one
/// line each: `counter: p50 1.2 ms · p99 4.8 ms · 3 slow of 812 frames
/// (budget 16 ms)`, `· busy now` when one is running past it.
#[must_use]
pub fn health_lines(meters: &Meters, extension: &str) -> Vec<String> {
    let prefix = format!("{extension}/");
    meters
        .summaries()
        .into_iter()
        .filter_map(|(name, s)| {
            let view = name.strip_prefix(&prefix)?.to_owned();
            let busy = if s.busy { " · busy now" } else { "" };
            Some(format!(
                "{view}: p50 {} ms · p99 {} ms · {} slow of {} frames (budget {} ms){busy}",
                ms(s.p50),
                ms(s.p99),
                s.slow,
                s.frames,
                ms(s.budget)
            ))
        })
        .collect()
}

/// The views of `extension` as [`summary_json`]s.
#[must_use]
pub fn health_json(meters: &Meters, extension: &str) -> Vec<Value> {
    let prefix = format!("{extension}/");
    meters
        .summaries()
        .iter()
        .filter(|(name, _)| name.starts_with(&prefix))
        .map(|(name, s)| summary_json(name, s))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use architect::action::Problems;
    use architect_workspace::Meters;

    use super::{ViewProblems, health_lines};

    /// A slow view is an `APP_STATUS` problem with its timings, a busy one
    /// too, and both clear when it keeps up again.
    #[test]
    fn slow_and_busy_views_are_problems_until_they_recover() {
        let problems = Problems::default();
        let meters = Meters::new().with_report(Arc::new(ViewProblems::new(problems.clone())));
        let meter = meters.meter("lab.naughty/spin", Duration::from_millis(16));
        meter.record(Duration::from_millis(5));
        assert!(problems.all().is_empty());
        meter.record(Duration::from_millis(40));
        let all = problems.all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].source, "view");
        assert_eq!(all[0].field.as_deref(), Some("lab.naughty/spin"));
        assert!(all[0].message.contains("slow"), "{}", all[0].message);
        assert!(all[0].message.contains("p99 40 ms"), "{}", all[0].message);
        meter.record(Duration::from_millis(3));
        assert!(problems.all().is_empty(), "recovered: cleared");
        meter.set_busy(true);
        assert!(problems.all()[0].message.contains("busy"));
        meter.record(Duration::from_millis(3));
        assert!(problems.all().is_empty());
        let lines = health_lines(&meters, "lab.naughty");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("spin: p50 "), "{}", lines[0]);
        assert!(lines[0].contains("1 slow of 4 frames"), "{}", lines[0]);
        assert!(health_lines(&meters, "lab.other").is_empty());
    }
}
