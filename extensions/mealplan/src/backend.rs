//! [`Backend`], the one implementation of [`Meals`]: notes in the
//! vault and the Cookbook's recipes, reached through [`Files`].

use chrono::{Duration, Local, NaiveDate};
use serde_json::{Value, json};

use crate::files::{CALENDAR, DAYPLAN, Files};
use crate::groceries::{For, build, read_list, render, tick};
use crate::model::{
    self, Meal, date, file_name, fold, is_item, item_of, meal, monday, properties, range,
    read_meals, read_subs, sort_meals, stem, week_id, write_item, write_plan, write_subs,
};
use crate::service::{
    Check, Day, Fulfilment, Groceries, MealRef, MealplanError, Meals, MoveTo, PantryItem,
    PantrySearch, PlannedMeal, Range, ScheduleHow, Scheduled, Stock, Substitution, Week,
};
use crate::units::convert;

/// Where its notes go unless its settings say otherwise.
pub const PLANS: &str = "Meal plans";
pub const PANTRY: &str = "Pantry";
pub const GROCERIES: &str = "Groceries";
/// The substitutions' note, in the pantry's folder.
pub const SUBSTITUTIONS: &str = "Substitutions.md";
/// How many a meal is for unless said (`mealplan.servings`).
pub const SERVINGS: f64 = 2.0;

/// Mealplan over the vault's notes and the Cookbook. Cheap to clone.
#[derive(Clone, architect::HasDispatcher)]
pub struct Backend {
    files: Files,
    today: Option<NaiveDate>,
}

impl architect::Services for Backend {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![crate::service::MealsService]
    }
}

fn weekday(d: NaiveDate) -> &'static str {
    use chrono::Datelike as _;
    match d.weekday() {
        chrono::Weekday::Mon => "monday",
        chrono::Weekday::Tue => "tuesday",
        chrono::Weekday::Wed => "wednesday",
        chrono::Weekday::Thu => "thursday",
        chrono::Weekday::Fri => "friday",
        chrono::Weekday::Sat => "saturday",
        chrono::Weekday::Sun => "sunday",
    }
}

/// The index in `meals` of the one `r` names.
fn find(meals: &[PlannedMeal], r: &MealRef, day: NaiveDate) -> Result<usize, MealplanError> {
    let m = meal(&r.meal)?;
    let d = day.to_string();
    let recipe = r.recipe.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let hits: Vec<usize> = meals
        .iter()
        .enumerate()
        .filter(|(_, x)| x.date == d && Meal::parse(&x.meal) == Some(m))
        .filter(|(_, x)| {
            recipe.is_none_or(|r| {
                x.recipe == r || fold(&x.name) == fold(r) || fold(&stem(r)) == fold(&x.name)
            })
        })
        .map(|(i, _)| i)
        .collect();
    match hits.as_slice() {
        [i] => Ok(*i),
        [] => Err(MealplanError::NotFound(format!(
            "no {} planned on {d}{}",
            m.as_str(),
            recipe.map(|r| format!(" with {r}")).unwrap_or_default()
        ))),
        _ => Err(MealplanError::Invalid(format!(
            "{d}'s {} has more than one recipe: say which (`recipe`)",
            m.as_str()
        ))),
    }
}

impl Backend {
    #[must_use]
    pub const fn new(files: Files) -> Self {
        Self { files, today: None }
    }

    /// As if it were `today` (tests).
    #[must_use]
    pub const fn on(mut self, today: NaiveDate) -> Self {
        self.today = Some(today);
        self
    }

    fn today(&self) -> NaiveDate {
        self.today.unwrap_or_else(|| Local::now().date_naive())
    }

