//! Every sandbox scenario (`apps/sandbox/scenarios`), the app's tours
//! (`apps/ui/tours`) and the tours of the sandbox's own extensions
//! (`[[tours]]`: the Board's) played as tests: no frames, each
//! step's `done` asserted and each step's target found. A failure names
//! the scenario, the step, and what its check's action gave back. The
//! Architect app's own (Projects, Tasks, Navigator, capture) play in
//! `tests/architect_tours.rs`.

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
    // The tours of the sandbox's own extensions (the Board's), as it runs
    // them. Architect's (the Navigator's, over a vault on disk) play in
    // tests/architect_tours.rs.
    let tours = architect_app::builtin_tours().unwrap_or_else(|e| panic!("{e}"));
    let ours: Vec<_> = tours
        .iter()
        .filter(|(owner, _)| owner.starts_with("lab."))
        .collect();
    assert!(
        ours.iter().any(|(owner, _)| owner == "lab.board"),
        "the Board ships a tour"
    );
    let failed: Vec<String> = ours
        .iter()
        .filter_map(|(owner, scenario)| {
            architect_guide::run::run(&architect_sandbox_media::app(), scenario, None)
                .err()
                .map(|e| format!("{owner}'s {}: {e}", scenario.id))
        })
        .collect();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
