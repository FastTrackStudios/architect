//! A note's ```` ```base ```` fence, drawn as static views (#346).
//!
//! The fence is a base, as Bases reads one (`features/bases`): its YAML,
//! or one line naming a `.base` file in the vault. Each of its views is
//! run over the vault's notes by the vault's own evaluator
//! (`vault::bases::view_table`, rows from
//! `vault::obsidian::base_query::vault_to_rows`) and written out: a table
//! as a table, cards as cards, a list as a list, and any other kind (a
//! board, a calendar, a type an extension adds) as a table, for now.
//! Each row's name links to its note's page.
//!
//! It is one more [`BlockRenderer`]. The site adds it after the caller's,
//! so an extension that claims `fence:base` itself draws it instead.

use std::collections::HashMap;
use std::fmt::Write as _;

use architect_extensions::contrib::Claim;
use serde_json::Value;
use vault::bases::{BaseRow, ParsedBase, TableRow, ViewKind, ViewTable, parse, view_table};
use vault::obsidian::Vault;
use vault::obsidian::base_query::vault_to_rows;

use crate::blocks::{Block, BlockRenderer, Drawn};
use crate::element_html::escape;

/// Draws ```` ```base ```` fences over one vault.
pub struct BaseBlocks {
    rows: Vec<BaseRow>,
    /// The vault's `.base` files, by vault path and by file name.
    files: HashMap<String, Result<ParsedBase, String>>,
    /// Each published note's URL and title, by vault path.
    pages: HashMap<String, (String, String)>,
}

impl std::fmt::Debug for BaseBlocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BaseBlocks")
            .field("rows", &self.rows.len())
            .field("files", &self.files.len())
            .finish_non_exhaustive()
    }
}

impl BaseBlocks {
    /// Over `vault`'s notes. A row whose vault path is in `pages` (its
    /// URL and title) shows that title, linked to that page.
    #[must_use]
    pub fn new(vault: &Vault, pages: HashMap<String, (String, String)>) -> Self {
        let mut files = HashMap::new();
        for base in &vault.bases {
            let parsed = base.parsed.clone();
            files.insert(base.rel_path.clone(), parsed.clone());
            files
                .entry(format!("{}.base", base.basename))
                .or_insert(parsed);
        }
        Self {
            rows: vault_to_rows(vault),
            files,
            pages,
        }
    }

    /// Only the rows of published notes: a scoped site's bases never
    /// list, count or group a note outside its scope.
    #[must_use]
    pub fn published_only(mut self) -> Self {
        self.rows.retain(|row| self.pages.contains_key(&row.path));
        self
    }