    async fn folder(&self, key: &str, default: &str) -> String {
        self.files
            .setting(key)
            .await
            .as_str()
            .map(|f| f.trim().trim_matches('/').to_owned())
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| default.to_owned())
    }

    async fn plan_path(&self, week: &str) -> String {
        format!(
            "{}/{week}.md",
            self.folder("mealplan.plans_folder", PLANS).await
        )
    }

    async fn pantry_folder(&self) -> String {
        self.folder("mealplan.pantry_folder", PANTRY).await
    }

    async fn subs_path(&self) -> String {
        format!("{}/{SUBSTITUTIONS}", self.pantry_folder().await)
    }

    async fn groceries_path(&self, title: &str) -> String {
        format!(
            "{}/{}.md",
            self.folder("mealplan.groceries_folder", GROCERIES).await,
            file_name(title)
        )
    }

    /// The week `day` is in: its id, note path, text and meals.
    async fn load(
        &self,
        day: NaiveDate,
    ) -> Result<(String, String, Option<String>, Vec<PlannedMeal>), MealplanError> {
        let week = week_id(day);
        let path = self.plan_path(&week).await;
        let text = self.files.read(&path).await?;
        let meals = text.as_deref().map(read_meals).unwrap_or_default();
        Ok((week, path, text, meals))
    }

    async fn save(
        &self,
        week: &str,
        path: &str,
        text: Option<&str>,
        meals: &mut [PlannedMeal],
        step: &str,
    ) -> Result<(), MealplanError> {
        sort_meals(meals);
        self.files
            .write(path, &write_plan(text, week, meals), step)
            .await
    }

    async fn week_of(&self, day: NaiveDate) -> Result<Week, MealplanError> {
        let m = monday(day);
        let (week, path, _, meals) = self.load(m).await?;
        let days = (0..7)
            .map(|i| {
                let d = m + Duration::days(i);
                let ds = d.to_string();
                Day {
                    date: ds.clone(),
                    weekday: weekday(d).to_owned(),
                    meals: meals.iter().filter(|x| x.date == ds).cloned().collect(),
                }
            })
            .collect();
        Ok(Week {
            week,
            path,
            from: m.to_string(),
            to: (m + Duration::days(7)).to_string(),
            days,
        })
    }

    async fn meals_in(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<PlannedMeal>, MealplanError> {
        let mut out = Vec::new();
        let mut m = monday(from);
        while m < to {
            let (_, _, _, meals) = self.load(m).await?;
            out.extend(meals.into_iter().filter(|x| {
                NaiveDate::parse_from_str(&x.date, "%Y-%m-%d").is_ok_and(|d| d >= from && d < to)
            }));
            m += Duration::days(7);
        }
        Ok(out)
    }

    /// The pantry's items, by name (not `Substitutions.md`, nor a note
    /// of another `type`).
    async fn items(&self) -> Result<Vec<PantryItem>, MealplanError> {
        let folder = self.pantry_folder().await;
        let subs = self.subs_path().await;
        let today = self.today();
        let mut out = Vec::new();
        for (path, indexed) in self.files.under(&folder).await? {
            if path == subs {
                continue;
            }
            let props = match self.files.read(&path).await {
                Ok(Some(t)) => properties(&t),
                _ => indexed,
            };
            if props
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| t != "pantry")
            {
                continue;
            }
            out.push(item_of(&path, &props, today));
        }
        out.sort_by_key(|i| (fold(&i.name), i.path.clone()));
        Ok(out)
    }

    async fn subs(&self) -> Result<(String, Option<String>, Vec<Substitution>), MealplanError> {
        let path = self.subs_path().await;
        let text = self.files.read(&path).await?;
        let subs = text.as_deref().map(read_subs).unwrap_or_default();
        Ok((path, text, subs))
    }

    async fn servings(&self, given: Option<f64>) -> Option<f64> {
        if let Some(s) = given.filter(|s| *s > 0.0) {
            return Some(s);
        }
        match self.files.setting("mealplan.servings").await {
            Value::Null => Some(SERVINGS),
            v => model::number_of(&v).filter(|s| *s > 0.0),
        }
    }

    /// The Day Plans meal block for `m` on `day`'s plan.
    fn block_for(day: &Value, m: Meal, wanted: Option<&str>) -> Option<String> {
        let blocks: Vec<&Value> = day
            .get("blocks")
            .and_then(Value::as_array)?
            .iter()
            .collect();
        let text = |b: &Value, k: &str| {
            b.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        if let Some(w) = wanted {
            return blocks
                .iter()
                .find(|b| text(b, "id") == w)
                .map(|b| text(b, "id"));
        }
        let meals: Vec<&&Value> = blocks
            .iter()
            .filter(|b| text(b, "category") == "meal")
            .collect();
        let name = m.as_str();
        if let Some(b) = meals
            .iter()
            .find(|b| fold(&text(b, "label")).contains(name) || fold(&text(b, "id")).contains(name))
        {
            return Some(text(b, "id"));
        }
        // By when it starts: breakfast before 11, lunch before 15, a
        // snack before 17, dinner after.
        let (lo, hi) = match m {
            Meal::Breakfast => ("00:00", "11:00"),
            Meal::Lunch => ("11:00", "15:00"),
            Meal::Snack => ("15:00", "17:00"),
            Meal::Dinner => ("17:00", "24:00"),
        };
        meals
            .iter()
            .find(|b| {
                let s = text(b, "start");
                s.as_str() >= lo && s.as_str() < hi
            })
            .map(|b| text(b, "id"))
    }

    /// Moves or cancels a meal's event; a Calendar that refuses is no
    /// failure of the plan's.
    async fn follow_event(&self, event: Option<&str>, to: Option<(&str, Meal)>) {
        let Some(path) = event else { return };
        let _ = match to {
            Some((d, m)) => {
                self.files
                    .service(
                        CALENDAR,
                        "reschedule",
                        json!({ "path": path, "start": format!("{d}T{}", m.time()) }),
                    )
                    .await
            }
            None => {
                self.files
                    .service(CALENDAR, "cancel", json!({ "path": path }))
                    .await
            }
        };
    }
}

