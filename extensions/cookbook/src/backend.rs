//! [`Backend`], the one implementation of [`Recipes`]: Cooklang files
//! in the vault, reached through [`Files`].

use std::collections::BTreeSet;

use serde_json::Value;

use crate::files::{Files, Listed, is_recipe_path, text_of};
use crate::model::Recipe;
use crate::parse::parse_cook;
use crate::scale::scaled;
use crate::service::{
    CookbookError, Import, Imported, Needed, Portion, RecipeSummary, Recipes, Save, Search,
};

/// Where new recipes go unless `cookbook.folder` says otherwise.
pub const FOLDER: &str = "Cookbook";

/// How deep recipes may call for recipes (`[[Hot Honey]]{6}`) before a
/// loop is assumed.
pub const NESTING: usize = 8;

/// The cookbook over the vault's files. Cheap to clone.
#[derive(Clone, architect::HasDispatcher)]
pub struct Backend {
    files: Files,
}

impl architect::Services for Backend {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![crate::service::RecipesService]
    }
}

impl Backend {
    #[must_use]
    pub const fn new(files: Files) -> Self {
        Self { files }
    }

    /// The folder new recipes are written to.
    pub async fn folder(&self) -> String {
        self.files
            .setting("cookbook.folder")
            .await
            .as_str()
            .map(|f| f.trim().trim_matches('/').to_owned())
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| FOLDER.to_owned())
    }

    async fn parsed(&self, path: &str) -> Result<Recipe, CookbookError> {
        if !is_recipe_path(path) {
            return Err(CookbookError::Invalid(format!(
                "{path} is not a .cook file"
            )));
        }
        let text = self.files.read(path).await?;
        parse_cook(path, &text).map_err(|e| CookbookError::Invalid(format!("{path}: {e}")))
    }

    /// Every recipe's summary, by name: from the index when there is
    /// one, else from the files.
    async fn summaries(&self) -> Result<Vec<RecipeSummary>, CookbookError> {
        let mut out: Vec<RecipeSummary> = if let Some(rows) = self.files.indexed().await? {
            rows.iter().map(summary_of_listed).collect()
        } else {
            {
                let mut v = Vec::new();
                for p in self.files.paths().await? {
                    // A file that does not parse is still listed, by name.
                    match self.parsed(&p).await {
                        Ok(r) => v.push(summary_of(&r)),
                        Err(_) => v.push(RecipeSummary {
                            name: stem(&p),
                            path: p,
                            ..RecipeSummary::default()
                        }),
                    }
                }
                v
            }
        };
        out.sort_by_key(|s| (fold(&s.name), s.path.clone()));
        Ok(out)
    }

    /// The path a recipe called `target` from `from` names: the same
    /// folder's `<target>.cook`, else any recipe of that name.
    fn resolve(target: &str, from: &str, all: &[String]) -> Option<String> {
        let t = target.trim().trim_start_matches("./");
        let t = t.strip_suffix(".cook").unwrap_or(t);
        let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        let near = if dir.is_empty() {
            format!("{t}.cook")
        } else {
            format!("{dir}/{t}.cook")
        };
        if let Some(p) = all.iter().find(|p| p.eq_ignore_ascii_case(&near)) {
            return Some(p.clone());
        }
        let name = t.rsplit('/').next().unwrap_or(t);
        all.iter()
            .find(|p| stem(p).eq_ignore_ascii_case(name))
            .cloned()
    }
}

impl Recipes for Backend {
    async fn list(&self, search: Search) -> Result<Vec<RecipeSummary>, CookbookError> {
        let mut found: Vec<RecipeSummary> = self
            .summaries()
            .await?
            .into_iter()
            .filter(|s| matches(s, &search))
            .collect();
        if let Some(ing) = search
            .ingredient
            .as_deref()
            .map(fold)
            .filter(|i| !i.is_empty())
        {
            let mut kept = Vec::new();
            for s in found {
                if let Ok(r) = self.parsed(&s.path).await
                    && r.ingredients.iter().any(|i| fold(&i.name).contains(&ing))
                {
                    kept.push(s);
                }
            }
            found = kept;
        }
        if let Some(n) = search.limit {
            found.truncate(usize::try_from(n).unwrap_or(usize::MAX));
        }
        Ok(found)
    }

