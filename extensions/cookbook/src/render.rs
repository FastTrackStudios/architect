//! A recipe's Cooklang as it reads in the editor (`[[renderers]]`,
//! `COOKBOOK_RENDER`): the source stays what is edited, and every line
//! but the one the caret is on reads as the recipe means it.
//!
//! - `@flour{250%g}(sifted)` reads as a chip "250 g flour (sifted)";
//! - `#large bowl{}` as a chip "large bowl";
//! - `~rest{10%minutes}` as a chip "rest 10 minutes";
//! - `= Batter` as a heading, its `=` hidden;
//! - `[[Hot Honey]]{3}` as "3 × Hot Honey";
//! - `-- a comment`, `[- a block -]`, `>> key: value` quieter, and a
//!   `> note` too, its `>` hidden.

use architect_extensions::render::{Look, RenderItem, Rendered};
use serde_json::Value;

/// `COOKBOOK_RENDER {text}`: how `text` reads ([`render`]).
///
/// # Errors
///
/// When it is given no `text`.
pub fn action(a: &Value) -> Result<Value, String> {
    let text = a
        .get("text")
        .and_then(Value::as_str)
        .ok_or("give `text`, the recipe's Cooklang")?;
    serde_json::to_value(render(text)).map_err(|e| e.to_string())
}

/// What `text`'s Cooklang reads as, byte range by byte range.
#[must_use]
pub fn render(text: &str) -> Rendered {
    let mut items = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        line_items(line, at, &mut items);
        at = at.saturating_add(line.len());
    }
    Rendered { items }
}

fn line_items(raw: &str, base: usize, out: &mut Vec<RenderItem>) {
    let line = raw.trim_end_matches(['\n', '\r']);
    let trimmed = line.trim_start();
    let indent = line.len().saturating_sub(trimmed.len());
    let start = base.saturating_add(indent);
    let end = base.saturating_add(line.len());
    if trimmed.is_empty() {
        return;
    }
    // `= Section` (or `== Section ==`): a heading, its marks hidden.
    if trimmed.starts_with('=') {
        let marks = trimmed
            .len()
            .saturating_sub(trimmed.trim_start_matches('=').len());
        let after = trimmed.get(marks..).unwrap_or("");
        let gap = after.len().saturating_sub(after.trim_start().len());
        let lead = marks.saturating_add(gap);
        let body = after.trim();
        let tail = body.len().saturating_sub(body.trim_end_matches('=').len());
        out.push(RenderItem::Line {
            at: start,
            look: Look::Heading,
        });
        out.push(RenderItem::Conceal {
            from: start,
            to: start.saturating_add(lead),
        });
        if tail > 0 {
            let body_end = start.saturating_add(lead).saturating_add(body.len());
            let kept = body.trim_end_matches('=').trim_end().len();
            out.push(RenderItem::Conceal {
                from: start.saturating_add(lead).saturating_add(kept),
                to: body_end,
            });
        }
        return;
    }
    // `>> key: value`: the recipe's metadata, quiet.
    if trimmed.starts_with(">>") {
        out.push(RenderItem::Line {
            at: start,
            look: Look::Muted,
        });
        return;
    }
    // `> a note`: quiet, its mark hidden.
    if let Some(rest) = trimmed.strip_prefix('>') {
        let gap = rest.len().saturating_sub(rest.trim_start().len());
        out.push(RenderItem::Line {
            at: start,
            look: Look::Muted,
        });
        out.push(RenderItem::Conceal {
            from: start,
            to: start.saturating_add(1).saturating_add(gap),
        });
        return;
    }
    spans(line, base, end, out);
}

