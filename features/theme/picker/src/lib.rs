//! The theme picker, a built-in extension (#272): ⌘K's "Choose a theme…"
//! (⌘O on `theme:`) and "Light or dark", and ⌘O's `theme:` source.
//!
//! One [`ThemePicker`] value runs it linked into an app (`Extensions::
//! with_linked`) or as its own process (`src/main.rs`, a development copy
//! made by `EXT_CLONE {"id": "theme.picker"}`). It reaches the theme
//! store only through the host service (the core's `THEME_*`), so the two
//! do the same, and its manifest (`extension.toml`) is all it adds: its
//! actions and its `[[openers]]`.

use architect_extensions::hostcall::HostRequest;
use architect_extensions::linked::{Builder, Extension, Handle};
use serde_json::{Value, json};

/// Its id.
pub const ID: &str = "theme.picker";

/// Its manifest.
pub const MANIFEST: &str = include_str!("../extension.toml");

/// The theme picker, as a `linked::Extension`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ThemePicker;

/// Runs app action `action` through the host service.
async fn run(host: &Handle, action: &str, args: Value) -> Result<Value, String> {
    host.host(HostRequest::Run {
        action: action.to_owned(),
        args,
        ticket: None,
    })
    .await
    .map_err(|e| format!("{action}: {e}"))
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The themes `query` finds in `THEME_LIST`'s answer, as ⌘O's items.
#[must_use]
pub fn found(list: &Value, query: &str) -> Vec<Value> {
    let showing = text(list, "showing");
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    list.get("themes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| {
            let hay = format!("{} {}", text(t, "label"), text(t, "id")).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|t| {
            let id = text(t, "id");
            let scheme = match text(t, "appearance") {
                "light" => "Light",
                "dark" => "Dark",
                _ => "Theme",
            };
            let detail = if id == showing {
                format!("{scheme} · showing")
            } else {
                scheme.to_owned()
            };
            json!({ "id": id, "title": text(t, "label"), "detail": detail })
        })
        .collect()
}

/// `THEME_SET`'s arguments that show `id` (of `appearance`, `light` or
/// `dark`) as the person's while `scheme` shows: its half of the pair,
/// and its appearance when the other one shows.
#[must_use]
pub fn pick(id: &str, appearance: &str, scheme: &str) -> Value {
    let half = if appearance == "light" {
        "light"
    } else {
        "dark"
    };
    let mut set = json!({ half: id });
    if half != scheme {
        set["appearance"] = json!(half);
    }
    set
}

impl Extension for ThemePicker {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        let host = ext.handle();
        let h = host.clone();
        ext.action("THEME_PICKER_CHOOSE", move |_| {
            let h = h.clone();
            async move {
                run(
                    &h,
                    "COMMANDS_SHOW",
                    json!({ "overlay": "quick-open", "query": "theme:" }),
                )
                .await
            }
        });
        let h = host.clone();
        ext.action("THEME_PICKER_SCHEME", move |_| {
            let h = h.clone();
            async move {
                let now = run(&h, "THEME_GET", json!({ "tokens": [] })).await?;
                let next = if text(&now, "scheme") == "dark" {
                    "light"
                } else {
                    "dark"
                };
                run(&h, "THEME_SET", json!({ "appearance": next })).await
            }
        });
        let h = host.clone();
        ext.action("THEME_PICKER_FIND", move |args| {
            let h = h.clone();
            async move {
                let list = run(&h, "THEME_LIST", Value::Null).await?;
                Ok(json!({ "items": found(&list, text(&args, "query")) }))
            }
        });
        let h = host;
        ext.action("THEME_PICKER_PICK", move |args| {
            let h = h.clone();
            async move {
                let id = text(&args, "id").to_owned();
                let list = run(&h, "THEME_LIST", Value::Null).await?;
                let theme = list
                    .get("themes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find(|t| text(t, "id") == id)
                    .cloned()
                    .ok_or_else(|| format!("no theme {id:?} (THEME_LIST has them)"))?;
                let now = run(&h, "THEME_GET", json!({ "tokens": [] })).await?;
                let set = pick(&id, text(&theme, "appearance"), text(&now, "scheme"));
                run(&h, "THEME_SET", set).await
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pick_sets_its_half_and_flips_the_appearance_only_when_needed() {
        assert_eq!(
            pick("gruvbox-dark", "dark", "dark"),
            json!({"dark": "gruvbox-dark"})
        );
        assert_eq!(
            pick("gruvbox-light", "light", "dark"),
            json!({"light": "gruvbox-light", "appearance": "light"})
        );
    }

    #[test]
    fn find_matches_every_word_and_says_which_shows() {
        let list = json!({
            "showing": "default-dark",
            "themes": [
                {"id": "default-dark", "label": "Default Dark", "appearance": "dark"},
                {"id": "gruvbox-light", "label": "Gruvbox Light", "appearance": "light"},
            ],
        });
        let all = found(&list, "");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["detail"], "Dark · showing");
        let one = found(&list, "gruv LIGHT");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0]["id"], "gruvbox-light");
    }
}