    async fn get(&self, path: String, servings: Option<f64>) -> Result<Recipe, CookbookError> {
        Ok(scaled(self.parsed(&path).await?, servings))
    }

    async fn save(&self, recipe: Save) -> Result<Recipe, CookbookError> {
        let given = recipe
            .path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        let path = if let Some(p) = given {
            p
        } else {
            let probe = parse_cook("Recipe.cook", &recipe.source)
                .map_err(|e| CookbookError::Invalid(e.to_string()))?;
            let name = recipe
                .name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or(probe.name);
            format!("{}/{}.cook", self.folder().await, file_name(&name))
        };
        if !is_recipe_path(&path) {
            return Err(CookbookError::Invalid(format!(
                "{path} is not a .cook file"
            )));
        }
        // Nothing that does not parse is written.
        let parsed = crate::parse::check(&path, &recipe.source)
            .map_err(|e| CookbookError::Invalid(format!("{path}: {e}")))?;
        let step = format!("Cookbook: save {}", parsed.name);
        if recipe.create {
            self.files.create(&path, &recipe.source).await?;
        } else {
            match self.files.replace(&path, &recipe.source, &step).await {
                Err(CookbookError::NotFound(_)) => self.files.create(&path, &recipe.source).await?,
                other => other?,
            }
        }
        Ok(parsed)
    }

    async fn rename(&self, path: String, to: String) -> Result<Recipe, CookbookError> {
        let to = if is_recipe_path(&to) {
            to
        } else {
            format!("{}.cook", to.trim_end_matches('/'))
        };
        self.files.rename(&path, &to).await?;
        self.parsed(&to).await
    }

    async fn delete(&self, path: String) -> Result<(), CookbookError> {
        if !is_recipe_path(&path) {
            return Err(CookbookError::Invalid(format!(
                "{path} is not a .cook file"
            )));
        }
        self.files.delete(&path).await
    }

    async fn ingredients(&self, portions: Vec<Portion>) -> Result<Vec<Needed>, CookbookError> {
        let all = self.files.paths().await?;
        let mut out = Vec::new();
        // (path, servings, depth, the chain that led there)
        let mut todo: Vec<(String, Option<f64>, usize, BTreeSet<String>)> = portions
            .into_iter()
            .rev()
            .map(|p| (p.path, p.servings, 0, BTreeSet::new()))
            .collect();
        while let Some((path, servings, depth, mut chain)) = todo.pop() {
            if depth > NESTING || !chain.insert(path.clone()) {
                return Err(CookbookError::Invalid(format!(
                    "{path} calls for itself through {} recipes",
                    chain.len()
                )));
            }
            let r = scaled(self.parsed(&path).await?, servings);
            let mut nested = Vec::new();
            for i in r.ingredients.iter() {
                if i.is_recipe_ref {
                    match Self::resolve(&i.name, &path, &all) {
                        Some(p) => nested.push((p, i.qty, depth.saturating_add(1), chain.clone())),
                        // A recipe it cannot find is bought as itself.
                        None => out.push(needed(i, &path)),
                    }
                } else {
                    out.push(needed(i, &path));
                }
            }
            todo.extend(nested.into_iter().rev());
        }
        Ok(out)
    }

