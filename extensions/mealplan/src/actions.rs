//! The `MEALPLAN_*` actions: JSON in and out, each one call of the
//! `Meals` client (its method `m` is `MEALPLAN_<M>`, the
//! `architect.mealplan` service other extensions call).

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::client::client;
use crate::service::{
    Check, MealRef, MealplanError, MoveTo, PantrySearch, PlannedMeal, Range, ScheduleHow, Stock,
    Substitution,
};

/// `args` as `T`; `null` as its default.
fn args<T: DeserializeOwned + Default>(v: Value) -> Result<T, String> {
    if v.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(v).map_err(|e| format!("arguments: {e}"))
}

fn out<T: Serialize>(v: Result<T, MealplanError>) -> Result<Value, String> {
    v.map_err(|e| e.to_string())
        .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn number(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(crate::model::number_of)
}

/// `MEALPLAN_WEEK {week?, date?}`.
pub async fn week(a: Value) -> Result<Value, String> {
    let r: Range = args(a)?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.week(r).await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_MEALS {from?, to?}`.
pub async fn meals(a: Value) -> Result<Value, String> {
    let r: Range = args(a)?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.meals(r).await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_PLAN {date?, meal?, recipe, servings?, note?}`.
pub async fn plan(a: Value) -> Result<Value, String> {
    let recipe = text(&a, "recipe").ok_or("give the `recipe`: a .cook path or its name")?;
    let m = PlannedMeal {
        date: text(&a, "date").unwrap_or_default(),
        meal: text(&a, "meal").unwrap_or_default(),
        recipe,
        servings: number(&a, "servings"),
        note: text(&a, "note"),
        ..PlannedMeal::default()
    };
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.plan(m).await.map_err(architect::vox_into_app))
}

fn meal_ref(a: &Value) -> Result<MealRef, String> {
    Ok(MealRef {
        date: text(a, "date").ok_or("give the meal's `date`")?,
        meal: text(a, "meal").ok_or("give the `meal`: breakfast, lunch, dinner or snack")?,
        recipe: text(a, "recipe"),
    })
}

/// `MEALPLAN_UNPLAN {date, meal, recipe?}`.
pub async fn unplan(a: Value) -> Result<Value, String> {
    let r = meal_ref(&a)?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.unplan(r).await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_MOVE_MEAL {date, meal, recipe?, to_date?, to_meal?}`.
pub async fn move_meal(a: Value) -> Result<Value, String> {
    let r = meal_ref(&a)?;
    let to = MoveTo {
        date: text(&a, "to_date"),
        meal: text(&a, "to_meal"),
    };
    if to == MoveTo::default() {
        return Err("give where it goes: `to_date`, `to_meal`".into());
    }
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.move_meal(r, to).await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_SCHEDULE {date, meal, recipe?, block?, time?, event?}`.
pub async fn schedule(a: Value) -> Result<Value, String> {
    let r = meal_ref(&a)?;
    let how = ScheduleHow {
        block: text(&a, "block"),
        time: text(&a, "time"),
        event: a.get("event").and_then(Value::as_bool).unwrap_or(false),
    };
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.schedule(r, how).await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_PANTRY {text?, aisle?, low?, expired?}`.
pub async fn pantry(a: Value) -> Result<Value, String> {
    let s: PantrySearch = args(a)?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.pantry(s).await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_STOCK {name, qty?, add?, unit?, aisle?, staple?, min?, expires?, aliases?}`.
pub async fn stock(a: Value) -> Result<Value, String> {
    if text(&a, "name").is_none() {
        return Err("give the item's `name`".into());
    }
    let s: Stock = args(a)?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.stock(s).await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_REMOVE_ITEM {name}`.
pub async fn remove_item(a: Value) -> Result<Value, String> {
    let name = text(&a, "name").ok_or("give the item's `name`")?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c
        .remove_item(name.clone())
        .await
        .map_err(architect::vox_into_app)
        .map(|()| json!({ "removed": name })))
}

/// `MEALPLAN_SUBSTITUTIONS`.
pub async fn substitutions(_: Value) -> Result<Value, String> {
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.substitutions().await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_SUBSTITUTE {from, to, ratio?, always?, note?}`.
pub async fn substitute(a: Value) -> Result<Value, String> {
    if text(&a, "from").is_none() || text(&a, "to").is_none() {
        return Err("give `from` and `to`".into());
    }
    let s: Substitution = args(a)?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.substitute(s).await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_UNSUBSTITUTE {from, to?}`.
pub async fn unsubstitute(a: Value) -> Result<Value, String> {
    let from = text(&a, "from").ok_or("give `from`")?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c
        .unsubstitute(from, text(&a, "to"))
        .await
        .map_err(architect::vox_into_app))
}

/// `MEALPLAN_GROCERIES {week?, date?, from?, to?}`.
pub async fn groceries(a: Value) -> Result<Value, String> {
    let r: Range = args(a)?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.groceries(r).await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_CHECK {item, week?, path?, checked?, bought?}`.
pub async fn check(a: Value) -> Result<Value, String> {
    if text(&a, "item").is_none() {
        return Err("give the `item`: its name or its line".into());
    }
    let ch: Check = serde_json::from_value(a).map_err(|e| format!("arguments: {e}"))?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c.check(ch).await.map_err(architect::vox_into_app))
}

/// `MEALPLAN_CAN_COOK {recipe, servings?}`.
pub async fn can_cook(a: Value) -> Result<Value, String> {
    let recipe = text(&a, "recipe").ok_or("give the `recipe`: a .cook path or its name")?;
    let c = client().await.map_err(|e| e.to_string())?;
    out(c
        .can_cook(recipe, number(&a, "servings"))
        .await
        .map_err(architect::vox_into_app))
}