    /// The base a fence names: its YAML, or a `.base` file.
    fn base_of(&self, source: &str) -> Result<ParsedBase, String> {
        let body = source.trim();
        let one_file = !body.contains('\n')
            && std::path::Path::new(body)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("base"));
        if one_file {
            return self
                .files
                .get(body)
                .cloned()
                .unwrap_or_else(|| Err(format!("no base file `{body}` in the vault")));
        }
        parse(source).map_err(|e| e.to_string())
    }

    /// A row's name: its page's title, linked, when it has a page.
    fn name(&self, row: &TableRow) -> String {
        self.pages.get(&row.path).map_or_else(
            || escape(&row.name),
            |(url, title)| {
                format!(
                    "<a class=\"site-link\" href=\"{}\">{}</a>",
                    escape(url),
                    escape(title)
                )
            },
        )
    }

    fn draw_view(&self, table: &ViewTable, kind: &ViewKind) -> String {
        let mut out = format!(
            "<figure class=\"site-base\" data-view=\"{}\"><figcaption>{}</figcaption>",
            escape(kind.as_str()),
            escape(&table.name)
        );
        // The name column is drawn as the row's link; the rest as values.
        let columns: Vec<&String> = table
            .columns
            .iter()
            .filter(|c| c.as_str() != "file.name")
            .collect();
        let rows = table.groups.iter().map(|g| g.rows.len()).sum::<usize>();
        if rows == 0 {
            out.push_str("<p class=\"site-base-empty\">No notes match.</p></figure>");
            return out;
        }
        match kind {
            ViewKind::Cards | ViewKind::Gallery | ViewKind::List => {
                let list = if matches!(kind, ViewKind::List) {
                    "site-base-list"
                } else {
                    "site-base-cards"
                };
                for group in &table.groups {
                    if !group.label.is_empty() {
                        let _ = write!(out, "<h4>{}</h4>", escape(&group.label));
                    }
                    let _ = write!(out, "<ul class=\"{list}\">");
                    for row in &group.rows {
                        out.push_str("<li><span class=\"site-base-name\">");
                        out.push_str(&self.name(row));
                        out.push_str("</span>");
                        for column in &columns {
                            let value = text(row.values.get(*column));
                            if !value.is_empty() {
                                let _ = write!(
                                    out,
                                    "<span class=\"site-base-value\"><b>{}</b> {}</span>",
                                    escape(&label(column)),
                                    escape(&value)
                                );
                            }
                        }
                        out.push_str("</li>");
                    }
                    out.push_str("</ul>");
                }
            }
            _ => {
                out.push_str("<div class=\"site-table\"><table><thead><tr><th>Name</th>");
                for column in &columns {
                    let _ = write!(out, "<th>{}</th>", escape(&label(column)));
                }
                out.push_str("</tr></thead><tbody>");
                let width = columns.len().saturating_add(1);
                for group in &table.groups {
                    if !group.label.is_empty() {
                        let _ = write!(
                            out,
                            "<tr class=\"site-base-group\"><th colspan=\"{width}\">{}</th></tr>",
                            escape(&group.label)
                        );
                    }
                    for row in &group.rows {
                        out.push_str("<tr><td>");
                        out.push_str(&self.name(row));
                        out.push_str("</td>");
                        for column in &columns {
                            let _ =
                                write!(out, "<td>{}</td>", escape(&text(row.values.get(*column))));
                        }
                        out.push_str("</tr>");
                    }
                }
                out.push_str("</tbody></table></div>");
            }
        }
        out.push_str("</figure>");
        out
    }
}

impl BlockRenderer for BaseBlocks {
    fn name(&self) -> &'static str {
        "architect.bases"
    }

    fn claims(&self, claim: &Claim) -> bool {
        matches!(claim, Claim::Fence(lang) if lang.eq_ignore_ascii_case("base"))
    }

    fn render(&self, block: &Block<'_>) -> Result<Drawn, String> {
        let base = self.base_of(block.source)?;
        if base.views.is_empty() {
            return Err("the base has no views".to_owned());
        }
        let html = base
            .views
            .iter()
            .map(|view| self.draw_view(&view_table(&base, view, &self.rows), &view.kind))
            .collect::<String>();
        Ok(Drawn::Html(html))
    }
}

/// A column's name as a heading: `formula.net` is `net`, `file.mtime`
/// is `mtime`.
fn label(column: &str) -> String {
    column
        .strip_prefix("formula.")
        .or_else(|| column.strip_prefix("file."))
        .or_else(|| column.strip_prefix("note."))
        .unwrap_or(column)
        .to_owned()
}

/// A value as text: a link as its target's name, a list joined.
fn text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s
            .strip_prefix("[[")
            .and_then(|s| s.strip_suffix("]]"))
            .map_or_else(
                || s.clone(),
                |inner| inner.split('|').next_back().unwrap_or(inner).to_owned(),
            ),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| text(Some(v)))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_and_labels_read_as_text() {
        assert_eq!(text(Some(&Value::String("[[Song|Intro]]".into()))), "Intro");
        assert_eq!(text(Some(&serde_json::json!(["a", null, 3]))), "a, 3");
        assert_eq!(label("formula.net"), "net");
        assert_eq!(label("fee"), "fee");
    }
}
