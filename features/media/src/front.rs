//! A note's frontmatter as JSON: the form every reader here takes, and
//! the one a vault's index already keeps (`architect_sites::listing::Entry`'s
//! `properties`).

use serde_json::{Map, Value};

/// The frontmatter block of `text` (between a first line `---` and the
/// next `---`), and the body after it. No block: `None` and the whole
/// text.
#[must_use]
pub fn split(text: &str) -> (Option<&str>, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (None, text);
    };
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let head = rest.get(..at).unwrap_or_default();
            let body = rest
                .get(at.saturating_add(line.len())..)
                .unwrap_or_default();
            return (Some(head), body);
        }
        at = at.saturating_add(line.len());
    }
    (None, text)
}

/// The frontmatter of `text` as a JSON object; empty when it has none or
/// it does not read as a YAML mapping.
#[must_use]
pub fn fields(text: &str) -> Map<String, Value> {
    split(text).0.map(parse).unwrap_or_default()
}

/// A frontmatter block (YAML, without its `---` lines) as a JSON object;
/// empty when it is not a mapping.
#[must_use]
pub fn parse(block: &str) -> Map<String, Value> {
    match serde_yaml::from_str::<Value>(block) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_reads_as_json_nested_lists_too() {
        let text =
            "---\ntitle: Set\nsets:\n  - name: One\n    songs:\n    - \"[[a]]\"\n---\n# Body\n";
        let f = fields(text);
        assert_eq!(f["title"], "Set");
        assert_eq!(f["sets"][0]["name"], "One");
        assert_eq!(f["sets"][0]["songs"][0], "[[a]]");
        assert_eq!(split(text).1, "# Body\n");
    }

    #[test]
    fn no_frontmatter_or_a_broken_one_is_empty() {
        assert!(fields("# Just a note\n").is_empty());
        assert!(fields("---\n: : :\n---\n").is_empty());
        assert!(fields("---\n- a list\n---\n").is_empty());
        assert_eq!(split("---\ntitle: x\n---").0, Some("title: x\n"));
    }
}