impl Meals for Backend {
    async fn week(&self, r: Range) -> Result<Week, MealplanError> {
        let (from, _, _) = range(&r, self.today())?;
        self.week_of(from).await
    }

    async fn meals(&self, r: Range) -> Result<Vec<PlannedMeal>, MealplanError> {
        let today = self.today();
        let r = if r == Range::default() {
            Range {
                from: Some(today.to_string()),
                ..Range::default()
            }
        } else {
            r
        };
        let (from, to, _) = range(&r, today)?;
        self.meals_in(from, to).await
    }

    async fn plan(&self, mut m: PlannedMeal) -> Result<Week, MealplanError> {
        let day = date(&m.date, self.today())?;
        let kind = if m.meal.trim().is_empty() {
            Meal::Dinner
        } else {
            meal(&m.meal)?
        };
        m.recipe = self.files.recipe_path(&m.recipe).await?;
        m.date = day.to_string();
        kind.as_str().clone_into(&mut m.meal);
        m.name = stem(&m.recipe);
        m.servings = self.servings(m.servings).await;
        m.note = m.note.filter(|n| !n.trim().is_empty());
        m.event = None;
        let (week, path, text, mut meals) = self.load(day).await?;
        let step = format!("Mealplan: plan {} for {} {}", m.name, kind.as_str(), m.date);
        meals.push(m);
        self.save(&week, &path, text.as_deref(), &mut meals, &step)
            .await?;
        self.week_of(day).await
    }

    async fn unplan(&self, r: MealRef) -> Result<Week, MealplanError> {
        let day = date(&r.date, self.today())?;
        let (week, path, text, mut meals) = self.load(day).await?;
        let gone = meals.remove(find(&meals, &r, day)?);
        let step = format!("Mealplan: unplan {} on {}", gone.name, gone.date);
        self.save(&week, &path, text.as_deref(), &mut meals, &step)
            .await?;
        self.follow_event(gone.event.as_deref(), None).await;
        self.week_of(day).await
    }

