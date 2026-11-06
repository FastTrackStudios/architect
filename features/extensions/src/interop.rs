//! What extensions declare to each other and to the app's windows.
//!
//! (`docs/content/architecture/extensions.md`, "Windows, points and
//! services".) The manifest's `[[windows]]`, `[[points]]`,
//! `[[contribute."<point>"]]`, `[provides]`, `[requires]` and `reserved`,
//! read and checked here; the host keeps what they add
//! (`crate::points`, `crate::host`).
//!
//! ```toml
//! reserved = ["APP_SETTINGS"]         # app ids it owns (an app allows it)
//!
//! [[windows]]                         # a window of its own
//! id = "settings"
//! view = "sheet"                      # one of its [[views]]
//! title = "Settings"
//! kind = "sheet"                      # over the main window, or "window": an OS window
//! opens = "APP_SETTINGS"              # the action that opens it (the host runs it)
//!
//! [[points]]                          # what others may add to it
//! id = "settings.sections"            # in its namespace (`<id>.…`)
//! deliver = "SETTINGS_SECTIONS"       # its action told {point, items} as they change
//! schema = { type = "object", required = ["id", "title"] }
//!
//! [[contribute."settings.sections"]]  # an item for another's point
//! id = "metronome"
//! title = "Metronome"
//!
//! [provides]
//! "lab.tempo" = "1.2.0"               # a service: method `m` is its action <PREFIX>M
//!
//! [requires]
//! "lab.tempo" = "^1"                  # loaded after a provider; refused without one
//! ```

use std::collections::BTreeMap;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The point every `[[sections]]` table contributes to: the settings
/// page's sections (`[[sections]]` is sugar for
/// `[[contribute."settings.sections"]]`).
pub const SETTINGS_SECTIONS: &str = "settings.sections";

/// Who the app itself contributes as (its own settings sections).
pub const APP: &str = "app";

/// What a window is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowKind {
    /// A sheet over the app's main window: a scrim, a card, closed by
    /// Escape, a click outside, or its own close.
    #[default]
    Sheet,
    /// An OS window of its own (the workspace's pop-out windows' machinery).
    Window,
}

impl WindowKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sheet => "sheet",
            Self::Window => "window",
        }
    }
}

/// A window it owns (`[[windows]]`): one of its views, drawn by the
/// app's window machinery, opened by an action.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WindowDecl {
    /// Unique among its windows.
    pub id: String,
    /// One of its `[[views]]`: what fills the window.
    pub view: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub kind: WindowKind,
    /// The action that opens it: one of its `[[actions]]`, whose handler is
    /// the host's (its arguments are the window's state). Empty: opened
    /// only through the host service (`window`).
    #[serde(default)]
    pub opens: String,
    /// Its size, points: a sheet's at most the window's, an OS window's
    /// first size. Unset: the sheet's own (92 % of the window, at most
    /// 1100 × 760), 720 × 560 for a window.
    #[serde(default)]
    pub width: Option<f32>,
    #[serde(default)]
    pub height: Option<f32>,
}

/// A contribution point it declares (`[[points]]`): what other
/// extensions (and the app) may add to it, each item checked against
/// `schema`, the set delivered to it live.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PointDecl {
    /// `<its id>.<name>` (`settings.sections`).
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The JSON Schema each item meets (a TOML table, or JSON text), the
    /// subset [`check_schema`] reads. None: any object.
    #[serde(default)]
    pub schema: Option<toml::Value>,
    /// One of its actions, run with `{point, items: [{from, value}]}`
    /// when it loads and whenever the set changes.
    #[serde(default)]
    pub deliver: Option<String>,
}

impl PointDecl {
    /// Its schema as JSON (`{"type": "object"}` when it gives none).
    #[must_use]
    pub fn schema_json(&self) -> Value {
        match &self.schema {
            None => serde_json::json!({ "type": "object" }),
            Some(toml::Value::String(text)) => {
                serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
            }
            Some(other) => toml_json(other),
        }
    }
}