/// The ingredients, cookware, timers, links and comments in a step.
fn spans(line: &str, base: usize, end: usize, out: &mut Vec<RenderItem>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = line.get(i..).unwrap_or("");
        let here = base.saturating_add(i);
        if rest.starts_with("--") {
            out.push(RenderItem::Mark {
                from: here,
                to: end,
                look: Look::Muted,
            });
            return;
        }
        if rest.starts_with("[-") {
            let close = rest.find("-]").map_or(rest.len(), |j| j.saturating_add(2));
            out.push(RenderItem::Mark {
                from: here,
                to: here.saturating_add(close),
                look: Look::Muted,
            });
            i = i.saturating_add(close);
            continue;
        }
        if rest.starts_with("[[") {
            if let Some(token) = link(rest) {
                out.push(RenderItem::Replace {
                    from: here,
                    to: here.saturating_add(token.len),
                    text: token.text,
                    look: Look::Accent,
                });
                i = i.saturating_add(token.len);
                continue;
            }
        }
        let sigil = bytes.get(i).copied();
        if matches!(sigil, Some(b'@' | b'#' | b'~'))
            && let Some(token) = component(rest)
        {
            let look = match sigil {
                Some(b'@') => Look::ChipAccent,
                _ => Look::Chip,
            };
            out.push(RenderItem::Replace {
                from: here,
                to: here.saturating_add(token.len),
                text: token.text,
                look,
            });
            i = i.saturating_add(token.len);
            continue;
        }
        i = i.saturating_add(rest.chars().next().map_or(1, char::len_utf8));
    }
}

/// A token read: how many bytes it takes and what it reads as.
struct Token {
    len: usize,
    text: String,
}

/// `@name{qty%unit}(note)`, `#name{}`, `~name{qty%unit}`, or a one-word
/// `@salt` / `#pan`; `None` when the sigil starts nothing (`a # b`).
fn component(s: &str) -> Option<Token> {
    let sigil = s.chars().next()?;
    let body = s.get(1..)?;
    // Cooklang's modifiers: `@?` optional, `@&` a reference, `@-` hidden.
    let modifiers = body
        .len()
        .saturating_sub(body.trim_start_matches(['?', '&', '-', '+']).len());
    let body = body.get(modifiers..)?;
    let (name, after_name, braces) = match body.find('{') {
        // A name with spaces needs its braces, up to the first `{` on the
        // same word run: `@baking powder{2%tsp}`.
        Some(open) if !body.get(..open)?.contains(['@', '#', '~', '.', ',', ';']) => {
            let close = body.get(open..)?.find('}')?.saturating_add(open);
            (
                body.get(..open)?,
                close.saturating_add(1),
                Some(body.get(open.saturating_add(1)..close)?),
            )
        }
        _ => {
            let word = body
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '\''))
                .unwrap_or(body.len());
            (body.get(..word)?, word, None)
        }
    };
    let name = name.trim();
    if name.is_empty() && sigil != '~' {
        return None;
    }
    if name.is_empty() && braces.is_none() {
        return None;
    }
    let mut len = 1_usize.saturating_add(modifiers).saturating_add(after_name);
    let amount = braces.map(amount).unwrap_or_default();
    // An ingredient's `(note)`, right after it.
    let mut note = None;
    if sigil == '@'
        && let Some(rest) = s.get(len..)
        && rest.starts_with('(')
        && let Some(close) = rest.find(')')
    {
        note = rest.get(1..close).map(str::trim).filter(|n| !n.is_empty());
        len = len.saturating_add(close).saturating_add(1);
    }
    let name = name.rsplit('/').next().unwrap_or(name);
    let mut text = match (sigil, amount.is_empty()) {
        ('@', false) => format!("{amount} {name}"),
        ('~', false) if name.is_empty() => amount,
        ('~', false) => format!("{name} {amount}"),
        _ => name.to_owned(),
    };
    if let Some(n) = note {
        text.push_str(" (");
        text.push_str(n);
        text.push(')');
    }
    Some(Token { len, text })
}

/// `250%g` as "250 g", `=1%pinch` as "1 pinch", `2` as "2"; nothing as
/// nothing.
fn amount(inside: &str) -> String {
    let inside = inside.trim().trim_start_matches('=').trim();
    match inside.split_once('%') {
        Some((q, u)) => format!("{} {}", q.trim(), u.trim()).trim().to_owned(),
        None => inside.to_owned(),
    }
}

