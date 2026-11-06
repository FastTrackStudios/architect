//! The Architect app's own tours played as tests, over the Architect app
//! (not the sandbox) on a copy of the demo vault
//! (`architect_sandbox_media::architect_app`): the capture tour and each
//! built-in extension's `[[tours]]` (Projects, Tasks, Time Tracker,
//! Navigator). Each
//! step's setup runs and its target must be on screen, so a tour that
//! rots fails here. A process of its own: it makes the process the
//! Architect app for good.

#![allow(clippy::panic)]

#[test]
fn every_architect_tour_plays_as_a_scenario() {
    let _ = architect_sandbox_media::architect_app();
    let tours = architect_app::builtin_tours().unwrap_or_else(|e| panic!("{e}"));
    for want in [
        "capture-tour",
        "projects-tour",
        "tasks-tour",
        "time-tour",
        "navigator-tour",
    ] {
        assert!(
            tours.iter().any(|(_, s)| s.id == want),
            "no tour {want} among {:?}",
            tours.iter().map(|(_, s)| &s.id).collect::<Vec<_>>()
        );
    }
    let failed: Vec<String> = tours
        .iter()
        .filter_map(|(owner, scenario)| {
            architect_guide::run::run(&architect_sandbox_media::architect_app(), scenario, None)
                .err()
                .map(|e| format!("{owner}'s {}: {e}", scenario.id))
        })
        .collect();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
