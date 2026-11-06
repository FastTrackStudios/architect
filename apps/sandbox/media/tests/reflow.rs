//! A note's text keeps its lines while the workspace moves around it
//! (architect#152): every frame of the films that move a dock, a sidebar
//! or a split, each paragraph in a note pane is broken into lines as wide
//! as its box, never into the one-word column of its narrowest size.

use architect_guide::run::{Desktop, Film, Pointer};

/// Lines broken where the next word would have fit, past which a
/// paragraph was broken at a narrower width than its box.
const NEEDLESS: usize = 2;

/// A paragraph seen broken too narrow: the frame and what was seen.
struct Seen {
    frame: u64,
    width: f32,
    lines: Vec<String>,
}

#[derive(Default)]
struct Watch {
    frame: u64,
    seen: Vec<Seen>,
    /// Centred titles drawn off their centre: the frame, the title, and
    /// how far off, points.
    off_centre: Vec<(u64, String, f32)>,
}

/// One line as drawn: its text and how far it reaches, points.
struct Line {
    text: String,
    advance: f32,
}

/// How many of `lines` end where the next line's first word would have
/// fitted in `width`: greedy breaking at `width` leaves none.
fn needless_breaks(lines: &[Line], width: f32) -> usize {
    lines
        .windows(2)
        .filter(|pair| {
            let [this, next] = pair else { return false };
            let chars = next.text.chars().count().max(1);
            let word = next
                .text
                .split_whitespace()
                .next()
                .unwrap_or("")
                .chars()
                .count();
            // The next word's share of its line, and a space.
            #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
            let word_w = next.advance * (word as f32 + 1.0) / chars as f32;
            word > 0 && this.advance + word_w < width * 0.9
        })
        .count()
}

impl Film for Watch {
    fn frame(&mut self, desktop: &Desktop, _pointer: Pointer) {
        self.frame = self.frame.saturating_add(1);
        let scale = desktop.scale();
        for window in desktop.windows() {
            let doc = window.doc.inner.borrow();
            // A pane's header title, centred in its row.
            if let Ok(ids) = doc.query_selector_all("[data-group] [style*=\"text-align:center\"]") {
                for id in ids {
                    let Some(node) = doc.get_node(id) else {
                        continue;
                    };
                    let Some(text) = node
                        .element_data()
                        .and_then(|e| e.inline_layout_data.as_ref())
                    else {
                        continue;
                    };
                    let width = node.final_layout().content_box_width();
                    let Some(line) = text.layout.lines().next() else {
                        continue;
                    };
                    let m = line.metrics();
                    let advance = (m.advance - m.hanging_advance) / scale;
                    if advance >= width || text.text.trim().is_empty() {
                        continue;
                    }
                    let off = m.offset / scale - (width - advance) / 2.0;
                    if off.abs() > 2.0 {
                        self.off_centre.push((self.frame, text.text.clone(), off));
                    }
                }
            }
            let Ok(ids) = doc.query_selector_all("[data-pane^=\"note-\"] *") else {
                continue;
            };
            for id in ids {
                let Some(node) = doc.get_node(id) else {
                    continue;
                };
                let Some(text) = node
                    .element_data()
                    .and_then(|e| e.inline_layout_data.as_ref())
                else {
                    continue;
                };
                let width = node.final_layout().content_box_width();
                let lines: Vec<Line> = text
                    .layout
                    .lines()
                    .map(|line| Line {
                        text: text
                            .text
                            .get(line.text_range())
                            .unwrap_or_default()
                            .trim()
                            .to_owned(),
                        advance: (line.metrics().advance - line.metrics().hanging_advance) / scale,
                    })
                    .collect();
                if needless_breaks(&lines, width) >= NEEDLESS {
                    self.seen.push(Seen {
                        frame: self.frame,
                        width,
                        lines: lines.into_iter().map(|l| l.text).collect(),
                    });
                }
            }
        }
    }
}

fn watched(id: &str) -> Result<Watch, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scenarios")
        .join(format!("{id}.styx"));
    let source = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let scenario = architect_guide::load(&source).map_err(|e| format!("{id}: {e}"))?;
    let mut watch = Watch::default();
    architect_guide::run::run(&architect_sandbox_media::app(), &scenario, Some(&mut watch))
        .map_err(|e| format!("{id}: {e}"))?;
    if watch.frame == 0 {
        return Err(format!("{id}: no frames"));
    }
    Ok(watch)
}

/// Every paragraph seen broken too narrow in `id`'s film, as an error.
fn lines_kept(id: &str) -> Result<(), String> {
    let seen = watched(id)?.seen;
    let Some(s) = seen.first() else {
        return Ok(());
    };
    Err(format!(
        "{id}: {} paragraphs broken too narrow; first, frame {}: a box {:.0} points wide broken as {:?}",
        seen.len(),
        s.frame,
        s.width,
        s.lines.iter().take(8).collect::<Vec<_>>()
    ))
}

#[test]
fn a_note_keeps_its_lines_while_the_dock_moves() -> Result<(), String> {
    lines_kept("workspace-docks-ribbon")
}

#[test]
fn a_note_keeps_its_lines_while_the_layout_changes() -> Result<(), String> {
    lines_kept("showcase-layout")
}

#[test]
fn a_pane_title_stays_centred_after_a_tab_drop() -> Result<(), String> {
    let off = watched("showcase-layout")?.off_centre;
    match off.first() {
        None => Ok(()),
        Some((frame, title, by)) => Err(format!(
            "{} titles off centre; first, frame {frame}: {title:?} by {by:.1} points",
            off.len()
        )),
    }
}
