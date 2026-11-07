//! A Vault note's Markdown, drawn (the lab has no editor yet): headings,
//! paragraphs and fenced blocks, with its frontmatter. A fenced block
//! whose language an extension claims, and a note whose `type:` one
//! claims, are drawn by that extension inline
//! (`architect_extensions::embeds::EmbedView`, the Board's ```` ```kf ````
//! chart and song card); the plain block when none does, or once it
//! unloads.
//!
//! The `FastTrackStudios` editor's fence plugins (`editor_state::plugin`)
//! render a fence to an HTML string, synchronously; an embed is a live
//! view with its own size, so it needs an editor-side hook the editor does
//! not have yet: a widget decoration that mounts a Dioxus element (or
//! names a node for a sub-document) at the fence, a height it is told
//! back for its layout, and the fence's text as it is edited. Until then
//! the lab draws its notes here.

use architect_extensions::contrib::Claim;
use architect_extensions::embeds::EmbedView;
use architect_shell::use_shell;
use dioxus::prelude::*;
use serde_json::{Map, Value, json};

/// One block of a note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(u8, String),
    Paragraph(String),
    /// A fenced block: its info string (```` ```kf chorus ```` → `kf
    /// chorus`) and its body.
    Fence {
        info: String,
        body: String,
    },
}

/// A note: its frontmatter (`key: value` lines between `---`), its body
/// (what follows it) and the body's blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Note {
    pub front: Vec<(String, String)>,
    pub body: String,
    pub blocks: Vec<Block>,
}

impl Note {
    /// A frontmatter value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.front
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The frontmatter as JSON, for an embed's `props`.
    #[must_use]
    pub fn front_json(&self) -> Value {
        let map: Map<String, Value> = self
            .front
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        Value::Object(map)
    }
}

/// Reads a note's Markdown.
#[must_use]
pub fn parse(text: &str) -> Note {
    let (front, body) = frontmatter(text);
    let mut blocks = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut fence: Option<(String, Vec<&str>)> = None;
    let flush = |para: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Paragraph(para.join(" ")));
            para.clear();
        }
    };
    for line in body.lines() {
        if let Some((info, lines)) = fence.as_mut() {
            if line.trim_start().starts_with("```") {
                blocks.push(Block::Fence {
                    info: std::mem::take(info),
                    body: lines.join("\n"),
                });
                fence = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        let trimmed = line.trim();
        if let Some(info) = trimmed.strip_prefix("```") {
            flush(&mut para, &mut blocks);
            fence = Some((info.trim().to_owned(), Vec::new()));
        } else if trimmed.is_empty() {
            flush(&mut para, &mut blocks);
        } else if let Some((level, title)) = heading(trimmed) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading(level, title.to_owned()));
        } else {
            para.push(trimmed);
        }
    }
    flush(&mut para, &mut blocks);
    // An unclosed fence runs to the end, as an editor shows it.
    if let Some((info, lines)) = fence {
        blocks.push(Block::Fence {
            info,
            body: lines.join("\n"),
        });
    }
    Note {
        front,
        body: body.to_owned(),
        blocks,
    }
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    let rest = line.get(hashes..)?;
    (1..=3)
        .contains(&hashes)
        .then(|| rest.strip_prefix(' '))
        .flatten()
        .and_then(|title| u8::try_from(hashes).ok().map(|l| (l, title.trim())))
}

pub fn frontmatter(text: &str) -> (Vec<(String, String)>, &str) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return (Vec::new(), text);
    };
    let Some((head, after)) = rest.split_once("\n---") else {
        return (Vec::new(), text);
    };
    let front = head
        .lines()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .filter(|(k, _)| !k.is_empty())
        .collect();
    (front, after.strip_prefix('\n').unwrap_or(after))
}

/// A note's Markdown, drawn: its type's embed at its head when an
/// extension claims it, then its blocks, each claimed fence an embed.
#[component]
pub fn NoteBody(text: String) -> Element {
    let t = use_shell().theme;
    let note = parse(&text);
    let head = note
        .get("type")
        .map(|ty| (ty.to_owned(), note.front_json()));
    rsx! {
        div { class: "note-body", style: "display: flex; flex-direction: column; gap: 12px; max-width: 640px;",
            if let Some((ty, props)) = head {
                EmbedView {
                    claim: Claim::NoteType(ty.clone()),
                    source: note.body.clone(),
                    props,
                    fallback: rsx! {
                        div { class: "note-type", style: "font-size: 12px; color: {t.ink_3};", "type: {ty}" }
                    },
                }
            }
            for (i, block) in note.blocks.into_iter().enumerate() {
                match block {
                    Block::Heading(level, title) => rsx! {
                        div { key: "{i}",
                            style: "font-size: {22 - 3 * i32::from(level)}px; font-weight: 700; color: {t.ink};",
                            "{title}"
                        }
                    },
                    Block::Paragraph(words) => rsx! {
                        div { key: "{i}", style: "font-size: 14px; line-height: 1.6; color: {t.ink_2};", "{words}" }
                    },
                    Block::Fence { info, body } => {
                        let plain = rsx! { PlainFence { info: info.clone(), body: body.clone() } };
                        match Claim::fence(&info) {
                            Some(claim) => rsx! {
                                EmbedView {
                                    key: "{i}",
                                    claim,
                                    source: body,
                                    props: json!({ "info": info }),
                                    fallback: plain,
                                }
                            },
                            None => rsx! { div { key: "{i}", {plain} } },
                        }
                    }
                }
            }
        }
    }
}

/// A fenced block as its source: what an unclaimed fence is.
#[component]
fn PlainFence(info: String, body: String) -> Element {
    let t = use_shell().theme;
    rsx! {
        div { class: "plain-fence",
            style: "display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; border-radius: 8px; \
                    border: 1px solid {t.rule}; background: {t.pill};",
            if !info.is_empty() {
                span { style: "font-size: 11px; color: {t.ink_3};", "{info}" }
            }
            pre { style: "margin: 0; font-family: ui-monospace, Menlo, monospace; font-size: 13px; \
                          line-height: 1.5; color: {t.ink}; white-space: pre-wrap;",
                "{body}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_reads_its_frontmatter_headings_paragraphs_and_fences() {
        let n = parse(
            "---\ntype: song\ntitle: Grace\n---\n# Grace\nline one\nline two\n\n```kf verse\n| C | G |\n```\n\n```\nopen",
        );
        assert_eq!(n.get("type"), Some("song"));
        assert_eq!(n.front_json()["title"], "Grace");
        assert!(n.body.starts_with("# Grace"));
        assert_eq!(
            n.blocks,
            [
                Block::Heading(1, "Grace".into()),
                Block::Paragraph("line one line two".into()),
                Block::Fence {
                    info: "kf verse".into(),
                    body: "| C | G |".into()
                },
                Block::Fence {
                    info: String::new(),
                    body: "open".into()
                },
            ]
        );
        assert_eq!(parse("no front").front, []);
    }
}
