//! ⌘O in the window's org, and across every org in All.

#![cfg(feature = "commands")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::cell::RefCell;
use std::rc::Rc;

use architect_commands::{OpenIn, OpenItem, Openers};
use architect_orgs::{Org, OrgId, Orgs, Role, Selection, scoped_opener};

#[test]
fn quick_open_follows_the_org_and_opens_an_item_where_it_came_from() {
    let orgs = Rc::new(RefCell::new(Orgs::new(vec![
        Org::new("fts", "FastTrackStudio", Role::Admin),
        Org::new("hp", "Harbor & Pine", Role::Member),
    ])));
    let opened: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
    let mut openers = Openers::new();
    openers.register(scoped_opener(
        "song",
        "Songs",
        {
            let orgs = Rc::clone(&orgs);
            move || orgs.borrow().clone()
        },
        |org, _query| {
            let title = if org.0 == "fts" {
                "Amazing Grace"
            } else {
                "Holy Holy"
            };
            vec![OpenItem::new("song", "s1", title)]
        },
        {
            let opened = Rc::clone(&opened);
            move |org, item, _how| opened.borrow_mut().push((org.0.clone(), item.id.clone()))
        },
    ));

    // In FastTrackStudio, only its songs.
    let found = openers.search("");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].item.title, "Amazing Grace");

    // In All, every org's, each naming its org.
    orgs.borrow_mut().switch(Selection::All);
    let found = openers.search("holy");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].item.detail, "Harbor & Pine");
    let item = found[0].item.clone();

    // Opened in the org it came from, by its own id, even after the
    // window moved back.
    orgs.borrow_mut().switch(Selection::One(OrgId::new("fts")));
    assert!(openers.open(&item, OpenIn::Here));
    assert_eq!(*opened.borrow(), vec![("hp".to_owned(), "s1".to_owned())]);
}
