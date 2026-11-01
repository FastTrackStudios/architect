//! Every lab scenario (`apps/lab/scenarios`) played as a test: no frames,
//! each step's `done` asserted. A failure names the scenario, the step,
//! and what its check's action gave back.

#[test]
fn every_lab_scenario_plays() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scenarios");
    match architect_guide::run::test_dir(architect_lab_media::app, &dir) {
        Ok(n) => assert!(n >= 3, "only {n} scenarios in {}", dir.display()),
        Err(failed) => panic!("{failed}"),
    }
}
