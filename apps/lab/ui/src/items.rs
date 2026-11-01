//! The lab's made-up lists: the Inbox's captures, the tasks (each in a
//! project), the schedule. Kept in a `Signal<Lists>`, so a capture adds
//! one, Done and Delete take one away and Undo puts it back where it was.

/// The projects on the left side, in their order.
pub const PROJECTS: [&str; 5] = [
    "Studio",
    "Harbor & Pine",
    "Album release",
    "Signal",
    "Admin",
];

/// One row of a list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    /// Its own, for as long as the lab runs (a route names it).
    pub id: usize,
    /// The tab whose list it is in.
    pub tab: &'static str,
    pub name: String,
    /// When, or where it came from: "due Friday", "Captured 1h ago".
    pub detail: String,
    pub project: Option<&'static str>,
}

impl Item {
    /// What a row shows under the name: the project, then the detail.
    #[must_use]
    pub fn meta(&self) -> String {
        self.project
            .map_or_else(|| self.detail.clone(), |p| format!("{p} · {}", self.detail))
    }
}

/// Every list, newest first.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Lists {
    items: Vec<Item>,
    next: usize,
}

impl Lists {
    /// The lab's lists as it opens.
    #[must_use]
    pub fn sample() -> Self {
        let mut lists = Self::default();
        let rows: [(&'static str, &str, &str, Option<&'static str>); 15] = [
            ("inbox", "Call back the venue", "Captured 10 min ago", None),
            ("inbox", "Idea: a capo chart view", "Captured 1h ago", None),
            ("inbox", "Bass DI is buzzing", "Captured yesterday", None),
            (
                "inbox",
                "Ask Cody about the click",
                "Captured yesterday",
                None,
            ),
            (
                "inbox",
                "Swap the opener and closer",
                "Captured Oct 28",
                None,
            ),
            ("tasks", "Mix the Sunday set", "due Friday", Some("Studio")),
            (
                "tasks",
                "Send stems to Jess",
                "due today",
                Some("Harbor & Pine"),
            ),
            (
                "tasks",
                "Book the Grey Eagle",
                "due next week",
                Some("Album release"),
            ),
            ("tasks", "Rewire the keys rig", "no date", Some("Signal")),
            (
                "tasks",
                "Chart Late Train in C",
                "due Thursday",
                Some("Studio"),
            ),
            ("tasks", "Renew the Task plan", "due Nov 3", Some("Admin")),
            (
                "schedule",
                "Rehearsal",
                "Today · 7:30 PM · Crossroads",
                None,
            ),
            (
                "schedule",
                "Mix review with Sam",
                "Tomorrow · 11:00",
                Some("Studio"),
            ),
            ("schedule", "Sunday Night Worship", "Sun · 6:00 PM", None),
            (
                "schedule",
                "Album Release Show",
                "Sat Nov 14 · 8:00 PM",
                Some("Album release"),
            ),
        ];
        for (tab, name, detail, project) in rows {
            lists.push(tab, name, detail, project);
        }
        lists
    }

    fn push(&mut self, tab: &'static str, name: &str, detail: &str, project: Option<&'static str>) {
        self.items.push(Item {
            id: self.next,
            tab,
            name: name.to_owned(),
            detail: detail.to_owned(),
            project,
        });
        self.next = self.next.saturating_add(1);
    }

    /// A tab's list (only `project`'s, when one is picked), newest first
    /// or oldest first.
    #[must_use]
    pub fn of(&self, tab: &str, project: Option<&str>, oldest_first: bool) -> Vec<&Item> {
        let mut list: Vec<&Item> = self
            .items
            .iter()
            .filter(|i| i.tab == tab && project.is_none_or(|p| i.project == Some(p)))
            .collect();
        if oldest_first {
            list.reverse();
        }
        list
    }

    #[must_use]
    pub fn get(&self, id: usize) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// How many a tab's list has.
    #[must_use]
    pub fn count(&self, tab: &str) -> usize {
        self.items.iter().filter(|i| i.tab == tab).count()
    }

    /// How many tasks `project` has.
    #[must_use]
    pub fn in_project(&self, project: &str) -> usize {
        self.items
            .iter()
            .filter(|i| i.tab == "tasks" && i.project == Some(project))
            .count()
    }

