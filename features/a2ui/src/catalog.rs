//! The component catalog: which A2UI components a surface may use, what
//! each takes, and the architect-ui component that draws it.
//!
//! The catalog is the security boundary. A component it does not list is
//! never drawn (the renderer shows a labelled placeholder in its place),
//! and the props it does list are the only ones read. Everything an agent
//! is told about the catalog — the JSON Schema ([`schema`]) and the
//! system prompt ([`prompt`]) — is generated from [`CATALOG`], so the
//! two cannot drift from what is drawn; `catalog.schema.json` in the
//! crate is the schema written out, and a test fails when it is stale.

use std::fmt::Write as _;

use serde_json::{Map, Value, json};

/// This catalog's id: what a `createSurface` names in `catalogId`.
pub const CATALOG_ID: &str = "https://architect.fasttrackstudio.dev/a2ui/catalog/v1";

/// The A2UI version it speaks.
pub const VERSION: &str = "v1.0";

/// What a prop holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prop {
    /// A `DynamicString`: a literal, a `{"path": …}` binding or a
    /// `{"call": …}` function.
    Text,
    /// A `DynamicNumber`.
    Number,
    /// A `DynamicBoolean`.
    Boolean,
    /// A `DynamicStringList`.
    TextList,
    /// One child component's id.
    Child,
    /// A `ChildList`: ids, or a template over a data-bound array.
    Children,
    /// An `Action`: `{"event": {"name": "<ACTION_ID>", "context": …}}`.
    Action,
    /// One of these words.
    OneOf(&'static [&'static str]),
    /// `[{"label": DynamicString, "value": string}]`.
    Options,
    /// `[{"title": DynamicString, "child": id}]`.
    Tabs,
}

/// One prop of a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropDef {
    pub name: &'static str,
    pub prop: Prop,
    pub required: bool,
    pub doc: &'static str,
}

const fn req(name: &'static str, prop: Prop, doc: &'static str) -> PropDef {
    PropDef {
        name,
        prop,
        required: true,
        doc,
    }
}

const fn opt(name: &'static str, prop: Prop, doc: &'static str) -> PropDef {
    PropDef {
        name,
        prop,
        required: false,
        doc,
    }
}

/// One component of the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentDef {
    /// Its A2UI name (`"Button"`).
    pub name: &'static str,
    /// The architect-ui component that draws it.
    pub draws_with: &'static str,
    pub doc: &'static str,
    pub props: &'static [PropDef],
}

const JUSTIFY: Prop = Prop::OneOf(&[
    "start",
    "center",
    "end",
    "spaceBetween",
    "spaceAround",
    "spaceEvenly",
    "stretch",
]);
const ALIGN: Prop = Prop::OneOf(&["start", "center", "end", "stretch"]);