    async fn move_meal(&self, r: MealRef, to: MoveTo) -> Result<Week, MealplanError> {
        let now = self.today();
        let day = date(&r.date, now)?;
        let to_day = match to.date.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(d) => date(d, now)?,
            None => day,
        };
        let to_meal = match to.meal.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(m) => meal(m)?,
            None => meal(&r.meal)?,
        };
        let (week, path, text, mut meals) = self.load(day).await?;
        let mut m = meals.remove(find(&meals, &r, day)?);
        let step = format!(
            "Mealplan: move {} to {} {}",
            m.name,
            to_meal.as_str(),
            to_day
        );
        m.date = to_day.to_string();
        to_meal.as_str().clone_into(&mut m.meal);
        let event = m.event.clone();
        if week_id(to_day) == week {
            meals.push(m);
            self.save(&week, &path, text.as_deref(), &mut meals, &step)
                .await?;
        } else {
            // Across weeks: the meal lands in the other week's note first,
            // so a failure there leaves it where it was.
            let (w2, p2, t2, mut there) = self.load(to_day).await?;
            there.push(m);
            self.save(&w2, &p2, t2.as_deref(), &mut there, &step)
                .await?;
            self.save(&week, &path, text.as_deref(), &mut meals, &step)
                .await?;
        }
        let d = to_day.to_string();
        self.follow_event(event.as_deref(), Some((&d, to_meal)))
            .await;
        self.week_of(to_day).await
    }

    async fn schedule(&self, r: MealRef, how: ScheduleHow) -> Result<Scheduled, MealplanError> {
        let day = date(&r.date, self.today())?;
        let (week, path, text, mut meals) = self.load(day).await?;
        let i = find(&meals, &r, day)?;
        let m = meals[i].clone();
        let kind = meal(&m.meal)?;
        let mut out = Scheduled::default();

        match self
            .files
            .service(DAYPLAN, "get", json!({ "date": m.date }))
            .await
        {
            None => out.notes.push("Day Plans is not installed".to_owned()),
            Some(Err(e)) => out.notes.push(format!("Day Plans: {e}")),
            Some(Ok(plan)) => match Self::block_for(&plan, kind, how.block.as_deref()) {
                None => out.notes.push(format!(
                    "{}'s day plan has no block for {}",
                    m.date,
                    kind.as_str()
                )),
                Some(block) => {
                    let assigned = self
                        .files
                        .service(
                            DAYPLAN,
                            "assign",
                            json!({ "date": m.date, "block": block, "kind": "recipe", "path": m.recipe, "title": m.name }),
                        )
                        .await;
                    match assigned {
                        Some(Ok(_)) => out.block = Some(block),
                        Some(Err(e)) => out.notes.push(format!("Day Plans: {e}")),
                        None => out.notes.push("Day Plans is not installed".to_owned()),
                    }
                }
            },
        }

        if how.event || out.block.is_none() {
            let time = how
                .time
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| kind.time());
            let made = self
                .files
                .service(
                    CALENDAR,
                    "create",
                    json!({
                        "title": format!("{}: {}", kind.title(), m.name),
                        "start": format!("{}T{time}", m.date),
                        "minutes": 60,
                        "links": [{ "relation": "recipe", "target": m.recipe }],
                        "external_ref": format!("mealplan:{}/{}/{}", m.date, kind.as_str(), m.recipe),
                    }),
                )
                .await;
            match made {
                None => out.notes.push("the Calendar is not installed".to_owned()),
                Some(Err(e)) => out.notes.push(format!("Calendar: {e}")),
                Some(Ok(v)) => {
                    let event = v
                        .get("path")
                        .or_else(|| v.get("event").and_then(|e| e.get("path")))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    if let Some(e) = &event
                        && m.event.as_deref() != Some(e)
                    {
                        meals[i].event = Some(e.clone());
                        let step = format!("Mealplan: {} on the calendar", m.name);
                        self.save(&week, &path, text.as_deref(), &mut meals, &step)
                            .await?;
                    }
                    out.event = event;
                }
            }
        }
        Ok(out)
    }

    async fn pantry(&self, s: PantrySearch) -> Result<Vec<PantryItem>, MealplanError> {
        let text = s.text.as_deref().map(fold).filter(|t| !t.is_empty());
        let aisle = s.aisle.as_deref().map(fold).filter(|t| !t.is_empty());
        Ok(self
            .items()
            .await?
            .into_iter()
            .filter(|i| {
                text.as_ref().is_none_or(|t| {
                    fold(&i.name).contains(t.as_str())
                        || i.aliases.iter().any(|a| fold(a).contains(t.as_str()))
                })
            })
            .filter(|i| {
                aisle
                    .as_ref()
                    .is_none_or(|a| i.aisle.as_deref().is_some_and(|x| fold(x) == *a))
            })
            .filter(|i| !s.low || i.low)
            .filter(|i| !s.expired || i.expired)
            .collect())
    }

    async fn stock(&self, c: Stock) -> Result<PantryItem, MealplanError> {
        let name = c.name.trim();
        if name.is_empty() {
            return Err(MealplanError::Invalid("give the item's `name`".to_owned()));
        }
        let today = self.today();
        let found = self.items().await?.into_iter().find(|i| is_item(i, name));
        let mut item = match found {
            Some(i) => i,
            None => PantryItem {
                path: format!("{}/{}.md", self.pantry_folder().await, file_name(name)),
                name: file_name(name),
                ..PantryItem::default()
            },
        };
        let given_unit = c.unit.as_deref().map(str::trim).map(str::to_owned);
        let unit = given_unit.clone().unwrap_or_else(|| item.unit.clone());
        if let Some(q) = c.qty {
            item.qty = Some(q.max(0.0));
            item.unit.clone_from(&unit);
        } else if let Some(u) = &given_unit
            && c.add.is_none()
        {
            item.unit.clone_from(u);
        }
        if let Some(a) = c.add {
            match item.qty {
                None => {
                    item.qty = Some(a.max(0.0));
                    item.unit.clone_from(&unit);
                }
                Some(q) => {
                    let add = convert(a, &unit, &item.unit).ok_or_else(|| {
                        MealplanError::Invalid(format!(
                            "{} is kept in {:?}: {a} {unit} does not convert",
                            item.name, item.unit
                        ))
                    })?;
                    item.qty = Some((q + add).max(0.0));
                }
            }
        }
        if let Some(a) = c.aisle {
            item.aisle = Some(a.trim().to_owned()).filter(|a| !a.is_empty());
        }
        if let Some(s) = c.staple {
            item.staple = s;
        }
        if let Some(m) = c.min {
            item.min = (m > 0.0).then_some(m);
        }
        if let Some(e) = c.expires {
            let e = e.trim();
            item.expires = if e.is_empty() {
                None
            } else {
                Some(date(e, today)?.to_string())
            };
        }
        if let Some(a) = c.aliases {
            item.aliases = a
                .into_iter()
                .map(|a| a.trim().to_owned())
                .filter(|a| !a.is_empty())
                .collect();
        }
        model::flags(&mut item, today);
        let existing = self.files.read(&item.path).await?;
        let step = format!("Mealplan: stock {}", item.name);
        self.files
            .write(&item.path, &write_item(existing.as_deref(), &item), &step)
            .await?;
        Ok(item)
    }

    async fn remove_item(&self, name: String) -> Result<(), MealplanError> {
        let item = self
            .items()
            .await?
            .into_iter()
            .find(|i| is_item(i, &name))
            .ok_or_else(|| MealplanError::NotFound(format!("no {name:?} in the pantry")))?;
        self.files.delete(&item.path).await
    }

    async fn substitutions(&self) -> Result<Vec<Substitution>, MealplanError> {
        Ok(self.subs().await?.2)
    }

    async fn substitute(&self, rule: Substitution) -> Result<Vec<Substitution>, MealplanError> {
        let rule = Substitution {
            from: rule.from.trim().to_owned(),
            to: rule.to.trim().to_owned(),
            note: rule.note.filter(|n| !n.trim().is_empty()),
            ..rule
        };
        if rule.from.is_empty() || rule.to.is_empty() || fold(&rule.from) == fold(&rule.to) {
            return Err(MealplanError::Invalid(
                "give a `from` and a different `to`".to_owned(),
            ));
        }
        if rule.ratio.is_nan() || rule.ratio <= 0.0 {
            return Err(MealplanError::Invalid("`ratio` is more than 0".to_owned()));
        }
        let (path, text, mut subs) = self.subs().await?;
        let step = format!("Mealplan: {} for {}", rule.to, rule.from);
        match subs
            .iter_mut()
            .find(|s| fold(&s.from) == fold(&rule.from) && fold(&s.to) == fold(&rule.to))
        {
            Some(s) => *s = rule,
            None => subs.push(rule),
        }
        self.files
            .write(&path, &write_subs(text.as_deref(), &subs), &step)
            .await?;
        Ok(subs)
    }

    async fn unsubstitute(
        &self,
        from: String,
        to: Option<String>,
    ) -> Result<Vec<Substitution>, MealplanError> {
        let (path, text, mut subs) = self.subs().await?;
        let before = subs.len();
        let to = to.filter(|t| !t.trim().is_empty());
        subs.retain(|s| {
            !(fold(&s.from) == fold(&from) && to.as_deref().is_none_or(|t| fold(&s.to) == fold(t)))
        });
        if subs.len() == before {
            return Err(MealplanError::NotFound(format!(
                "no substitution for {from:?}"
            )));
        }
        let step = format!("Mealplan: no substitute for {from}");
        self.files
            .write(&path, &write_subs(text.as_deref(), &subs), &step)
            .await?;
        Ok(subs)
    }

    async fn groceries(&self, r: Range) -> Result<Groceries, MealplanError> {
        let (from, to, week) = range(&r, self.today())?;
        let meals = self.meals_in(from, to).await?;
        let portions: Vec<(String, Option<f64>)> = meals
            .iter()
            .map(|m| (m.recipe.clone(), m.servings))
            .collect();
        let needs = self.files.ingredients(&portions).await?;
        let pantry = self.items().await?;
        let subs = self.subs().await?.2;
        let built = build(&needs, &pantry, &subs, from, true);
        let span = For { week, from, to };
        let path = self.groceries_path(&span.title()).await;
        let existing = self.files.read(&path).await?;
        let text = render(&span, &built, existing.as_deref());
        if existing.as_deref() != Some(text.as_str()) {
            let step = format!("Mealplan: groceries for {}", span.title());
            self.files.write(&path, &text, &step).await?;
        }
        Ok(read_list(&path, &text))
    }

    async fn grocery_list(&self, r: Range) -> Result<Groceries, MealplanError> {
        let (from, to, week) = range(&r, self.today())?;
        let title = For { week, from, to }.title();
        let path = self.groceries_path(&title).await;
        let text = self.files.read(&path).await?.ok_or_else(|| {
            MealplanError::NotFound(format!(
                "no grocery list for {title} yet: build it (MEALPLAN_GROCERIES)"
            ))
        })?;
        Ok(read_list(&path, &text))
    }

    async fn check(&self, c: Check) -> Result<Groceries, MealplanError> {
        let given = c.path.as_deref().map(str::trim).filter(|p| !p.is_empty());
        let path = if let Some(p) = given {
            p.to_owned()
        } else {
            let r = Range {
                week: c.week.clone(),
                ..Range::default()
            };
            let (from, to, week) = range(&r, self.today())?;
            self.groceries_path(&For { week, from, to }.title()).await
        };
        let text = self.files.read(&path).await?.ok_or_else(|| {
            MealplanError::NotFound(format!(
                "no grocery list {path}: build it with MEALPLAN_GROCERIES"
            ))
        })?;
        let (text, item) = tick(&text, &c.item, c.checked)
            .ok_or_else(|| MealplanError::NotFound(format!("no {:?} on {path}", c.item)))?;
        let step = format!(
            "Mealplan: {} {}",
            if c.checked { "tick" } else { "untick" },
            item.name
        );
        self.files.write(&path, &text, &step).await?;
        if c.bought && c.checked {
            let added = self
                .stock(Stock {
                    name: item.name.clone(),
                    add: Some(item.qty.unwrap_or(1.0)),
                    unit: Some(item.unit.clone()),
                    ..Stock::default()
                })
                .await;
            match added {
                Ok(_) | Err(MealplanError::Invalid(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(read_list(&path, &text))
    }

    async fn can_cook(
        &self,
        recipe: String,
        servings: Option<f64>,
    ) -> Result<Fulfilment, MealplanError> {
        let path = self.files.recipe_path(&recipe).await?;
        let needs = self.files.ingredients(&[(path.clone(), servings)]).await?;
        let pantry = self.items().await?;
        let subs = self.subs().await?.2;
        let built = build(&needs, &pantry, &subs, self.today(), false);
        Ok(Fulfilment {
            recipe: path,
            can_cook: built.items.is_empty(),
            have: built.have,
            missing: built.items.into_iter().map(|(_, i)| i).collect(),
            substituted: built.substituted,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::float_cmp)]
    use super::*;
    use crate::files::{Memory, MemoryRecipe};
    use crate::groceries::Need;

    fn block<F: std::future::Future>(f: F) -> F::Output {
        futures_lite_block(f)
    }

    /// A tiny executor: every future here is ready without waking.
    fn futures_lite_block<F: std::future::Future>(f: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let mut f = std::pin::pin!(f);
        loop {
            if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    fn need(name: &str, qty: f64, unit: &str) -> Need {
        Need {
            name: name.into(),
            qty: Some(qty),
            unit: unit.into(),
            ..Need::default()
        }
    }

    fn kitchen() -> (Backend, Files) {
        let mut m = Memory::default();
        m.recipes.insert(
            "Cookbook/Pancakes.cook".into(),
            MemoryRecipe {
                servings: 2.0,
                ingredients: vec![
                    need("flour", 200.0, "g"),
                    need("milk", 300.0, "ml"),
                    need("eggs", 2.0, ""),
                ],
            },
        );
        m.recipes.insert(
            "Cookbook/Bread.cook".into(),
            MemoryRecipe {
                servings: 1.0,
                ingredients: vec![need("flour", 0.5, "kg"), need("yeast", 7.0, "g")],
            },
        );
        let files = Files::Memory(std::sync::Arc::new(std::sync::Mutex::new(m)));
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        (Backend::new(files.clone()).on(today), files)
    }

    fn file(files: &Files, path: &str) -> String {
        block(files.read(path))
            .unwrap()
            .unwrap_or_else(|| panic!("no {path}"))
    }

    // Task's end_to_end, over notes: plan, stock, list, buy.
    #[test]
    fn a_week_planned_listed_and_bought() {
        let (b, files) = kitchen();
        let plan = |date: &str, meal: &str, recipe: &str, servings: Option<f64>| {
            block(b.plan(PlannedMeal {
                date: date.into(),
                meal: meal.into(),
                recipe: recipe.into(),
                servings,
                ..PlannedMeal::default()
            }))
            .unwrap()
        };
        plan("2026-11-16", "breakfast", "pancakes", Some(4.0));
        let week = plan("2026-11-17", "dinner", "Cookbook/Bread.cook", None);
        assert_eq!(week.week, "2026-W47");
        assert_eq!(week.days[0].meals[0].name, "Pancakes");
        assert_eq!(
            week.days[1].meals[0].servings,
            Some(2.0),
            "mealplan.servings by default"
        );
        assert!(
            file(&files, "Meal plans/2026-W47.md")
                .contains("    recipe: Cookbook/Pancakes.cook\n    servings: 4\n")
        );
        // Ambiguity and absence say so.
        assert!(matches!(
            block(b.plan(PlannedMeal {
                recipe: "nothing".into(),
                ..PlannedMeal::default()
            })),
            Err(MealplanError::NotFound(_))
        ));
        assert!(matches!(
            block(b.plan(PlannedMeal {
                recipe: "bread".into(),
                meal: "elevenses".into(),
                ..PlannedMeal::default()
            })),
            Err(MealplanError::Invalid(_))
        ));

        block(b.stock(Stock {
            name: "Flour".into(),
            qty: Some(0.5),
            unit: Some("kg".into()),
            aisle: Some("Baking".into()),
            ..Stock::default()
        }))
        .unwrap();
        block(b.stock(Stock {
            name: "Milk".into(),
            qty: Some(1.0),
            unit: Some("l".into()),
            ..Stock::default()
        }))
        .unwrap();
        let flour = block(b.stock(Stock {
            name: "flour".into(),
            add: Some(250.0),
            unit: Some("g".into()),
            ..Stock::default()
        }))
        .unwrap();
        assert_eq!(flour.qty, Some(0.75), "250 g added to 0.5 kg");
        assert!(
            block(b.stock(Stock {
                name: "flour".into(),
                add: Some(1.0),
                unit: Some("clove".into()),
                ..Stock::default()
            }))
            .is_err()
        );

        assert!(matches!(
            block(b.grocery_list(Range::default())),
            Err(MealplanError::NotFound(_))
        ));
        let list = block(b.groceries(Range::default())).unwrap();
        assert_eq!(block(b.grocery_list(Range::default())).unwrap(), list);
        assert_eq!(list.path, "Groceries/2026-W47.md");
        // Pancakes for 4: 400 g flour, 600 ml milk, 4 eggs; bread for 2:
        // 1 kg flour, 14 g yeast. Less 750 g flour and 1 l milk.
        let text = file(&files, &list.path);
        assert!(
            text.contains(
                "## Dairy & eggs\n- [ ] 4 eggs\n\n## Baking\n- [ ] 650 g flour\n- [ ] 14 g yeast\n"
            ),
            "{text}"
        );
        assert!(!text.contains("milk"), "{text}");

        let list = block(b.check(Check {
            item: "flour".into(),
            checked: true,
            bought: true,
            ..Check::default()
        }))
        .unwrap();
        assert!(
            list.aisles
                .iter()
                .flat_map(|a| &a.items)
                .any(|i| i.name == "flour" && i.checked)
        );
        let flour = block(b.pantry(PantrySearch {
            text: Some("flour".into()),
            ..PantrySearch::default()
        }))
        .unwrap();
        assert!((flour[0].qty.unwrap() - 1.4).abs() < 1e-9, "{flour:?}");

        // Moved to next week and unplanned.
        let next = block(b.move_meal(
            MealRef {
                date: "2026-11-17".into(),
                meal: "dinner".into(),
                recipe: None,
            },
            MoveTo {
                date: Some("2026-11-24".into()),
                meal: Some("lunch".into()),
            },
        ))
        .unwrap();
        assert_eq!(next.week, "2026-W48");
        assert_eq!(next.days[1].meals[0].meal, "lunch");
        assert!(
            block(b.week(Range::default())).unwrap().days[1]
                .meals
                .is_empty()
        );
        let w = block(b.unplan(MealRef {
            date: "2026-11-24".into(),
            meal: "lunch".into(),
            recipe: Some("Bread".into()),
        }))
        .unwrap();
        assert!(w.days.iter().all(|d| d.meals.is_empty()));
    }

    #[test]
    fn substitutions_and_can_cook() {
        let (b, _) = kitchen();
        block(b.stock(Stock {
            name: "Flour".into(),
            qty: Some(1.0),
            unit: Some("kg".into()),
            ..Stock::default()
        }))
        .unwrap();
        block(b.stock(Stock {
            name: "Instant yeast".into(),
            qty: Some(50.0),
            unit: Some("g".into()),
            ..Stock::default()
        }))
        .unwrap();
        let f = block(b.can_cook("Bread".into(), None)).unwrap();
        assert!(!f.can_cook);
        assert_eq!(f.missing[0].text, "7 g yeast");
        let subs = block(b.substitute(Substitution {
            from: "yeast".into(),
            to: "instant yeast".into(),
            ratio: 0.5,
            ..Substitution::default()
        }))
        .unwrap();
        assert_eq!(subs.len(), 1);
        let f = block(b.can_cook("Bread".into(), Some(2.0))).unwrap();
        assert!(f.can_cook, "{f:?}");
        assert_eq!(f.substituted, ["yeast → instant yeast"]);
        assert!(
            block(b.unsubstitute("yeast".into(), None))
                .unwrap()
                .is_empty()
        );
        block(b.remove_item("instant yeast".into())).unwrap();
        assert_eq!(block(b.pantry(PantrySearch::default())).unwrap().len(), 1);
    }
}
