//! Every sandbox scenario (`apps/sandbox/scenarios`) and the app's tours
//! (`apps/ui/tours`) played as tests: no frames, each step's `done`
//! asserted. A failure names the scenario, the step, and what its check's
//! action gave back.

#[test]
fn every_sandbox_scenario_plays() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scenarios");
    match architect_guide::run::test_dir(architect_sandbox_media::app, &dir) {
        Ok(n) => assert!(n >= 3, "only {n} scenarios in {}", dir.display()),
        Err(failed) => panic!("{failed}"),
    }
}

#[test]
fn every_tour_plays_as_a_scenario() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/tours");
    match architect_guide::run::test_dir(architect_sandbox_media::app, &dir) {
        Ok(n) => assert!(n >= 1, "no tour in {}", dir.display()),
        Err(failed) => panic!("{failed}"),
    }
}