/// A TOML value as JSON.
#[must_use]
pub fn toml_json(v: &toml::Value) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Why a declaration of this module's is refused: the field, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bad {
    pub field: String,
    pub why: String,
}

fn bad(field: impl Into<String>, why: impl Into<String>) -> Bad {
    Bad {
        field: field.into(),
        why: why.into(),
    }
}

/// A point's, a service's name: lower case words joined by `.`, `-`, `_`.
#[must_use]
pub fn good_name(name: &str) -> bool {
    !name.is_empty()
        && name.contains('.')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ".-_".contains(c))
        && !name.starts_with('.')
        && !name.ends_with('.')
}

/// An action id an app keeps for itself: `APP_…`. An extension may own
/// one only when it lists it in `reserved` and the app allows it
/// (`Host::allow_reserved`).
#[must_use]
pub fn is_reserved(action: &str) -> bool {
    action.starts_with("APP_")
}

/// What [`check`] reads from a manifest.
pub struct Decls<'a> {
    pub id: &'a str,
    pub actions: &'a [String],
    pub views: &'a [String],
    pub windows: &'a [WindowDecl],
    pub points: &'a [PointDecl],
    pub contribute: &'a BTreeMap<String, Vec<toml::Value>>,
    pub provides: &'a BTreeMap<String, String>,
    pub requires: &'a BTreeMap<String, String>,
    pub uses: &'a BTreeMap<String, String>,
    pub reserved: &'a [String],
}

/// Checks a manifest's windows, points, contributions to points,
/// services and reserved ids.
///
/// # Errors
///
/// The first rule broken.
pub fn check(d: &Decls<'_>) -> Result<(), Bad> {
    for r in d.reserved {
        if !is_reserved(r) {
            return Err(bad(
                format!("reserved[{r}]"),
                "only an app's own ids (APP_…) are reserved",
            ));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for w in d.windows {
        let field = |f: &str| format!("windows[{}].{f}", w.id);
        if w.id.is_empty() || !seen.insert(w.id.as_str()) {
            return Err(bad(field("id"), "empty, or two windows share it"));
        }
        if !d.views.contains(&w.view) {
            return Err(bad(
                field("view"),
                format!("it has no view {:?} ([[views]])", w.view),
            ));
        }
        if !w.opens.is_empty() && !d.actions.contains(&w.opens) {
            return Err(bad(
                field("opens"),
                format!("{} is not one of its [[actions]]", w.opens),
            ));
        }
        for (n, size) in [("width", w.width), ("height", w.height)] {
            if size.is_some_and(|s| !(s.is_finite() && s > 0.0)) {
                return Err(bad(field(n), "not a size"));
            }
        }
    }
    let own = format!("{}.", d.id);
    let mut seen = std::collections::BTreeSet::new();
    for p in d.points {
        let field = |f: &str| format!("points[{}].{f}", p.id);
        if !good_name(&p.id) || !p.id.starts_with(&own) || !seen.insert(p.id.as_str()) {
            return Err(bad(
                field("id"),
                format!("a point is named in its namespace ({own}…), once"),
            ));
        }
        check_schema(&p.schema_json()).map_err(|why| bad(field("schema"), why))?;
        if let Some(deliver) = &p.deliver
            && !d.actions.iter().any(|a| a == deliver)
        {
            return Err(bad(
                field("deliver"),
                format!("{deliver} is not one of its [[actions]]"),
            ));
        }
    }
    for (point, items) in d.contribute {
        if !good_name(point) {
            return Err(bad(format!("contribute.{point}"), "not a point's name"));
        }
        if let Some(i) = items.iter().position(|v| !v.is_table()) {
            return Err(bad(format!("contribute.{point}[{i}]"), "not a table"));
        }
    }
    for (service, version) in d.provides {
        if !good_name(service) {
            return Err(bad(format!("provides.{service}"), "not a service's name"));
        }
        Version::parse(version)
            .map_err(|e| bad(format!("provides.{service}"), format!("{version}: {e}")))?;
    }
    for (table, list) in [("requires", d.requires), ("uses", d.uses)] {
        for (service, req) in list {
            let field = format!("{table}.{service}");
            if !good_name(service) {
                return Err(bad(field, "not a service's name"));
            }
            VersionReq::parse(req).map_err(|e| bad(field.clone(), format!("{req}: {e}")))?;
            if d.provides.contains_key(service) {
                return Err(bad(field, "it provides that service itself"));
            }
        }
    }
    if let Some(service) = d.uses.keys().find(|s| d.requires.contains_key(*s)) {
        return Err(bad(
            format!("uses.{service}"),
            "it is in [requires] too: one or the other",
        ));
    }
    Ok(())
}

/// The action that answers method `method` of a service: `<PREFIX><METHOD>`.
///
/// `prefix` is its provider's action namespace; the method is upper-cased,
/// each run of other characters one `_` (`find-all` → `TEMPO_FIND_ALL`).
#[must_use]
pub fn method_action(prefix: &str, method: &str) -> String {
    let mut out = prefix.to_owned();
    let mut last_sep = out.ends_with('_');
    for c in method.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
            last_sep = false;
        } else if !last_sep {
            out.push('_');
            last_sep = true;
        }
    }
    out
}