    async fn import(&self, import: Import) -> Result<Imported, CookbookError> {
        use crate::import::{coverage::coverage, extract::extract, formats, synthesize_heuristic};
        let (source, review, unplaced) = if let Some(url) = import
            .url
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
        {
            let html = self.files.fetch(url).await?;
            let found = extract(&html, url).map_err(CookbookError::Invalid)?;
            let source = synthesize_heuristic(&found);
            let unplaced = coverage(&found, &source).unmatched;
            (source, true, unplaced)
        } else if let Some(text) = import.text.as_deref().filter(|t| !t.trim().is_empty()) {
            let lower = text.to_ascii_lowercase();
            if lower.contains("application/ld+json") {
                let found = extract(text, "").map_err(CookbookError::Invalid)?;
                let source = synthesize_heuristic(&found);
                let unplaced = coverage(&found, &source).unmatched;
                (source, true, unplaced)
            } else {
                (formats::arrows_to_frontmatter(text), false, Vec::new())
            }
        } else {
            return Err(CookbookError::Invalid(
                "give a page's `url` or a recipe's `text`".into(),
            ));
        };
        let probe = crate::parse::check("Recipe.cook", &source).map_err(CookbookError::Invalid)?;
        if probe.ingredients.is_empty() && probe.cook_steps.is_empty() {
            return Err(CookbookError::Invalid(
                "no ingredients or steps Cooklang reads".into(),
            ));
        }
        let path = match import
            .path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            Some(p) => p.to_owned(),
            None => format!("{}/{}.cook", self.folder().await, file_name(&probe.name)),
        };
        let recipe = crate::parse::check(&path, &source).map_err(CookbookError::Invalid)?;
        if import.save {
            self.files.create(&path, &source).await?;
        }
        Ok(Imported {
            path,
            source,
            recipe,
            saved: import.save,
            review,
            unplaced,
        })
    }

    async fn tidy(&self, folder: Option<String>) -> Result<Vec<String>, CookbookError> {
        let under = folder
            .map(|f| f.trim().trim_matches('/').to_owned())
            .filter(|f| !f.is_empty());
        let mut changed = Vec::new();
        for p in self.files.paths().await? {
            if under
                .as_deref()
                .is_some_and(|u| !p.starts_with(&format!("{u}/")))
            {
                continue;
            }
            let text = self.files.read(&p).await?;
            let tidy = crate::import::formats::arrows_to_frontmatter(&text);
            if tidy != text && crate::parse::check(&p, &tidy).is_ok() {
                self.files
                    .replace(&p, &tidy, "Cookbook: metadata as frontmatter")
                    .await?;
                changed.push(p);
            }
        }
        Ok(changed)
    }
}

fn needed(i: &crate::model::Ingredient, recipe: &str) -> Needed {
    Needed {
        name: i.name.clone(),
        qty: i.qty,
        unit: i.unit.clone(),
        display: if i.qty.is_none() {
            i.qty_display.clone()
        } else {
            None
        },
        recipe: recipe.to_owned(),
        optional: i.optional,
    }
}

/// A recipe's summary from its parse.
#[must_use]
pub fn summary_of(r: &Recipe) -> RecipeSummary {
    RecipeSummary {
        path: r.path.clone(),
        name: r.name.clone(),
        description: r.description.clone(),
        course: r.course.clone(),
        cuisine: r.cuisine.clone(),
        servings: r.servings.map(|s| s.to_string()),
        prep_minutes: r.prep_minutes,
        cook_minutes: r.cook_minutes,
        tags: r.tags.0.clone(),
        source: r.source_url.clone(),
    }
}

/// A recipe's summary from its index entry.
#[must_use]
pub fn summary_of_listed(l: &Listed) -> RecipeSummary {
    let v = |k: &str| l.values.get(k).and_then(text_of);
    RecipeSummary {
        path: l.path.clone(),
        name: v("title").unwrap_or_else(|| {
            if l.name.is_empty() {
                stem(&l.path)
            } else {
                l.name.clone()
            }
        }),
        description: v("description"),
        course: v("course"),
        cuisine: v("cuisine"),
        servings: v("servings"),
        prep_minutes: v("prep time")
            .or_else(|| v("prep_time"))
            .as_deref()
            .and_then(minutes),
        cook_minutes: v("cook time")
            .or_else(|| v("cook_time"))
            .or_else(|| v("time"))
            .as_deref()
            .and_then(minutes),
        tags: l.values.get("tags").map(tags_of).unwrap_or_default(),
        source: v("source"),
    }
}

fn tags_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(text_of).collect(),
        Value::String(s) => s
            .split(',')
            .map(|t| t.trim().trim_start_matches('#').to_owned())
            .filter(|t| !t.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Minutes from `15`, `15 min`, `1 hour`, `1h 30min`.
#[must_use]
pub fn minutes(text: &str) -> Option<u32> {
    let mut total: u32 = 0;
    let mut any = false;
    let mut num = String::new();
    let lower = text.to_lowercase();
    let mut chars = lower.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        if num.is_empty() || c.is_whitespace() {
            continue;
        }
        let n: u32 = num.parse().ok()?;
        num.clear();
        any = true;
        total = total.saturating_add(if c == 'h' { n.saturating_mul(60) } else { n });
        // Skip the rest of the unit's word.
        while chars.peek().is_some_and(|c| c.is_alphabetic()) {
            chars.next();
        }
    }
    if !num.is_empty() {
        total = total.saturating_add(num.parse().ok()?);
        any = true;
    }
    any.then_some(total)
}

