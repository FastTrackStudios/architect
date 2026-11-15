//! A page's schema.org Recipe, read from its JSON-LD.
//!
//! Most recipe sites publish one (`<script type="application/ld+json">`,
//! sometimes inside an `@graph`, sometimes as a list). This reads it with
//! no HTML parser, so it runs in a browser too: the scripts are found by
//! their tag, their JSON parsed, the first `Recipe` taken. A page with
//! none is refused with what to do instead (paste the recipe's text).

use std::collections::BTreeMap;

use serde_json::Value;

use super::normalized::NormalizedRecipe;

/// The page's Recipe, or why there is none.
///
/// # Errors
///
/// No JSON-LD Recipe on the page, or one with no ingredients or steps.
pub fn extract(html: &str, url: &str) -> Result<NormalizedRecipe, String> {
    let recipe = ld_json_blocks(html)
        .iter()
        .filter_map(|b| serde_json::from_str::<Value>(b).ok())
        .find_map(|v| find_recipe(&v).cloned())
        .ok_or_else(|| {
            format!("{url} has no schema.org Recipe (JSON-LD); paste the recipe's text instead")
        })?;
    let r = normalize(&recipe, url);
    if r.is_usable() {
        Ok(r)
    } else {
        Err(format!("{url}: its Recipe has no ingredients or no steps"))
    }
}

/// The text of every `<script type="application/ld+json">` on the page.
#[must_use]
pub fn ld_json_blocks(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = lower.get(from..).and_then(|r| r.find("<script")) {
        let open = from.saturating_add(at);
        let Some(close_tag) = lower.get(open..).and_then(|r| r.find('>')) else {
            break;
        };
        let body_from = open.saturating_add(close_tag).saturating_add(1);
        let tag = lower.get(open..body_from).unwrap_or_default();
        let Some(end) = lower.get(body_from..).and_then(|r| r.find("</script")) else {
            break;
        };
        let body_to = body_from.saturating_add(end);
        if tag.contains("application/ld+json")
            && let Some(body) = html.get(body_from..body_to)
        {
            out.push(body.trim().to_owned());
        }
        from = body_to;
    }
    out
}

fn is_recipe(v: &Value) -> bool {
    match v.get("@type") {
        Some(Value::String(t)) => t.eq_ignore_ascii_case("recipe"),
        Some(Value::Array(a)) => a
            .iter()
            .any(|t| t.as_str().is_some_and(|t| t.eq_ignore_ascii_case("recipe"))),
        _ => false,
    }
}

fn find_recipe(v: &Value) -> Option<&Value> {
    match v {
        Value::Array(a) => a.iter().find_map(find_recipe),
        Value::Object(m) => {
            if is_recipe(v) {
                return Some(v);
            }
            m.get("@graph")
                .and_then(find_recipe)
                .or_else(|| m.get("mainEntity").and_then(find_recipe))
        }
        _ => None,
    }
}

fn text(v: &Value) -> Option<String> {
    let t = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => return a.iter().find_map(text),
        Value::Object(m) => return m.get("name").or_else(|| m.get("text")).and_then(text),
        _ => return None,
    };
    let t = unescape(&t)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!t.is_empty()).then_some(t)
}

/// Every text of a value that may be one string, a list, or objects.
fn texts(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().flat_map(texts).collect(),
        Value::String(s) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(unescape)
            .collect(),
        other => text(other).into_iter().collect(),
    }
}

/// `recipeInstructions`: text, `HowToStep`s, `HowToSection`s, nested.
fn steps(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.extend(
            unescape(s)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned),
        ),
        Value::Array(a) => {
            for x in a {
                steps(x, out);
            }
        }
        Value::Object(m) => {
            if let Some(items) = m.get("itemListElement") {
                steps(items, out);
            } else if let Some(t) = m.get("text").or_else(|| m.get("name")).and_then(text) {
                out.push(t);
            }
        }
        _ => {}
    }
}

/// `PT1H30M` as `1 hour 30 minutes` (Cooklang reads either).
#[must_use]
pub fn duration(iso: &str) -> Option<String> {
    let t = iso
        .trim()
        .strip_prefix("PT")
        .or_else(|| iso.trim().strip_prefix("P0DT"))?;
    let mut num = String::new();
    let (mut h, mut m) = (0_u32, 0_u32);
    for c in t.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: u32 = num.parse().unwrap_or(0);
        num.clear();
        match c {
            'H' => h = h.saturating_add(n),
            'M' => m = m.saturating_add(n),
            _ => {}
        }
    }
    let total = h.saturating_mul(60).saturating_add(m);
    (total > 0).then(|| format!("{total} minutes"))
}