// ---- the schema subset ------------------------------------------------

const TYPES: [&str; 7] = [
    "object", "array", "string", "number", "integer", "boolean", "null",
];

/// Checks that `schema` is one this host reads.
///
/// An object whose `type` (one name or a list), `properties`, `required`, `items`, `enum` and
/// `additionalProperties` (true or false) are the right shapes. Other
/// keywords (`description`, `title`, `default`) are let be.
///
/// # Errors
///
/// What is wrong with it, in words.
pub fn check_schema(schema: &Value) -> Result<(), String> {
    let Value::Object(map) = schema else {
        return Err("a schema is an object".to_owned());
    };
    match map.get("type") {
        None => {}
        Some(Value::String(t)) if TYPES.contains(&t.as_str()) => {}
        Some(Value::Array(ts))
            if ts
                .iter()
                .all(|t| t.as_str().is_some_and(|t| TYPES.contains(&t))) => {}
        Some(other) => return Err(format!("type {other} is not one of {}", TYPES.join(", "))),
    }
    if let Some(props) = map.get("properties") {
        let Value::Object(props) = props else {
            return Err("properties is an object".to_owned());
        };
        for (name, sub) in props {
            check_schema(sub).map_err(|e| format!("properties.{name}: {e}"))?;
        }
    }
    if let Some(req) = map.get("required")
        && !req
            .as_array()
            .is_some_and(|r| r.iter().all(Value::is_string))
    {
        return Err("required is a list of names".to_owned());
    }
    if let Some(items) = map.get("items") {
        check_schema(items).map_err(|e| format!("items: {e}"))?;
    }
    if let Some(e) = map.get("enum")
        && !e.is_array()
    {
        return Err("enum is a list".to_owned());
    }
    if let Some(a) = map.get("additionalProperties")
        && !a.is_boolean()
    {
        return Err("additionalProperties is true or false".to_owned());
    }
    Ok(())
}

fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_ok(want: &str, got: &'static str) -> bool {
    want == got || (want == "number" && got == "integer")
}

/// Checks `value` against `schema` (the subset [`check_schema`] reads).
///
/// Where it first differs, as `path: why`.
///
/// # Errors
///
/// The first place it does not meet the schema.
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    validate_at(schema, value, "")
}