/// Every component a surface may use.
pub const CATALOG: &[ComponentDef] = &[
    ComponentDef {
        name: "Text",
        draws_with: "architect_ui::Heading / architect_ui::Text",
        doc: "Text: a heading (h1-h5) or body, caption text.",
        props: &[
            req("text", Prop::Text, "What it says."),
            opt(
                "variant",
                Prop::OneOf(&["h1", "h2", "h3", "h4", "h5", "body", "caption"]),
                "Its style; body when absent.",
            ),
        ],
    },
    ComponentDef {
        name: "Row",
        draws_with: "architect_ui::HStack",
        doc: "Children side by side.",
        props: &[
            req("children", Prop::Children, "Its children."),
            opt("justify", JUSTIFY, "Along the row."),
            opt("align", ALIGN, "Across the row."),
        ],
    },
    ComponentDef {
        name: "Column",
        draws_with: "architect_ui::VStack",
        doc: "Children one under another.",
        props: &[
            req("children", Prop::Children, "Its children."),
            opt("justify", JUSTIFY, "Down the column."),
            opt("align", ALIGN, "Across the column."),
        ],
    },
    ComponentDef {
        name: "List",
        draws_with: "architect_ui::VStack / architect_ui::HStack",
        doc: "A list of children, often a template over a data-bound array.",
        props: &[
            req("children", Prop::Children, "Its items."),
            opt(
                "direction",
                Prop::OneOf(&["vertical", "horizontal"]),
                "Vertical when absent.",
            ),
        ],
    },
    ComponentDef {
        name: "Card",
        draws_with: "architect_ui::Card",
        doc: "A card around one child.",
        props: &[req("child", Prop::Child, "What it holds.")],
    },
    ComponentDef {
        name: "Divider",
        draws_with: "architect_ui::Divider",
        doc: "A rule between things.",
        props: &[opt(
            "axis",
            Prop::OneOf(&["horizontal", "vertical"]),
            "Horizontal when absent.",
        )],
    },
    ComponentDef {
        name: "Button",
        draws_with: "architect_ui::Button",
        doc: "A button: its child is its label, its action an Action List action the person runs by pressing it. An action that changes something waits for the person's yes.",
        props: &[
            req("child", Prop::Child, "Its label (a Text)."),
            req("action", Prop::Action, "What pressing it runs."),
            opt(
                "variant",
                Prop::OneOf(&["default", "primary", "borderless"]),
                "Its emphasis.",
            ),
        ],
    },
    ComponentDef {
        name: "CheckBox",
        draws_with: "architect_ui::Checkbox",
        doc: "A checkbox bound to a boolean.",
        props: &[
            req("label", Prop::Text, "What it says beside it."),
            req(
                "value",
                Prop::Boolean,
                "Bind it to a path to keep its state.",
            ),
        ],
    },
    ComponentDef {
        name: "TextField",
        draws_with: "architect_ui::Input / architect_ui::Textarea",
        doc: "A text field bound to a string.",
        props: &[
            req("label", Prop::Text, "What it asks for."),
            opt(
                "value",
                Prop::Text,
                "Bind it to a path to keep what is typed.",
            ),
            opt(
                "variant",
                Prop::OneOf(&["shortText", "longText", "number", "obscured"]),
                "shortText when absent.",
            ),
        ],
    },
    ComponentDef {
        name: "Slider",
        draws_with: "architect_ui::Slider",
        doc: "A slider bound to a number.",
        props: &[
            opt("label", Prop::Text, "What it sets."),
            req(
                "value",
                Prop::Number,
                "Bind it to a path to keep its value.",
            ),
            opt("min", Prop::Number, "0 when absent."),
            opt("max", Prop::Number, "100 when absent."),
        ],
    },
    ComponentDef {
        name: "ChoicePicker",
        draws_with: "architect_ui::NativeSelect / architect_ui::Checkbox",
        doc: "A choice among options: one (a select) or several (checkboxes).",
        props: &[
            opt("label", Prop::Text, "What it asks."),
            req("options", Prop::Options, "The choices."),
            req(
                "value",
                Prop::TextList,
                "The chosen values; bind it to a path.",
            ),
            opt(
                "variant",
                Prop::OneOf(&["mutuallyExclusive", "multipleSelection"]),
                "mutuallyExclusive when absent.",
            ),
        ],
    },
    ComponentDef {
        name: "Tabs",
        draws_with: "architect_ui::Tabs",
        doc: "Tabs, each showing one child.",
        props: &[
            req("tabs", Prop::Tabs, "Each tab's title and child."),
            opt(
                "activeTab",
                Prop::Number,
                "The tab on show; bind it to keep it.",
            ),
        ],
    },
    ComponentDef {
        name: "Image",
        draws_with: "img",
        doc: "An image from a URL.",
        props: &[
            req("url", Prop::Text, "Where it is."),
            opt(
                "description",
                Prop::Text,
                "What it shows, for a screen reader.",
            ),
            opt(
                "fit",
                Prop::OneOf(&["contain", "cover", "fill", "none", "scale-down"]),
                "contain when absent.",
            ),
        ],
    },
];

/// The catalog's component `name`, or `None` when it is not in it (and
/// so is never drawn).
#[must_use]
pub fn component(name: &str) -> Option<&'static ComponentDef> {
    CATALOG.iter().find(|c| c.name == name)
}

/// Every component's name, in catalog order.
#[must_use]
pub fn names() -> Vec<&'static str> {
    CATALOG.iter().map(|c| c.name).collect()
}

fn dynamic(literal: &Value) -> Value {
    json!({ "oneOf": [
        literal,
        { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] },
        { "type": "object", "properties": { "call": { "type": "string" }, "args": { "type": "object" } }, "required": ["call"] },
    ]})
}

fn prop_schema(prop: Prop) -> Value {
    match prop {
        Prop::Text => dynamic(&json!({ "type": "string" })),
        Prop::Number => dynamic(&json!({ "type": "number" })),
        Prop::Boolean => dynamic(&json!({ "type": "boolean" })),
        Prop::TextList => dynamic(&json!({ "type": "array", "items": { "type": "string" } })),
        Prop::Child => json!({ "type": "string", "description": "A component id." }),
        Prop::Children => json!({ "oneOf": [
            { "type": "array", "items": { "type": "string" } },
            { "type": "object", "properties": {
                "componentId": { "type": "string" }, "path": { "type": "string" } },
              "required": ["componentId", "path"] },
        ]}),
        Prop::Action => json!({ "type": "object", "properties": { "event": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "An Action List action id." },
                "context": { "type": "object", "description": "Its arguments: literals or bindings." },
                "wantResponse": { "type": "boolean" },
                "responsePath": { "type": "string" },
            },
            "required": ["name"],
        }}, "required": ["event"] }),
        Prop::OneOf(words) => json!({ "type": "string", "enum": words }),
        Prop::Options => json!({ "type": "array", "items": {
            "type": "object",
            "properties": { "label": dynamic(&json!({ "type": "string" })), "value": { "type": "string" } },
            "required": ["label", "value"],
        }}),
        Prop::Tabs => json!({ "type": "array", "items": {
            "type": "object",
            "properties": { "title": dynamic(&json!({ "type": "string" })), "child": { "type": "string" } },
            "required": ["title", "child"],
        }}),
    }
}

