//! Kanban-lite (#176, #710 U2): a board whose cards drag between columns,
//! drawn by an extension's own process and shown in a pane.
//!
//! A card is dragged with pointer events: `pointerdown` on it picks it
//! up, the press holds the pointer (the host sends its moves and its
//! release to the card wherever the pointer is), `pointermove` moves it
//! with the pointer, `pointerup` drops it in the column under the
//! pointer, at the row it is over. A `pointercancel` puts it back.

use std::sync::{Mutex, PoisonError};

use architect_extensions::guest::Guest;
use dioxus::prelude::*;
use serde_json::json;

/// The extension's id.
pub const ID: &str = "lab.kanban";
/// Its action: each column's cards.
pub const STATE: &str = "LAB_KANBAN_STATE";

/// The columns, left to right.
pub const COLUMNS: [&str; 3] = ["To do", "Doing", "Done"];
/// A column's width and a card's height, CSS pixels.
pub const COLUMN_WIDTH: f64 = 120.0;
pub const CARD_HEIGHT: f64 = 36.0;
/// Where the first card's top is in a column.
pub const HEADER: f64 = 28.0;

/// The board: each column's card titles, in order.
pub type Board = [Vec<String>; 3];

/// The board this process shows (its action reads it).
static BOARD: Mutex<Option<Board>> = Mutex::new(None);

fn first_board() -> Board {
    [
        vec!["Write the chart".into(), "Book the room".into(), "Print setlists".into()],
        vec!["Rehearse".into()],
        Vec::new(),
    ]
}

/// The board now.
#[must_use]
pub fn board() -> Board {
    BOARD
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(first_board)
        .clone()
}

fn store(b: &Board) {
    *BOARD.lock().unwrap_or_else(PoisonError::into_inner) = Some(b.clone());
}

/// Where a drop at `(x, y)` on the board lands: its column and row.
#[must_use]
pub fn drop_at(board: &Board, x: f64, y: f64) -> (usize, usize) {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::as_conversions)]
    let column = ((x / COLUMN_WIDTH).floor().max(0.0) as usize).min(COLUMNS.len().saturating_sub(1));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::as_conversions)]
    let row = ((y - HEADER) / CARD_HEIGHT).round().max(0.0) as usize;
    let len = board.get(column).map_or(0, Vec::len);
    (column, row.min(len))
}

/// `board` with the card at `from` moved to `to` (`drop_at`'s place, as
/// seen before the card left its own column). A place that isn't there
/// leaves it as it was.
#[must_use]
pub fn moved(mut board: Board, from: (usize, usize), to: (usize, usize)) -> Board {
    let (fc, fr) = from;
    let (tc, tr) = to;
    let Some(card) = board.get_mut(fc).filter(|c| fr < c.len()).map(|c| c.remove(fr)) else {
        return board;
    };
    let tr = if fc == tc && tr > fr { tr.saturating_sub(1) } else { tr };
    match board.get_mut(tc) {
        Some(column) => column.insert(tr.min(column.len()), card),
        None => {
            if let Some(column) = board.get_mut(fc) {
                column.insert(fr.min(column.len()), card);
            }
        }
    }
    board
}

/// A card being dragged: where it was, and where the pointer is.
#[derive(Clone, Copy, PartialEq)]
struct Drag {
    from: (usize, usize),
    start: (f64, f64),
    now: (f64, f64),
}

/// The board view.
#[component]
pub fn KanbanBoard() -> Element {
    let mut cards = use_signal(board);
    let mut drag = use_signal(|| None::<Drag>);
    rsx! {
        div {
            id: "board",
            style: "position: relative; display: flex; height: 100%; font-size: 13px; \
                    color: var(--foreground, #e4e4e7); user-select: none;",
            for (c, title) in COLUMNS.iter().enumerate() {
                div {
                    key: "{c}",
                    class: "column",
                    style: "position: relative; width: {COLUMN_WIDTH}px; padding: 0 4px; box-sizing: border-box; \
                            border-right: 1px solid var(--border, #3f3f46);",
                    div { style: "height: {HEADER}px; line-height: {HEADER}px; font-weight: 600;", "{title}" }
                    for (r, card) in cards.read().get(c).cloned().unwrap_or_default().into_iter().enumerate() {
                        {
                            let lifted = drag().filter(|d| d.from == (c, r));
                            let shift = lifted.map_or((0.0, 0.0), |d| (d.now.0 - d.start.0, d.now.1 - d.start.1));
                            let z = if lifted.is_some() { 2 } else { 1 };
                            rsx! {
                                div {
                                    key: "{card}",
                                    class: "card",
                                    "data-card": "{card}",
                                    style: "position: relative; height: {CARD_HEIGHT - 6.0}px; margin-bottom: 6px; \
                                            padding: 6px 8px; box-sizing: border-box; border-radius: 6px; \
                                            background: var(--card, #27272a); border: 1px solid var(--border, #3f3f46); \
                                            transform: translate({shift.0}px, {shift.1}px); z-index: {z};",
                                    onpointerdown: move |e| {
                                        let p = e.client_coordinates();
                                        drag.set(Some(Drag { from: (c, r), start: (p.x, p.y), now: (p.x, p.y) }));
                                    },
                                    onpointermove: move |e| {
                                        if let Some(mut d) = drag() {
                                            let p = e.client_coordinates();
                                            d.now = (p.x, p.y);
                                            drag.set(Some(d));
                                        }
                                    },
                                    onpointerup: move |e| {
                                        let Some(d) = drag.take() else { return };
                                        let p = e.client_coordinates();
                                        let now = cards();
                                        let to = drop_at(&now, p.x, p.y);
                                        let next = moved(now, d.from, to);
                                        store(&next);
                                        cards.set(next);
                                    },
                                    onpointercancel: move |_| drag.set(None),
                                    "{card}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The extension's process: its view and its action.
#[must_use]
pub fn guest() -> Guest {
    Guest::new()
        .view("board", KanbanBoard)
        .action(STATE, |_| async { Ok(json!(board())) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_lands_in_the_column_and_row_under_it() {
        let b = first_board();
        assert_eq!(drop_at(&b, 10.0, 0.0), (0, 0));
        assert_eq!(drop_at(&b, 130.0, 200.0), (1, 1));
        assert_eq!(drop_at(&b, 999.0, 40.0), (2, 0));
    }

    #[test]
    fn a_card_moves_between_columns_and_within_one() {
        let b = moved(first_board(), (0, 0), (2, 0));
        assert_eq!(b[2], ["Write the chart"]);
        assert_eq!(b[0], ["Book the room", "Print setlists"]);
        let b = moved(b, (0, 0), (0, 2));
        assert_eq!(b[0], ["Print setlists", "Book the room"]);
    }
}
