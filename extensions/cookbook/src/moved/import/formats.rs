//! Recipes written elsewhere, as the cookbook keeps them.
//!
//! - A `.cook` file with Cooklang's `>> key: value` metadata (Task's
//!   cookbook): the metadata becomes YAML frontmatter, which the vault
//!   reads as the recipe's properties; the Cooklang is kept as it is.
//! - A `.cook` file with frontmatter already: kept as it is.

/// `source` with its `>> key: value` lines (wherever they are) moved
/// into YAML frontmatter at its head; unchanged when it has frontmatter
/// or no metadata lines.
#[must_use]
pub fn arrows_to_frontmatter(source: &str) -> String {
    if source.starts_with("---\n") || source.starts_with("---\r\n") {
        return source.to_owned();
    }
    let mut meta = serde_yaml::Mapping::new();
    let mut body = Vec::new();
    for line in source.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(">>")
            && let Some((k, v)) = rest.split_once(':')
            && !k.trim().is_empty()
        {
            meta.insert(
                serde_yaml::Value::from(k.trim()),
                serde_yaml::Value::from(v.trim()),
            );
            continue;
        }
        body.push(line);
    }
    if meta.is_empty() {
        return source.to_owned();
    }
    let yaml = serde_yaml::to_string(&meta).unwrap_or_default();
    let body = body.join("\n");
    format!("---\n{yaml}---\n{}\n", body.trim_matches('\n'))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn tasks_metadata_becomes_frontmatter_and_the_cooklang_stays() {
        let src = ">> title: Sourdough Loaf\n>> servings: 1 loaf\n>> prep time: 20 min\n\nMix @flour{450%g} with @water{330%g}.\n";
        let out = arrows_to_frontmatter(src);
        assert!(out.starts_with("---\n"), "{out}");
        assert!(out.contains("title: Sourdough Loaf"));
        assert!(out.contains("prep time: 20 min"));
        assert!(
            out.ends_with("Mix @flour{450%g} with @water{330%g}.\n"),
            "{out}"
        );
        let r = crate::parse::parse_cook("x.cook", &out).unwrap();
        assert_eq!(r.name, "Sourdough Loaf");
        assert_eq!(r.prep_minutes, Some(20));
        assert_eq!(arrows_to_frontmatter(&out), out);
    }
}
