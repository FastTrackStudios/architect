//! A dragged tab follows the pointer point for point on a 2x screen: its
//! slide is a relative `left` (points), not a `translate` (device pixels,
//! half as far at 2x), so where it is laid out is where it is drawn.

use architect_guide::run::{Desktop, Film, Pointer};

/// The tab `tab-reorder` drags.
const DRAGGED: &str = "note-welcome";

/// The frames the pointer was held down outside the dragged tab: the
/// frame, the pointer's x, and the tab's left and right edges (desktop
/// points).
#[derive(Default)]
struct Follow {
    frame: u64,
    held: u64,
    left_behind: Vec<(u64, f32, f32, f32)>,
}

impl Film for Follow {
    fn frame(&mut self, desktop: &Desktop, pointer: Pointer) {
        self.frame = self.frame.saturating_add(1);
        if !pointer.down {
            return;
        }
        let Some(window) = desktop.windows().first() else {
            return;
        };
        let doc = window.doc.inner.borrow();
        let Some(r) = doc
            .query_selector(&format!("[data-tab=\"{DRAGGED}\"]"))
            .ok()
            .flatten()
            .and_then(|id| doc.get_client_bounding_rect(id))
        else {
            return;
        };
        self.held = self.held.saturating_add(1);
        #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
        let (left, right) = (
            window.rect.x + r.x as f32,
            window.rect.x + (r.x + r.width) as f32,
        );
        if pointer.x < left - 1.0 || pointer.x > right + 1.0 {
            self.left_behind.push((self.frame, pointer.x, left, right));
        }
    }
}

#[test]
fn a_dragged_tab_follows_the_pointer_on_a_2x_screen() -> Result<(), String> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scenarios/tab-reorder.styx");
    let source = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut scenario = architect_guide::load(&source).map_err(|e| format!("tab-reorder: {e}"))?;
    scenario.view.scale = 2.0;
    let mut follow = Follow::default();
    architect_guide::run::run(&architect_lab_media::app(), &scenario, Some(&mut follow))
        .map_err(|e| format!("tab-reorder: {e}"))?;
    if follow.held < 5 {
        return Err(format!(
            "tab-reorder: the pointer was held only {} frames",
            follow.held
        ));
    }
    match follow.left_behind.first() {
        None => Ok(()),
        Some((frame, x, left, right)) => Err(format!(
            "{} of {} held frames left the tab behind; first, frame {frame}: the pointer at {x:.0}, the tab {left:.0}–{right:.0}",
            follow.left_behind.len(),
            follow.held
        )),
    }
}
