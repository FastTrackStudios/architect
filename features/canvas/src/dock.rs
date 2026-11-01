//! Which of a script's windows have panes: what a host opens, closes and
//! tells the script, each time the windows drawn or the panes open
//! change.
//!
//! A window the script begins gets a pane; a pane the person closes is
//! told to the script (once: `Begin` gives `open == false`), and not
//! opened again while the script still draws the window; a window the
//! script stops drawing loses its pane. A pane already open for a window
//! (a screenset brought it back) is taken as its pane.

use std::collections::BTreeSet;

/// What to do now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Windows to open a pane for.
    pub open: Vec<String>,
    /// Windows whose pane to close: the script no longer draws them.
    pub close: Vec<String>,
    /// Windows whose pane the person closed: tell the script.
    pub closed: Vec<String>,
}

impl Plan {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.open.is_empty() && self.close.is_empty() && self.closed.is_empty()
    }
}

/// The windows a host has given panes, across changes.
#[derive(Clone, Debug, Default)]
pub struct Docking {
    opened: BTreeSet<String>,
    /// Closed by the person, still drawn: not reopened until the script
    /// has stopped drawing them.
    closing: BTreeSet<String>,
}

impl Docking {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What to do, given the windows `drawn` this frame and those with a
    /// pane `open` now.
    pub fn plan(&mut self, drawn: &[&str], open: &BTreeSet<String>) -> Plan {
        let is_drawn = |id: &str| drawn.contains(&id);
        let mut plan = Plan::default();
        self.closing.retain(|id| is_drawn(id));
        let opened: Vec<String> = self.opened.iter().cloned().collect();
        for id in opened {
            if !open.contains(&id) {
                self.opened.remove(&id);
                if is_drawn(&id) {
                    self.closing.insert(id.clone());
                    plan.closed.push(id);
                }
            } else if !is_drawn(&id) {
                self.opened.remove(&id);
                plan.close.push(id);
            }
        }
        for id in drawn {
            if self.closing.contains(*id) || self.opened.contains(*id) {
                continue;
            }
            self.opened.insert((*id).to_owned());
            if !open.contains(*id) {
                plan.open.push((*id).to_owned());
            }
        }
        plan
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Docking, Plan};

    fn panes(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_window_gets_a_pane_and_loses_it_when_the_script_stops_drawing_it() {
        let mut d = Docking::new();
        let plan = d.plan(&["t/a"], &panes(&[]));
        assert_eq!(plan.open, vec!["t/a".to_owned()]);
        assert!(d.plan(&["t/a"], &panes(&["t/a"])).is_empty());
        let plan = d.plan(&[], &panes(&["t/a"]));
        assert_eq!(plan.close, vec!["t/a".to_owned()]);
        assert!(d.plan(&[], &panes(&[])).is_empty());
    }

    #[test]
    fn a_pane_closed_by_the_person_is_told_once_and_not_reopened_while_drawn() {
        let mut d = Docking::new();
        d.plan(&["t/a"], &panes(&[]));
        let plan = d.plan(&["t/a"], &panes(&[]));
        assert_eq!(
            plan,
            Plan {
                closed: vec!["t/a".to_owned()],
                ..Plan::default()
            }
        );
        assert!(
            d.plan(&["t/a"], &panes(&[])).is_empty(),
            "still drawn, not reopened"
        );
        assert!(d.plan(&[], &panes(&[])).is_empty(), "the script let it go");
        assert_eq!(
            d.plan(&["t/a"], &panes(&[])).open,
            vec!["t/a".to_owned()],
            "begun again"
        );
    }

    #[test]
    fn a_pane_a_screenset_brought_back_is_the_windows_own() {
        let mut d = Docking::new();
        assert!(d.plan(&["t/a"], &panes(&["t/a"])).is_empty());
        assert_eq!(d.plan(&["t/a"], &panes(&[])).closed, vec!["t/a".to_owned()]);
    }
}
