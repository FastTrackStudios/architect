//! `Bases`, linked in: it claims ```` ```base ```` fences, and lets them go
//! when it is off.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use architect::action::ActionList;
use architect_bases::ui::{Bases, ID};
use architect_extensions::contrib::Claim;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_settings::Registry;

#[test]
fn bases_claims_the_base_fence_while_it_is_on() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list, SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&Bases)
        .unwrap();
    let _ = ext.sync();
    let fence = Claim::fence("base").unwrap();
    let spec = ext.host().embeds().resolve(&fence).expect("claimed");
    assert_eq!(spec.extension, ID);
    assert!(ext.host().linked_view(ID, "fence").is_some());
    let _ = ext.disable(ID);
    assert!(ext.host().embeds().resolve(&fence).is_none());
}