fn normalize(r: &Value, url: &str) -> NormalizedRecipe {
    let mut metadata = BTreeMap::new();
    let put = |metadata: &mut BTreeMap<String, String>, key: &str, v: Option<String>| {
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            metadata.insert(key.to_owned(), v);
        }
    };
    put(
        &mut metadata,
        "servings",
        r.get("recipeYield").and_then(text).map(|y| {
            // "4 servings", "Serves 4", 4: the number when there is one.
            y.split_whitespace()
                .find(|w| w.chars().all(|c| c.is_ascii_digit()))
                .map_or_else(|| y.clone(), str::to_owned)
        }),
    );
    let time = |k: &str| r.get(k).and_then(text).and_then(|t| duration(&t));
    put(&mut metadata, "prep time", time("prepTime"));
    put(&mut metadata, "cook time", time("cookTime"));
    if !metadata.contains_key("prep time") && !metadata.contains_key("cook time") {
        put(&mut metadata, "time", time("totalTime"));
    }
    put(
        &mut metadata,
        "course",
        r.get("recipeCategory").and_then(text),
    );
    put(
        &mut metadata,
        "cuisine",
        r.get("recipeCuisine").and_then(text),
    );
    put(&mut metadata, "author", r.get("author").and_then(text));
    let tags = r.get("keywords").map(texts).unwrap_or_default();
    put(
        &mut metadata,
        "tags",
        (!tags.is_empty()).then(|| tags.join(", ")),
    );
    let mut instructions = Vec::new();
    if let Some(i) = r.get("recipeInstructions") {
        steps(i, &mut instructions);
    }
    NormalizedRecipe {
        name: r
            .get("name")
            .and_then(text)
            .unwrap_or_else(|| "Imported recipe".to_owned()),
        description: r.get("description").and_then(text),
        image: r.get("image").and_then(|i| match i {
            Value::Object(m) => m.get("url").and_then(text),
            other => text(other),
        }),
        ingredients: r
            .get("recipeIngredient")
            .or_else(|| r.get("ingredients"))
            .map(texts_whole)
            .unwrap_or_default(),
        steps: instructions,
        metadata,
        source_url: Some(url.to_owned()).filter(|u| !u.is_empty()),
    }
}

/// Ingredient lines are whole lines: never split on their commas.
fn texts_whole(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(text).collect(),
        other => text(other).into_iter().collect(),
    }
}

/// The HTML entities recipe sites leave in their JSON.
#[must_use]
pub fn unescape(s: &str) -> String {
    let mut out = s
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    // Tags some sites leave in step text.
    while let (Some(a), Some(b)) = (out.find('<'), out.find('>')) {
        if b <= a {
            break;
        }
        out.replace_range(a..=b, "");
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    const PAGE: &str = r#"<html><head>
<script type="application/ld+json">{"@context":"https://schema.org","@graph":[
 {"@type":"WebPage","name":"x"},
 {"@type":["Recipe"],"name":"Banana Bread","description":"Moist &amp; easy.",
  "recipeYield":["8","8 slices"],"prepTime":"PT15M","cookTime":"PT1H",
  "recipeCategory":"Dessert","keywords":"bread, baking",
  "recipeIngredient":["1 1/2 cups flour, sifted","3 ripe bananas"],
  "recipeInstructions":[{"@type":"HowToSection","name":"Mix","itemListElement":[
     {"@type":"HowToStep","text":"Mash the bananas."},{"@type":"HowToStep","text":"Stir in the flour."}]},
   {"@type":"HowToStep","text":"Bake for 1 hour."}]}
]}</script></head><body></body></html>"#;

    #[test]
    fn reads_a_recipe_from_a_graph() {
        let r = extract(PAGE, "https://example.test/banana").unwrap();
        assert_eq!(r.name, "Banana Bread");
        assert_eq!(r.description.as_deref(), Some("Moist & easy."));
        assert_eq!(
            r.ingredients,
            ["1 1/2 cups flour, sifted", "3 ripe bananas"]
        );
        assert_eq!(r.steps.len(), 3);
        assert_eq!(r.metadata["servings"], "8");
        assert_eq!(r.metadata["prep time"], "15 minutes");
        assert_eq!(r.metadata["cook time"], "60 minutes");
        assert_eq!(r.metadata["tags"], "bread, baking");
        assert_eq!(r.source_url.as_deref(), Some("https://example.test/banana"));
    }

    #[test]
    fn a_page_without_one_says_what_to_do() {
        let e = extract("<html><body>hi</body></html>", "https://x.test").unwrap_err();
        assert!(e.contains("paste"), "{e}");
    }

    #[test]
    fn durations_read_as_minutes() {
        assert_eq!(duration("PT1H30M").as_deref(), Some("90 minutes"));
        assert_eq!(duration("PT0M"), None);
    }
}
