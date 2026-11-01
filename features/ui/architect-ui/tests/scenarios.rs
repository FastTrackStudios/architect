//! The stories' guide scenarios (`#[story(scenario = "…")]`), played by
//! architect-guide's runner over each story alone: no frames, each step's
//! `done` asserted, every target found.

#[test]
fn every_story_scenario_plays() {
    architect_ui::stories::force_link();
    let stories: Vec<_> = architect_guide::story::with_scenarios().collect();
    assert!(
        stories
            .iter()
            .any(|s| s.name == "default" && s.category == Some("Collapsible")),
        "the Collapsible story carries its scenario"
    );
    match architect_guide::story::test_stories(stories) {
        Ok(n) => assert!(n >= 1, "no story scenarios ran"),
        Err(failed) => panic!("{failed}"),
    }
}