fn validate_at(schema: &Value, value: &Value, at: &str) -> Result<(), String> {
    let Value::Object(s) = schema else {
        return Ok(());
    };
    let here = || {
        if at.is_empty() {
            "the item".to_owned()
        } else {
            at.to_owned()
        }
    };
    let got = type_of(value);
    match s.get("type") {
        Some(Value::String(t)) if !type_ok(t, got) => {
            return Err(format!("{}: is {got}, not {t}", here()));
        }
        Some(Value::Array(ts)) if !ts.iter().filter_map(Value::as_str).any(|t| type_ok(t, got)) => {
            return Err(format!("{}: is {got}", here()));
        }
        _ => {}
    }
    if let Some(Value::Array(choices)) = s.get("enum")
        && !choices.contains(value)
    {
        return Err(format!(
            "{}: {value} is not one of {}",
            here(),
            Value::Array(choices.clone())
        ));
    }
    if let Value::Object(obj) = value {
        if let Some(Value::Array(req)) = s.get("required") {
            for name in req.iter().filter_map(Value::as_str) {
                if !obj.contains_key(name) {
                    return Err(format!("{}: has no {name}", here()));
                }
            }
        }
        let props = s.get("properties").and_then(Value::as_object);
        for (name, v) in obj {
            let path = if at.is_empty() {
                name.clone()
            } else {
                format!("{at}.{name}")
            };
            match props.and_then(|p| p.get(name)) {
                Some(sub) => validate_at(sub, v, &path)?,
                None if s.get("additionalProperties") == Some(&Value::Bool(false)) => {
                    return Err(format!("{path}: not one of its fields"));
                }
                None => {}
            }
        }
    }
    if let (Value::Array(list), Some(items)) = (value, s.get("items")) {
        for (i, v) in list.iter().enumerate() {
            validate_at(items, v, &format!("{at}[{i}]"))?;
        }
    }
    Ok(())
}

/// A service's provider as the host chooses it: the newest loaded
/// version that meets `req`, from `providers` (id and version).
#[must_use]
pub fn choose_provider<'a>(
    providers: &'a [(String, Version)],
    req: &VersionReq,
) -> Option<&'a (String, Version)> {
    providers
        .iter()
        .filter(|(_, v)| req.matches(v))
        .max_by(|a, b| a.1.cmp(&b.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_schema_checks_an_item() {
        let s = json!({
            "type": "object",
            "required": ["id", "title"],
            "properties": {
                "id": {"type": "string"},
                "title": {"type": "string"},
                "n": {"type": "number"},
                "kind": {"enum": ["a", "b"]},
                "tags": {"type": "array", "items": {"type": "string"}}
            }
        });
        check_schema(&s).unwrap();
        validate(&s, &json!({"id": "x", "title": "X", "n": 2})).unwrap();
        assert_eq!(
            validate(&s, &json!({"id": "x"})).unwrap_err(),
            "the item: has no title"
        );
        assert_eq!(
            validate(&s, &json!({"id": 1, "title": "X"})).unwrap_err(),
            "id: is integer, not string"
        );
        assert!(validate(&s, &json!({"id": "x", "title": "X", "kind": "c"})).is_err());
        assert_eq!(
            validate(&s, &json!({"id": "x", "title": "X", "tags": ["a", 2]})).unwrap_err(),
            "tags[1]: is integer, not string"
        );
        assert!(check_schema(&json!({"type": "thing"})).is_err());
        assert!(check_schema(&json!({"required": "id"})).is_err());
    }

    #[test]
    fn a_method_is_its_providers_action() {
        assert_eq!(method_action("LAB_TEMPO_", "get"), "LAB_TEMPO_GET");
        assert_eq!(
            method_action("LAB_TEMPO_", "find-all"),
            "LAB_TEMPO_FIND_ALL"
        );
    }

    #[test]
    fn the_newest_provider_that_meets_the_requirement() {
        let p = vec![
            ("a".to_owned(), Version::new(1, 2, 0)),
            ("b".to_owned(), Version::new(1, 4, 0)),
            ("c".to_owned(), Version::new(2, 0, 0)),
        ];
        let req = VersionReq::parse("^1").unwrap();
        assert_eq!(choose_provider(&p, &req).unwrap().0, "b");
        assert!(choose_provider(&p, &VersionReq::parse("^3").unwrap()).is_none());
    }
}
