//! How many of an import's ingredient lines its Cooklang annotates
//! (Task's `recipe-import` `synthesize::coverage`).

use super::normalized::NormalizedRecipe;

/// An import's ingredient lines against its `@` annotations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub total: usize,
    pub matched: usize,
    /// Lines with no annotation.
    pub unmatched: Vec<String>,
}

impl Coverage {
    #[must_use]
    pub fn complete(&self) -> bool {
        self.matched == self.total
    }
}

/// Compare the normalized ingredient lines against the `@` annotations
/// the parser actually sees in `source`.
#[must_use]
pub fn coverage(recipe: &NormalizedRecipe, source: &str) -> Coverage {
    let parser =
        cooklang::CooklangParser::new(cooklang::Extensions::all(), cooklang::Converter::bundled());
    let annotated: Vec<String> = parser
        .parse(source)
        .into_result()
        .map(|(parsed, _)| {
            parsed
                .ingredients
                .iter()
                .map(|i| i.name.to_lowercase())
                .collect()
        })
        .unwrap_or_default();

    let mut matched = 0usize;
    let mut unmatched = Vec::new();
    for line in &recipe.ingredients {
        let line_lc = line.to_lowercase();
        let hit = annotated.iter().any(|name| {
            name.len() >= 3 && (line_lc.contains(name.as_str()) || name.contains(line_lc.as_str()))
        });
        if hit {
            matched += 1;
        } else {
            unmatched.push(line.clone());
        }
    }
    Coverage {
        total: recipe.ingredients.len(),
        matched,
        unmatched,
    }
}
