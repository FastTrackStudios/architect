//! Mealplan linked into a host beside the Cookbook, over the app's
//! documents, as the app links them: a week planned by recipe name,
//! the pantry stocked, a substitution, the grocery list built from the
//! Cookbook's `ingredients` (scaled, a nested recipe cooked out) and
//! checked as a note, an item ticked as bought into the pantry, and the
//! list built again with the tick kept.
//!
//! Run with nextest: the extensions keep process-wide statics.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp,
    clippy::needless_pass_by_value,
    clippy::many_single_char_names
)]

use std::path::Path;

use architect::action::{ActionList, Principal};
use architect_extensions::documents::{CrdtDocuments, DocEdit, Documents as _};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_mealplan::{ID, Mealplan};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

fn try_run(list: &ActionList, id: &str, args: Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

fn docs() -> CrdtDocuments {
    let docs = CrdtDocuments::new();
    let app = Principal::Service {
        name: "test".to_owned(),
    };
    for (path, text) in [
        (
            "Cookbook/Pancakes.cook",
            "---\nservings: 2\n---\nWhisk @flour{200%g}, @milk{300%ml} and @eggs{2} with @salt{}.\n",
        ),
        (
            "Cookbook/Hot Honey.cook",
            "---\nservings: 6\n---\nWarm @honey{120%g} with @chilli flakes{1%tsp}.\n",
        ),
        (
            "Cookbook/Chicken and Waffles.cook",
            "---\nservings: 4\n---\nFry @chicken thighs{8}, dredged in @flour{0.5%kg}.\n\nDrizzle [[Hot Honey]]{3} over.\n",
        ),
    ] {
        docs.create(path, text, &app).unwrap();
    }
    docs
}

fn extensions(tmp: &Path, docs: &CrdtDocuments) -> (Extensions, ActionList) {
    let list = ActionList::new();
    let host =
        Host::new(list.clone(), SharedSections::new(Registry::new())).with_documents(docs.clone());
    let ext = Extensions::new(host, Installs::new(tmp.join("installs")))
        .with_linked(&architect_cookbook::Cookbook)
        .unwrap()
        .with_linked(&Mealplan)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    (ext, list)
}

const WEEK: &str = "2030-W10";

#[test]
fn a_week_planned_and_its_grocery_list_built() {
    let tmp = tempfile::tempdir().unwrap();
    let docs = docs();
    let (ext, list) = extensions(tmp.path(), &docs);
    assert!(
        ext.host().get(ID).is_some(),
        "Mealplan loaded beside the Cookbook"
    );

    // Planned by name: the Cookbook finds the path.
    run(
        &list,
        "MEALPLAN_PLAN",
        json!({ "date": "2030-03-04", "meal": "breakfast", "recipe": "Pancakes", "servings": 4 }),
    );
    let week = run(
        &list,
        "MEALPLAN_PLAN",
        json!({ "date": "2030-03-06", "meal": "supper", "recipe": "Chicken and Waffles", "servings": 4 }),
    );
    assert_eq!(week["week"], WEEK);
    assert_eq!(
        week["days"][0]["meals"][0]["recipe"],
        "Cookbook/Pancakes.cook"
    );
    assert_eq!(week["days"][2]["meals"][0]["meal"], "dinner");
    assert_eq!(
        docs.read("Meal plans/2030-W10.md").unwrap(),
        "---\ntype: meal-plan\nweek: 2030-W10\nmeals:\n  - date: 2030-03-04\n    meal: breakfast\n    recipe: Cookbook/Pancakes.cook\n    servings: 4\n  - date: 2030-03-06\n    meal: dinner\n    recipe: Cookbook/Chicken and Waffles.cook\n    servings: 4\n---\n# Meals, 2030-W10\n"
    );
    assert!(
        try_run(
            &list,
            "MEALPLAN_PLAN",
            json!({ "date": "2030-03-04", "recipe": "Toast" })
        )
        .is_err()
    );

    // The pantry: enough flour and milk, salt always there, golden syrup
    // that stands in for honey.
    run(
        &list,
        "MEALPLAN_STOCK",
        json!({ "name": "Flour", "qty": 1, "unit": "kg", "aisle": "Baking" }),
    );
    run(
        &list,
        "MEALPLAN_STOCK",
        json!({ "name": "Milk", "qty": 1, "unit": "l" }),
    );
    run(
        &list,
        "MEALPLAN_STOCK",
        json!({ "name": "Salt", "staple": true }),
    );
    run(
        &list,
        "MEALPLAN_STOCK",
        json!({ "name": "Golden syrup", "qty": 500, "unit": "g" }),
    );
    assert_eq!(
        docs.read("Pantry/Flour.md").unwrap(),
        "---\ntype: pantry\nqty: 1\nunit: kg\naisle: Baking\n---\n# Flour\n"
    );
    run(
        &list,
        "MEALPLAN_SUBSTITUTE",
        json!({ "from": "honey", "to": "golden syrup" }),
    );
    assert_eq!(
        run(&list, "MEALPLAN_PANTRY", Value::Null)
            .as_array()
            .unwrap()
            .len(),
        4
    );

    // Pancakes for 4 (400 g flour, 600 ml milk, 4 eggs, salt) and
    // chicken and waffles for 4 (8 thighs, 0.5 kg flour, and half a Hot
    // Honey: 60 g honey, 0.5 tsp chilli flakes).
    let g = run(&list, "MEALPLAN_GROCERIES", json!({ "week": WEEK }));
    assert_eq!(g["path"], "Groceries/2030-W10.md");
    assert_eq!(g["substituted"], json!(["honey → golden syrup"]));
    let text = docs.read("Groceries/2030-W10.md").unwrap();
    let body = text.split("\n---\n").nth(1).unwrap();
    assert_eq!(
        body,
        "# Groceries, 2030-W10\n\n## Meat & fish\n- [ ] 8 chicken thighs\n\n## Dairy & eggs\n- [ ] 4 eggs\n\n## Spices\n- [ ] 0.5 tsp chilli flakes\n",
        "{text}"
    );

    // Read as it is, not built again.
    let as_is = run(&list, "MEALPLAN_GROCERY_LIST", json!({ "week": WEEK }));
    assert_eq!(as_is["text"], text.as_str());
    assert!(
        try_run(
            &list,
            "MEALPLAN_GROCERY_LIST",
            json!({ "week": "2030-W11" })
        )
        .is_err()
    );

    // Bought: ticked, and into the pantry.
    run(
        &list,
        "MEALPLAN_CHECK",
        json!({ "week": WEEK, "item": "eggs", "bought": true }),
    );
    let eggs = run(&list, "MEALPLAN_PANTRY", json!({ "text": "egg" }));
    assert_eq!(eggs[0]["qty"], 4.0, "{eggs}");
    assert!(
        docs.read("Groceries/2030-W10.md")
            .unwrap()
            .contains("- [x] 4 eggs\n")
    );

    // A line written by hand, then the list built again: the eggs are in
    // the pantry now, the tick and the hand-written line stay.
    let t = docs.read("Groceries/2030-W10.md").unwrap();
    let app = Principal::Service {
        name: "test".to_owned(),
    };
    docs.edit(
        "Groceries/2030-W10.md",
        &[DocEdit::Replace {
            text: t.replace("## Spices\n", "## Spices\n- [ ] smoked salt\n"),
        }],
        "by hand",
        &app,
    )
    .unwrap();
    let g = run(&list, "MEALPLAN_GROCERIES", json!({ "week": WEEK }));
    let text = g["text"].as_str().unwrap();
    assert!(text.contains("- [x] 4 eggs\n"), "{text}");
    assert!(text.contains("- [ ] smoked salt\n"), "{text}");
    assert!(text.contains("- [ ] 8 chicken thighs\n"), "{text}");

    // Can I cook pancakes for 2 now? Flour, milk, eggs and salt are in.
    let f = run(
        &list,
        "MEALPLAN_CAN_COOK",
        json!({ "recipe": "Pancakes", "servings": 2 }),
    );
    assert_eq!(f["canCook"], true, "{f}");

    // Moved and taken off: no Day Plans, no Calendar here, so it says so.
    let s = run(
        &list,
        "MEALPLAN_SCHEDULE",
        json!({ "date": "2030-03-04", "meal": "breakfast" }),
    );
    assert!(s["block"].is_null() && s["event"].is_null(), "{s}");
    run(
        &list,
        "MEALPLAN_MOVE_MEAL",
        json!({ "date": "2030-03-04", "meal": "breakfast", "to_date": "2030-03-05" }),
    );
    let w = run(
        &list,
        "MEALPLAN_UNPLAN",
        json!({ "date": "2030-03-05", "meal": "breakfast" }),
    );
    assert!(w["days"][0]["meals"].as_array().unwrap().is_empty());
    assert!(w["days"][1]["meals"].as_array().unwrap().is_empty());
}