fn matches(s: &RecipeSummary, q: &Search) -> bool {
    let words = q.text.as_deref().map(fold).unwrap_or_default();
    let hay = fold(&format!(
        "{} {} {} {} {} {}",
        s.name,
        s.description.as_deref().unwrap_or_default(),
        s.tags.join(" "),
        s.course.as_deref().unwrap_or_default(),
        s.cuisine.as_deref().unwrap_or_default(),
        s.path,
    ));
    words.split_whitespace().all(|w| hay.contains(w))
        && q.tag.as_deref().is_none_or(|t| {
            let t = fold(t.trim_start_matches('#'));
            s.tags.iter().any(|x| fold(x) == t)
        })
        && q.course
            .as_deref()
            .is_none_or(|c| s.course.as_deref().is_some_and(|x| fold(x) == fold(c)))
}

/// Text folded for matching: lower case, accents off the common letters.
#[must_use]
pub fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// A path's file name without its extension.
#[must_use]
pub fn stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_owned()
}

/// A recipe's name as a file name: no path separators or characters a
/// file system refuses.
#[must_use]
pub fn file_name(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let s = s.trim().trim_start_matches('.').to_owned();
    if s.is_empty() { "Recipe".to_owned() } else { s }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::float_cmp)]

    use super::*;

    fn block<F: std::future::Future>(f: F) -> F::Output {
        futures::executor::block_on(f)
    }

    fn book() -> Backend {
        Backend::new(Files::memory([
            (
                "Cookbook/Pancakes.cook",
                "---\ntitle: Pancakes\nservings: 2\ntags: [breakfast, quick]\ncourse: breakfast\n---\nWhisk @flour{200%g}, @egg{2} and @milk{300%ml}. Fry for ~{3%minutes}.\n",
            ),
            (
                "Cookbook/Hot Honey.cook",
                "---\nservings: 6\n---\nWarm @honey{120%g} with @chilli flakes{1%tsp}.\n",
            ),
            (
                "Cookbook/Chicken and Waffles.cook",
                "---\nservings: 4\ncourse: dinner\n---\nFry @chicken thighs{8}. Drizzle [[Hot Honey]]{3} over.\n",
            ),
            ("Notes/Shopping.md", "# not a recipe"),
        ]))
    }

    #[test]
    fn lists_and_searches_by_name_tag_course_and_ingredient() {
        let b = book();
        let all = block(b.list(Search::default())).unwrap();
        let names: Vec<_> = all.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Chicken and Waffles", "Hot Honey", "Pancakes"]);
        let s = |q: Search| -> Vec<String> {
            block(b.list(q))
                .unwrap()
                .into_iter()
                .map(|s| s.name)
                .collect()
        };
        assert_eq!(
            s(Search {
                text: Some("pan".into()),
                ..Search::default()
            }),
            ["Pancakes"]
        );
        assert_eq!(
            s(Search {
                tag: Some("#Quick".into()),
                ..Search::default()
            }),
            ["Pancakes"]
        );
        assert_eq!(
            s(Search {
                course: Some("Dinner".into()),
                ..Search::default()
            }),
            ["Chicken and Waffles"]
        );
        // A dish calling for Hot Honey uses honey too.
        assert_eq!(
            s(Search {
                ingredient: Some("honey".into()),
                ..Search::default()
            }),
            ["Chicken and Waffles", "Hot Honey"]
        );
        assert_eq!(
            s(Search {
                ingredient: Some("egg".into()),
                ..Search::default()
            }),
            ["Pancakes"]
        );
    }

    #[test]
    fn nested_recipes_are_cooked_out_and_scaled() {
        let b = book();
        let need = block(b.ingredients(vec![Portion {
            path: "Cookbook/Chicken and Waffles.cook".into(),
            servings: Some(8.0),
        }]))
        .unwrap();
        let by = |n: &str| need.iter().find(|x| x.name == n).unwrap().clone();
        assert_eq!(by("chicken thighs").qty, Some(16.0));
        // 3 servings of Hot Honey, doubled: 6 servings, its whole batch.
        assert_eq!(by("honey").qty, Some(120.0));
        assert_eq!(by("honey").recipe, "Cookbook/Hot Honey.cook");
        assert!(need.iter().all(|x| x.name != "Hot Honey"));
    }

    #[test]
    fn saves_only_cooklang_that_parses_and_names_new_files() {
        let b = book();
        let r = block(b.save(Save {
            source: "---\ntitle: Toast\n---\nToast @bread{2%slices}.\n".into(),
            create: true,
            ..Save::default()
        }))
        .unwrap();
        assert_eq!(r.path, "Cookbook/Toast.cook");
        assert!(matches!(
            block(b.save(Save {
                source: "x".into(),
                path: Some("Cookbook/Toast.cook".into()),
                create: true,
                ..Save::default()
            })),
            Err(CookbookError::Exists(_))
        ));
        let moved = block(b.rename(
            "Cookbook/Toast.cook".into(),
            "Cookbook/Breakfast/Toast".into(),
        ))
        .unwrap();
        assert_eq!(moved.path, "Cookbook/Breakfast/Toast.cook");
        block(b.delete(moved.path)).unwrap();
        assert!(matches!(
            block(b.get("Cookbook/Breakfast/Toast.cook".into(), None)),
            Err(CookbookError::NotFound(_))
        ));
    }

    #[test]
    fn a_loop_of_recipes_is_refused() {
        let b = Backend::new(Files::memory([
            ("A.cook", "Use [[B]]{1}.\n"),
            ("B.cook", "Use [[A]]{1}.\n"),
        ]));
        assert!(matches!(
            block(b.ingredients(vec![Portion {
                path: "A.cook".into(),
                servings: None
            }])),
            Err(CookbookError::Invalid(_))
        ));
    }

    #[test]
    fn imports_tasks_cook_files_and_pages_and_tidies_metadata() {
        let b = book();
        let got = block(b.import(Import {
            text: Some(">> title: Sourdough Loaf\n>> servings: 1\n\nMix @flour{450%g} with @water{330%g}.\n".into()),
            save: true,
            ..Import::default()
        }))
        .unwrap();
        assert_eq!(got.path, "Cookbook/Sourdough Loaf.cook");
        assert!(got.source.starts_with("---\n"));
        assert!(!got.review);
        assert_eq!(block(b.get(got.path, None)).unwrap().ingredients.len(), 2);

        let page = r#"<script type="application/ld+json">{"@type":"Recipe","name":"Toast","recipeYield":"2","recipeIngredient":["2 slices bread","1 tbsp butter"],"recipeInstructions":"Toast the bread.\nButter it."}</script>"#;
        let draft = block(b.import(Import {
            text: Some(page.into()),
            ..Import::default()
        }))
        .unwrap();
        assert!(draft.review && !draft.saved);
        assert_eq!(draft.recipe.name, "Toast");
        assert_eq!(draft.recipe.ingredients.len(), 2, "{}", draft.source);
        assert!(
            block(b.get("Cookbook/Toast.cook".into(), None)).is_err(),
            "a draft is not written"
        );

        let old = Backend::new(Files::memory([(
            "Cookbook/Old.cook",
            ">> servings: 2\n\nBoil @egg{2}.\n",
        )]));
        assert_eq!(block(old.tidy(None)).unwrap(), ["Cookbook/Old.cook"]);
        assert!(block(old.tidy(None)).unwrap().is_empty(), "twice is once");
    }

    #[test]
    fn index_rows_read_as_summaries() {
        let l = Listed {
            path: "Cookbook/Stew.cook".into(),
            name: "Stew".into(),
            values: serde_json::json!({ "servings": 4, "prep time": "20 min", "cook_time": "1h 30min", "tags": "winter, #slow" })
                .as_object()
                .unwrap()
                .clone(),
        };
        let s = summary_of_listed(&l);
        assert_eq!(s.name, "Stew");
        assert_eq!(s.servings.as_deref(), Some("4"));
        assert_eq!(s.prep_minutes, Some(20));
        assert_eq!(s.cook_minutes, Some(90));
        assert_eq!(s.tags, ["winter", "slow"]);
    }
}
