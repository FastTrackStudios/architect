//! The playhead as a linked extension: its actions are [`crate::ask`],
//! for whoever the host says called (the context's `caller`, from the
//! call's principal; none is the app).

use architect_extensions::linked::{Builder, Extension};
use serde_json::Value;

use crate::{ACTIONS, Caller, ask};

/// The extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Playhead;

impl Extension for Playhead {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        for &action in ACTIONS {
            ext.action_in(action, move |args: Value, cx: Value| async move {
                ask(&Caller::of_context(&cx), action, &args)
            });
        }
    }
}
