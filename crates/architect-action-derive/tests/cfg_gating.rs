//! Proves `#[cfg(...)]` on an `#[action]`-decorated method is respected:
//! a method disabled by a feature flag produces no `ActionMeta` and no
//! dangling registration call referencing a method rustc already
//! stripped from the trait — exactly as if the method were never
//! declared at all.
//!
//! Run both ways to exercise both states of the same trait:
//!
//!   cargo test -p architect-action-derive --test cfg_gating
//!   cargo test -p architect-action-derive --test cfg_gating --features test-gated
//!
//! If the cfg passthrough were broken, the `test-gated`-off run wouldn't
//! even compile: `register_cfg_gated_actions` would still try to
//! call `CfgGatedActions::gated_on`, a method rustc has already removed
//! from the trait. So the mere fact both feature states compile is part
//! of the proof; the two tests below additionally check `all()`'s
//! contents are exactly right in each state.

use architect::action::ActionBackend;
use architect::actions;

#[actions(namespace = "CFGTEST")]
trait CfgGatedActions {
    #[action(
        description = "Always present regardless of feature state",
        category = "Test",
        group = "Always"
    )]
    fn always_on(&self);

    #[cfg(feature = "test-gated")]
    #[action(
        description = "Only present when the test-gated feature is enabled",
        category = "Test",
        group = "Gated"
    )]
    fn gated_on(&self);
}

struct Impl;

impl CfgGatedActions for Impl {
    fn always_on(&self) {}

    #[cfg(feature = "test-gated")]
    fn gated_on(&self) {}
}

/// Stand-in `ActionBackend` — just needs to exist so
/// `register_cfg_gated_actions` can be called (proving the
/// register call sites compile in lockstep with the const/method too,
/// not just `all()`).
struct NoopBackend;
impl ActionBackend for NoopBackend {
    fn register(
        &self,
        _meta: &'static architect::action::ActionMeta,
        _handler: std::sync::Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    ) {
    }
}

#[cfg(not(feature = "test-gated"))]
#[test]
fn gated_method_absent_when_feature_off() {
    let ids: Vec<_> = CfgGatedActionsActions::all().iter().map(|m| m.id).collect();
    assert_eq!(ids, ["CFGTEST_ALWAYS_ON"]);

    register_cfg_gated_actions(&NoopBackend, std::sync::Arc::new(Impl));
}

#[cfg(feature = "test-gated")]
#[test]
fn gated_method_present_when_feature_on() {
    let ids: Vec<_> = CfgGatedActionsActions::all().iter().map(|m| m.id).collect();
    assert_eq!(ids, ["CFGTEST_ALWAYS_ON", "CFGTEST_GATED_ON"]);

    register_cfg_gated_actions(&NoopBackend, std::sync::Arc::new(Impl));
}
