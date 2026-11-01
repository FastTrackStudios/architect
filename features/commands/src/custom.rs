//! Custom actions (REAPER's, with SWS's conditions): a named list of
//! actions run in order, each step with its arguments, run only when its
//! test holds, as many times as it says. A custom action is itself an
//! action: bindable, in the palette, runnable by a model.

use facet::Facet;

/// When a step runs.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(C)]
pub enum Condition {
    /// The action reports itself checked (a toggle that is on).
    Checked(String),
    /// The action can run now.
    Enabled(String),
    /// The test does not hold.
    Not(Box<Self>),
}

/// One step of a custom action.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Step {
    /// The action it runs.
    pub action: String,
    /// Its arguments, as JSON (`""` for none).
    pub args: String,
    pub when: Option<Condition>,
    /// How many times it runs (1 for once).
    pub repeat: u32,
}

impl Step {
    pub fn new(action: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            args: String::new(),
            when: None,
            repeat: 1,
        }
    }

    #[must_use]
    pub fn args(mut self, json: impl Into<String>) -> Self {
        self.args = json.into();
        self
    }

    #[must_use]
    pub fn when(mut self, condition: Condition) -> Self {
        self.when = Some(condition);
        self
    }

    #[must_use]
    pub const fn repeat(mut self, times: u32) -> Self {
        self.repeat = times;
        self
    }
}

/// A named list of steps, kept with the keymap.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct CustomAction {
    /// Its own action id (`CUSTOM_…`).
    pub id: String,
    pub name: String,
    pub steps: Vec<Step>,
}

/// What a custom action runs against: the app's actions.
pub trait Runner {
    /// Runs `action` with `args` (JSON, empty for none).
    ///
    /// # Errors
    ///
    /// The action's own error, as text.
    fn run(&mut self, action: &str, args: &str) -> Result<(), String>;
    fn checked(&self, action: &str) -> bool;
    fn enabled(&self, action: &str) -> bool;
}

/// Why a custom action stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepFailed {
    /// Which step (from 0).
    pub step: usize,
    pub action: String,
    pub message: String,
}

impl CustomAction {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            steps: Vec::new(),
        }
    }

    #[must_use]
    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    /// Runs every step in order; stops at the first that fails. How many
    /// actions ran.
    ///
    /// # Errors
    ///
    /// The step that failed: a later one does not run (REAPER's).
    pub fn run(&self, runner: &mut dyn Runner) -> Result<usize, StepFailed> {
        let mut ran = 0usize;
        for (i, step) in self.steps.iter().enumerate() {
            if !step.when.as_ref().is_none_or(|c| holds(c, runner)) {
                continue;
            }
            for _ in 0..step.repeat.max(1) {
                runner
                    .run(&step.action, &step.args)
                    .map_err(|message| StepFailed {
                        step: i,
                        action: step.action.clone(),
                        message,
                    })?;
                ran = ran.saturating_add(1);
            }
        }
        Ok(ran)
    }
}

fn holds(c: &Condition, runner: &dyn Runner) -> bool {
    match c {
        Condition::Checked(a) => runner.checked(a),
        Condition::Enabled(a) => runner.enabled(a),
        Condition::Not(inner) => !holds(inner, runner),
    }
}
