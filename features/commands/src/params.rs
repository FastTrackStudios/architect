//! What an action takes, asked a step at a time.
//!
//! `actions-everywhere.md` §7, "Recall screenset ▸ Mix": an action's
//! params schema read into a list of [`Param`]s, each answer checked as
//! it is given, and the answers put together as the JSON the action runs
//! with.
//!
//! The schema is what `architect::schema::json_schema` writes for the
//! action's facet `Args`: an object of fields, the ones without a default
//! required. Each field is asked by its kind — a choice (a unit enum),
//! a number, a whole number, on/off, or text.

use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde_json::{Map, Value};

/// How a field is answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamKind {
    Text,
    /// Any number.
    Number,
    /// A whole number; `min` when it has one (an unsigned one is 0).
    Integer {
        min: Option<i64>,
    },
    /// On or off.
    Toggle,
    /// One of these.
    Choice(Vec<String>),
}

/// One field to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    /// Its wire name (`name`).
    pub name: String,
    /// What it says about itself (its doc comment), for the prompt.
    pub description: String,
    pub kind: ParamKind,
    /// It must be answered; an optional one may be skipped (left empty).
    pub required: bool,
}

impl Param {
    /// The prompt: the description, else the name as words.
    #[must_use]
    pub fn prompt(&self) -> String {
        if self.description.is_empty() {
            self.name.replace('_', " ")
        } else {
            self.description.clone()
        }
    }

    /// `text` as this field's value; `Ok(None)` when an optional field is
    /// left empty.
    ///
    /// # Errors
    ///
    /// Why the answer does not fit, to show beside the field.
    pub fn answer(&self, text: &str) -> Result<Option<Value>, String> {
        let text = text.trim();
        if text.is_empty() {
            return if self.required {
                Err(format!("{} is needed", self.prompt()))
            } else {
                Ok(None)
            };
        }
        let value = match &self.kind {
            ParamKind::Text => Value::String(text.to_owned()),
            ParamKind::Number => text
                .parse::<f64>()
                .ok()
                .and_then(|n| serde_json::Number::from_f64(n).map(Value::Number))
                .ok_or_else(|| format!("{text} is not a number"))?,
            ParamKind::Integer { min } => {
                let n = text
                    .parse::<i64>()
                    .map_err(|_| format!("{text} is not a whole number"))?;
                if let Some(min) = min
                    && n < *min
                {
                    return Err(format!("at least {min}"));
                }
                Value::from(n)
            }
            ParamKind::Toggle => match text.to_ascii_lowercase().as_str() {
                "on" | "yes" | "true" | "1" => Value::Bool(true),
                "off" | "no" | "false" | "0" => Value::Bool(false),
                _ => return Err("on or off".to_owned()),
            },
            ParamKind::Choice(options) => options
                .iter()
                .find(|o| o.eq_ignore_ascii_case(text))
                .map(|o| Value::String(o.clone()))
                .ok_or_else(|| format!("one of {}", options.join(", ")))?,
        };
        Ok(Some(value))
    }

    /// What can be picked for it, if it is picked rather than typed.
    #[must_use]
    pub fn options(&self) -> Vec<String> {
        match &self.kind {
            ParamKind::Choice(options) => options.clone(),
            ParamKind::Toggle => vec!["on".to_owned(), "off".to_owned()],
            _ => Vec::new(),
        }
    }
}

/// The fields of a params schema, in its order.
///
/// A schema that is not an object of fields this can ask for (a nested
/// struct, a list) gives `None`: such an action is run from the palette
/// with no arguments, and asked for by a caller that knows its shape (a
/// model, a script).
#[must_use]
pub fn params(schema: &str) -> Option<Vec<Param>> {
    let root: Root = serde_json::from_str(schema).ok()?;
    let properties = root.properties?.0;
    let required = root.required;
    properties
        .iter()
        .map(|(name, field)| {
            let (field, optional) = strip_null(field);
            let field = unwrap_schema(field);
            Some(Param {
                name: name.clone(),
                description: field
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                kind: kind(field)?,
                required: !optional && required.contains(name),
            })
        })
        .collect()
}

/// The fields to ask for before running an action with `schema`; `None`
/// when there are none to ask (it takes nothing, or nothing this can ask
/// for), so it runs at once.
#[must_use]
pub fn to_ask(schema: Option<&str>) -> Option<Vec<Param>> {
    params(schema?).filter(|p| !p.is_empty())
}