fn component_schema(def: &ComponentDef) -> Value {
    let mut properties = Map::new();
    properties.insert("id".to_owned(), json!({ "type": "string" }));
    properties.insert("component".to_owned(), json!({ "const": def.name }));
    properties.insert("weight".to_owned(), json!({ "type": "number" }));
    let mut required = vec![json!("id"), json!("component")];
    for p in def.props {
        let mut schema = prop_schema(p.prop);
        if let Some(object) = schema.as_object_mut() {
            object.insert("description".to_owned(), json!(p.doc));
        }
        properties.insert(p.name.to_owned(), schema);
        if p.required {
            required.push(json!(p.name));
        }
    }
    json!({
        "description": format!("{} Drawn with {}.", def.doc, def.draws_with),
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// The catalog's JSON Schema: one definition per component, and a
/// component as one of them.
#[must_use]
pub fn schema() -> Value {
    let mut defs = Map::new();
    for def in CATALOG {
        defs.insert(def.name.to_owned(), component_schema(def));
    }
    let any: Vec<Value> = CATALOG
        .iter()
        .map(|d| json!({ "$ref": format!("#/$defs/{}", d.name) }))
        .collect();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": CATALOG_ID,
        "title": "architect A2UI catalog",
        "description": format!("The A2UI {VERSION} components an architect host draws, with architect-ui. Generated from architect-a2ui's CATALOG: do not edit by hand."),
        "$defs": defs,
        "oneOf": any,
    })
}

/// The schema as `catalog.schema.json` holds it: pretty, with a final
/// newline.
#[must_use]
pub fn schema_text() -> String {
    let mut text = serde_json::to_string_pretty(&schema()).unwrap_or_default();
    text.push('\n');
    text
}

fn describe(prop: Prop) -> String {
    match prop {
        Prop::Text => "text (literal or {\"path\"})".to_owned(),
        Prop::Number => "number (literal or {\"path\"})".to_owned(),
        Prop::Boolean => "boolean (literal or {\"path\"})".to_owned(),
        Prop::TextList => "list of strings (literal or {\"path\"})".to_owned(),
        Prop::Child => "component id".to_owned(),
        Prop::Children => "[ids] or {\"componentId\", \"path\"} template".to_owned(),
        Prop::Action => "{\"event\": {\"name\": ACTION_ID, \"context\": {...}}}".to_owned(),
        Prop::OneOf(words) => format!("one of {}", words.join("|")),
        Prop::Options => "[{\"label\", \"value\"}]".to_owned(),
        Prop::Tabs => "[{\"title\", \"child\"}]".to_owned(),
    }
}

/// The system prompt that tells a model what it may draw: generated from
/// the catalog, as json-render's `catalog.prompt()` is.
#[must_use]
pub fn prompt() -> String {
    let mut out = format!(
        "You can show the person a UI by sending A2UI {VERSION} messages (createSurface with \
         catalogId \"{CATALOG_ID}\", updateComponents, updateDataModel, deleteSurface) to \
         A2UI_RENDER, and change it later with A2UI_PATCH. Components are a flat list with ids; \
         the root is \"root\". Only these components are drawn; any other shows as a \
         placeholder. A Button's action names an Action List action: one that changes \
         something waits for the person's yes. Read what the person entered with A2UI_SURFACE.\n"
    );
    for def in CATALOG {
        let _ = write!(out, "\n- {}: {}", def.name, def.doc);
        for p in def.props {
            let _ = write!(
                out,
                "\n  - {}{}: {}. {}",
                p.name,
                if p.required { "" } else { "?" },
                describe(p.prop),
                p.doc
            );
        }
    }
    out.push('\n');
    out
}

/// Why a component was not drawn: what the placeholder in its place says.
#[must_use]
pub fn refusal(name: &str) -> String {
    format!("“{name}” is not in this app's A2UI catalog, so it is not drawn")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_has_ten_or_more_components_each_once() {
        let mut seen = std::collections::BTreeSet::new();
        for def in CATALOG {
            assert!(seen.insert(def.name), "{} twice", def.name);
        }
        assert!(CATALOG.len() >= 10);
        assert!(component("Button").is_some());
        assert!(component("Script").is_none());
    }

    #[test]
    fn the_prompt_names_every_component() {
        let prompt = prompt();
        for def in CATALOG {
            assert!(prompt.contains(&format!("- {}:", def.name)));
        }
    }
}
