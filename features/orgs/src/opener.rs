//! ⌘O in the window's org, or across every org in All (feature
//! `commands`): one [`Opener`] that asks each org in the scope and opens
//! an item in the org it came from.

use architect_commands::{OpenIn, OpenItem, Opener};

use crate::model::{OrgId, Orgs};

/// An opener of `kind` over the window's orgs, read through `orgs` each
/// time ⌘O asks (so switching org changes what it finds).
///
/// `items(org, query)` is the feature's own lister for one org; `open`
/// opens one of its items there. In All every org is asked and each
/// item's detail names its org. An item's id carries its org
/// ([`OrgId::path`]), so it opens in the org it came from even after the
/// window switched.
pub fn scoped_opener(
    kind: impl Into<String>,
    title: impl Into<String>,
    orgs: impl Fn() -> Orgs + 'static,
    items: impl Fn(&OrgId, &str) -> Vec<OpenItem> + 'static,
    open: impl Fn(&OrgId, &OpenItem, OpenIn) + 'static,
) -> Opener {
    Opener::new(
        kind,
        title,
        move |query| {
            let orgs = orgs();
            let scope = orgs.scope();
            let all = scope.is_all();
            scope
                .orgs()
                .iter()
                .flat_map(|org| {
                    let name = orgs.get(org).map(|o| o.name.clone()).unwrap_or_default();
                    items(org, query).into_iter().map(move |mut item| {
                        item.id = org.path(&item.id);
                        if all {
                            item.detail = if item.detail.is_empty() {
                                name.clone()
                            } else {
                                format!("{name} · {}", item.detail)
                            };
                        }
                        item
                    })
                })
                .collect()
        },
        move |item, how| {
            if let Some((org, id)) = OrgId::of_path(&item.id) {
                let mut own = item.clone();
                id.clone_into(&mut own.id);
                open(&org, &own, how);
            }
        },
    )
}
