//! The playhead as a linked extension: its actions are [`crate::ask`],
//! for whoever the host says called (the context's `caller`, from the
//! call's principal; none is the app only for the app's own invocations,
//! `Caller::of_context`).

#[cfg(feature = "host")]
use architect_extensions::host::Host;
use architect_extensions::linked::{Builder, Extension};
use serde_json::Value;

use crate::{ACTIONS, Caller, ask};
#[cfg(feature = "host")]
use crate::{CHANGED, heard_by};

/// Raises the app event `playhead.changed` for the change `state`.
///
/// Each extension hearing it gets [`heard_by`] itself: its own source's
/// state, or only `{active}` (#660 M1). How many hooks started; 0 before
/// the app defined the event.
// How many is for whoever wants it.
#[cfg(feature = "host")]
#[allow(clippy::must_use_candidate)]
pub fn raise_changed(host: &Host, state: &Value) -> usize {
    host.raise_app_each(CHANGED, &|id: &str| heard_by(id, state))
        .unwrap_or(0)
}

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
                ask(&Caller::of_context(&cx)?, action, &args)
            });
        }
    }
}
