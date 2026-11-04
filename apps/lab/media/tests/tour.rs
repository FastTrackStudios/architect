//! The Architect tour (`apps/lab/scenarios/architect-tour.styx`) played as
//! the lab plays it: started from the Action List, moved on with Next, and
//! each chapter, once its setup has run, on show with its target found.
//! Then Back runs a chapter's setup again, and a start at a step (where
//! "Take the tour" resumes) begins at that chapter.

#![allow(clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;

const TOUR: &str = include_str!("../../scenarios/architect-tour.styx");

/// The chapters a person sees, by step id, in order.
fn chapters() -> Vec<String> {
    let tour = architect_guide::load(TOUR).expect("the tour reads");
    assert_eq!(tour.id, "architect-tour");
    tour.steps
        .iter()
        .filter(|s| !s.hide)
        .map(|s| s.id.clone())
        .collect()
}

/// A step waiting until the tour shows `chapter`, its target on screen.
fn on_show(out: &mut String, id: &str, chapter: &str, index: usize, of: usize) {
    let _ = writeln!(
        out,
        "    {{id {id}, done {{check ({{id GUIDE_TOUR_STATE, has {{scenario architect-tour, step {chapter}, index {index}, of {of}, found true}}}}), timeout 10}}}}"
    );
}

fn play(steps: &str) {
    let src =
        format!("id architect-tour-walk\nview {{width 1440, height 860}}\nsteps (\n{steps})\n");
    let scenario = architect_guide::load(&src).expect("the walk reads");
    architect_guide::run::run(&architect_lab_media::app(), &scenario, None)
        .unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn every_chapter_opens_with_its_target_on_screen() {
    let chapters = chapters();
    assert!(chapters.len() >= 20, "only {} chapters", chapters.len());
    let of = chapters.len();
    let mut steps =
        String::from("    {id start, do @run{id GUIDE_TOUR_START, args {id architect-tour}}}\n");
    for (n, chapter) in chapters.iter().enumerate() {
        on_show(&mut steps, &format!("see-{chapter}"), chapter, n + 1, of);
        let _ = writeln!(
            steps,
            "    {{id next-{chapter}, do @run{{id GUIDE_TOUR_NEXT}}}}"
        );
    }
    steps.push_str("    {id ended, done {check ({id GUIDE_TOUR_STATE, has {scenario \"\"}})}}\n");
    play(&steps);
}

#[test]
fn back_sets_a_chapter_up_again_and_a_start_resumes_at_one() {
    let chapters = chapters();
    let of = chapters.len();
    let at = |id: &str| chapters.iter().position(|c| c == id).expect(id) + 1;
    let mut steps = String::new();
    // Resume at the pages chapter: its setup (the Board's page) runs.
    steps.push_str(
        "    {id resume, do @run{id GUIDE_TOUR_START, args {id architect-tour, step pages}}}\n",
    );
    on_show(&mut steps, "at-pages", "pages", at("pages"), of);
    // Back to settings: the settings window opens again.
    steps.push_str("    {id back, do @run{id GUIDE_TOUR_BACK}}\n");
    on_show(&mut steps, "at-settings", "settings", at("settings"), of);
    steps.push_str("    {id skip, do @run{id GUIDE_TOUR_SKIP}}\n");
    steps.push_str("    {id left, done {check ({id GUIDE_TOUR_STATE, has {scenario \"\"}})}}\n");
    play(&steps);
}