/// `[[Hot Honey]]{3}` as "3 × Hot Honey", `[[Slaw|the slaw]]` as "the
/// slaw".
fn link(s: &str) -> Option<Token> {
    let inner_end = s.find("]]")?;
    let inner = s.get(2..inner_end)?;
    let shown = inner.rsplit_once('|').map_or(inner, |(_, a)| a).trim();
    let mut len = inner_end.saturating_add(2);
    let mut text = shown.to_owned();
    if let Some(rest) = s.get(len..)
        && rest.starts_with('{')
        && let Some(close) = rest.find('}')
    {
        let q = amount(rest.get(1..close).unwrap_or(""));
        if !q.is_empty() {
            text = format!("{q} × {shown}");
        }
        len = len.saturating_add(close).saturating_add(1);
    }
    Some(Token { len, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replaced(text: &str) -> Vec<(String, String)> {
        render(text)
            .items
            .into_iter()
            .filter_map(|i| match i {
                RenderItem::Replace {
                    from,
                    to,
                    text: shown,
                    ..
                } => Some((text.get(from..to).unwrap_or_default().to_owned(), shown)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn ingredients_cookware_and_timers_read_as_words() {
        let t = "Whisk @flour{250%g}, @baking powder{2%tsp} and @salt{=1%pinch} in a #large bowl{}.\nAdd @eggs{2} and @butter{40%g}(melted). Rest ~{10%minutes}, then ~bake{20%min}. Season with @pepper.";
        assert_eq!(
            replaced(t),
            [
                ("@flour{250%g}", "250 g flour"),
                ("@baking powder{2%tsp}", "2 tsp baking powder"),
                ("@salt{=1%pinch}", "1 pinch salt"),
                ("#large bowl{}", "large bowl"),
                ("@eggs{2}", "2 eggs"),
                ("@butter{40%g}(melted)", "40 g butter (melted)"),
                ("~{10%minutes}", "10 minutes"),
                ("~bake{20%min}", "bake 20 min"),
                ("@pepper", "pepper"),
            ]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
    }

    #[test]
    fn a_section_is_a_heading_and_a_comment_is_quiet() {
        let t = "= Batter\n\nMix. -- gently\n== Cook ==\n>> source: Gran\n> Lumps are fine.\n";
        let items = render(t).items;
        assert!(items.contains(&RenderItem::Line {
            at: 0,
            look: Look::Heading
        }));
        assert!(items.contains(&RenderItem::Conceal { from: 0, to: 2 }));
        let comment = t.find("--").unwrap();
        assert!(items.contains(&RenderItem::Mark {
            from: comment,
            to: comment + 9,
            look: Look::Muted
        }));
        let cook = t.find("== Cook").unwrap();
        assert!(items.contains(&RenderItem::Conceal {
            from: cook,
            to: cook + 3
        }));
        assert!(items.contains(&RenderItem::Conceal {
            from: cook + 7,
            to: cook + 10
        }));
        let note = t.find("> Lumps").unwrap();
        assert!(items.contains(&RenderItem::Conceal {
            from: note,
            to: note + 2
        }));
    }

    #[test]
    fn a_recipe_link_reads_with_its_amount() {
        assert_eq!(
            replaced("Drizzle [[Hot Honey]]{3%tbsp} over [[Slaw|the slaw]]."),
            [
                ("[[Hot Honey]]{3%tbsp}", "3 tbsp × Hot Honey"),
                ("[[Slaw|the slaw]]", "the slaw")
            ]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
    }

    #[test]
    fn a_lone_hash_or_at_is_left_alone() {
        assert!(replaced("Serves 4 # roughly, @ home").is_empty());
    }

    #[test]
    fn every_item_fits_the_text() {
        let t = "Fry @onion{1}(diced) in @olive oil{2%tbsp} — ~{5%min} — in a #pan.\n= Done ==\n";
        assert!(render(t).items.iter().all(|i| i.fits(t)));
    }
}