    /// Adds `name` at the top of `tab`'s list (in `project`, if one is
    /// picked); its id, or `None` for a name that is only spaces.
    pub fn add(
        &mut self,
        tab: &'static str,
        name: &str,
        project: Option<&'static str>,
    ) -> Option<usize> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let detail = if tab == "inbox" {
            "Captured just now"
        } else {
            "Added just now"
        };
        let id = self.next;
        let at = self
            .items
            .iter()
            .position(|i| i.tab == tab)
            .unwrap_or(self.items.len());
        self.items.insert(
            at,
            Item {
                id,
                tab,
                name: name.to_owned(),
                detail: detail.to_owned(),
                project,
            },
        );
        self.next = self.next.saturating_add(1);
        Some(id)
    }

    /// Files `id` in `tab`'s list, in `project` (an Inbox capture made a
    /// task): at the top there, with its date kept. Whether it moved.
    pub fn move_to(&mut self, id: usize, tab: &'static str, project: Option<&'static str>) -> bool {
        let Some((_, mut item)) = self.remove(id) else {
            return false;
        };
        if item.tab == "inbox" && tab != "inbox" {
            "no date".clone_into(&mut item.detail);
        }
        item.tab = tab;
        item.project = project;
        let at = self
            .items
            .iter()
            .position(|i| i.tab == tab)
            .unwrap_or(self.items.len());
        self.items.insert(at, item);
        true
    }

    /// Takes `id` out; where it was and what it was, for [`Lists::restore`].
    pub fn remove(&mut self, id: usize) -> Option<(usize, Item)> {
        let at = self.items.iter().position(|i| i.id == id)?;
        Some((at, self.items.remove(at)))
    }

    /// Puts back what [`Lists::remove`] took, where it was.
    pub fn restore(&mut self, at: usize, item: Item) {
        let at = at.min(self.items.len());
        self.items.insert(at, item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_shows_only_its_tasks() {
        let lists = Lists::sample();
        let studio = lists.of("tasks", Some("Studio"), false);
        assert_eq!(studio.len(), 2);
        assert!(studio.iter().all(|i| i.project == Some("Studio")));
        assert_eq!(lists.in_project("Studio"), 2);
        assert_eq!(lists.of("tasks", None, false).len(), 6);
    }

    #[test]
    fn every_project_on_the_left_has_tasks() {
        let lists = Lists::sample();
        assert!(PROJECTS.iter().all(|p| lists.in_project(p) > 0));
    }

    #[test]
    fn a_capture_goes_to_the_top_of_its_list() {
        let mut lists = Lists::sample();
        let id = lists.add("inbox", "  Restring the Tele ", None);
        let inbox = lists.of("inbox", None, false);
        assert_eq!(inbox.first().map(|i| i.id), id);
        assert_eq!(
            inbox.first().map(|i| i.name.as_str()),
            Some("Restring the Tele")
        );
        assert_eq!(lists.count("inbox"), 6);
    }

    #[test]
    fn a_blank_capture_adds_nothing() {
        let mut lists = Lists::sample();
        assert_eq!(lists.add("inbox", "   ", None), None);
        assert_eq!(lists.count("inbox"), 5);
    }

    #[test]
    fn a_new_task_lands_in_the_picked_project() {
        let mut lists = Lists::sample();
        let id = lists.add("tasks", "Print the charts", Some("Signal"));
        let first = lists.of("tasks", Some("Signal"), false)[0].id;
        assert_eq!(Some(first), id);
    }

    #[test]
    fn undo_puts_a_removed_item_back_where_it_was() {
        let mut lists = Lists::sample();
        let before = lists.clone();
        let id = lists.of("inbox", None, false)[2].id;
        let (at, item) = lists.remove(id).unwrap();
        assert!(lists.get(id).is_none());
        lists.restore(at, item);
        assert_eq!(lists, before);
    }

    #[test]
    fn a_capture_filed_in_a_project_becomes_its_task() {
        let mut lists = Lists::sample();
        let id = lists.of("inbox", None, false)[0].id;
        assert!(lists.move_to(id, "tasks", Some("Signal")));
        assert_eq!(lists.count("inbox"), 4);
        let first = lists.of("tasks", Some("Signal"), false)[0];
        assert_eq!((first.id, first.detail.as_str()), (id, "no date"));
        assert!(!lists.move_to(9999, "tasks", None));
    }

    #[test]
    fn oldest_first_reverses_the_list() {
        let lists = Lists::sample();
        let newest = lists.of("inbox", None, false);
        let oldest = lists.of("inbox", None, true);
        assert_eq!(newest.first(), oldest.last());
    }

    #[test]
    fn a_rows_meta_leads_with_its_project() {
        let lists = Lists::sample();
        let task = lists.of("tasks", None, false)[0];
        assert_eq!(task.meta(), "Studio · due Friday");
        let capture = lists.of("inbox", None, false)[0];
        assert_eq!(capture.meta(), "Captured 10 min ago");
    }
}
