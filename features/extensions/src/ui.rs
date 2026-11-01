//! Loaded extensions' settings on the settings page (feature `ui`).
//!
//! A [`Host`](crate::host::Host) is `Send`; the page's registry is a
//! signal of the window's. [`use_extension_sections`] is the bridge: a
//! [`SectionSink`] the host writes to from any thread, whose changes the
//! window applies to `architect-settings`' registry — an extension's
//! sections come when it loads, change when it reloads (values kept), and
//! go when it unloads.

use std::cell::RefCell;
use std::rc::Rc;

use architect_settings::{Section, try_use_settings};
use dioxus::prelude::*;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use crate::host::{SectionSink, with_sections};

/// A change to the page, from the host.
#[derive(Debug)]
enum Change {
    Put(String, Vec<Section>),
    Withdraw(String),
}

/// The settings page, as a host sees it.
#[derive(Clone, Debug)]
pub struct UiSections(UnboundedSender<Change>);

impl SectionSink for UiSections {
    fn put(&self, owner: &str, sections: Vec<Section>) -> Result<(), String> {
        self.0
            .unbounded_send(Change::Put(owner.to_owned(), sections))
            .map_err(|_| "the settings page is gone".to_owned())
    }

    fn withdraw(&self, owner: &str) {
        let _ = self.0.unbounded_send(Change::Withdraw(owner.to_owned()));
    }
}

/// The settings page above (`use_settings_provider`), as a host's
/// [`SectionSink`]: give it to `Host::new`.
#[must_use = "give it to `Host::new`"]
pub fn use_extension_sections() -> UiSections {
    let handle = try_use_settings();
    let (sink, receiver) = use_hook(|| {
        let (tx, rx) = unbounded::<Change>();
        (UiSections(tx), Rc::new(RefCell::new(Some(rx))))
    });
    use_future(move || {
        let receiver: Option<UnboundedReceiver<Change>> = receiver.borrow_mut().take();
        async move {
            let (Some(mut rx), Some(h)) = (receiver, handle) else {
                return;
            };
            while let Some(change) = rx.next().await {
                match change {
                    Change::Put(owner, sections) => {
                        let next = with_sections(&h.registry.peek(), &owner, sections);
                        match next {
                            Ok(registry) => {
                                let mut signal = h.registry;
                                signal.set(registry);
                            }
                            Err(why) => {
                                let _span = tracing::info_span!(
                                    "ext.sections",
                                    ext.id = %owner,
                                    ext.outcome = "refused",
                                    ext.why = %why,
                                )
                                .entered();
                            }
                        }
                    }
                    Change::Withdraw(owner) => h.remove_extension(&owner),
                }
            }
        }
    });
    sink
}