/// A params schema's root: its fields in the order it lists them (a
/// `serde_json::Map` would sort them), and which are required.
#[derive(Deserialize)]
struct Root {
    #[serde(default)]
    properties: Option<Ordered>,
    #[serde(default)]
    required: Vec<String>,
}

/// A JSON object's entries in their order.
struct Ordered(Vec<(String, Value)>);

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Ordered, M::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, Value>()? {
                    entries.push(entry);
                }
                Ok(Ordered(entries))
            }
        }
        deserializer.deserialize_map(Entries)
    }
}

/// The answers, by field, as the JSON an action runs with. Fields left
/// out are left out (an optional field's "none").
#[must_use]
pub fn args_json(answers: &[(String, Value)]) -> String {
    let map: Map<String, Value> = answers.iter().cloned().collect();
    Value::Object(map).to_string()
}

/// What a schema is, past an `allOf` of one (a newtype, a pointer).
fn unwrap_schema(schema: &Value) -> &Value {
    match schema.get("allOf").and_then(Value::as_array) {
        Some(all) if all.len() == 1 => all.first().map_or(schema, unwrap_schema),
        _ => schema,
    }
}

/// `T` of an `anyOf: [T, null]` (an `Option<T>`), and whether it was one.
fn strip_null(schema: &Value) -> (&Value, bool) {
    let Some(any) = schema.get("anyOf").and_then(Value::as_array) else {
        return (schema, false);
    };
    let mut not_null = any
        .iter()
        .filter(|s| s.get("type").and_then(Value::as_str) != Some("null"));
    match (not_null.next(), not_null.next()) {
        (Some(only), None) if any.len() == 2 => (only, true),
        _ => (schema, false),
    }
}

fn kind(field: &Value) -> Option<ParamKind> {
    if let Some(options) = field.get("enum").and_then(Value::as_array) {
        return Some(ParamKind::Choice(
            options
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        ));
    }
    match field.get("type").and_then(Value::as_str)? {
        "string" => Some(ParamKind::Text),
        "number" => Some(ParamKind::Number),
        "integer" => Some(ParamKind::Integer {
            min: field.get("minimum").and_then(Value::as_i64),
        }),
        "boolean" => Some(ParamKind::Toggle),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECALL: &str = r#"{"type":"object","properties":{
        "screenset":{"description":"Which screenset","type":"string","enum":["Mix","Edit","Record"]},
        "slot":{"type":"integer","minimum":0},
        "fade":{"anyOf":[{"type":"number"},{"type":"null"}]}
    },"required":["screenset","slot"]}"#;

    #[test]
    fn a_schema_is_asked_field_by_field() {
        let ps = params(RECALL).unwrap();
        let names: Vec<&str> = ps.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["screenset", "slot", "fade"]);
        let screenset = &ps[0];
        assert_eq!(screenset.prompt(), "Which screenset");
        assert_eq!(screenset.answer("mix").unwrap(), Some(Value::from("Mix")));
        assert!(screenset.answer("Sing").is_err());
        assert!(screenset.answer("").is_err(), "required");
        assert_eq!(ps[1].answer("-1"), Err("at least 0".to_owned()));
        assert_eq!(ps[2].answer("").unwrap(), None, "optional");
        assert!(!ps[2].required);
    }

    #[test]
    fn the_answers_are_the_actions_json() {
        let json = args_json(&[
            ("screenset".into(), Value::from("Mix")),
            ("slot".into(), Value::from(2)),
        ]);
        let back: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back["screenset"], "Mix");
        assert_eq!(back["slot"], 2);
    }

    #[test]
    fn what_takes_nothing_is_run_at_once() {
        assert!(to_ask(None).is_none());
        assert!(to_ask(Some(r#"{"type":"object","properties":{}}"#)).is_none());
        assert!(to_ask(Some(r#"{"type":"null"}"#)).is_none(), "()");
        assert_eq!(to_ask(Some(RECALL)).map(|p| p.len()), Some(3));
        assert!(
            params(r#"{"type":"object","properties":{"items":{"type":"array"}}}"#).is_none(),
            "a list is not asked for here"
        );
    }
}
