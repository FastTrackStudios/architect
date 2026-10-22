//! `architect::schema` — a facet type as JSON Schema.
//!
//! Every value an action takes or returns is a facet type, so what it
//! looks like on the wire is already known: [`json_schema`] writes that
//! down in the form a tool-using model (MCP's `inputSchema`), a form, or
//! a validator reads. It describes what `facet_json` writes and reads —
//! the two must agree, or a caller that follows the schema is refused:
//!
//! - a struct is an object of its fields (by their wire name), the ones
//!   without a default required; a field's doc comment is its
//!   `description`, beside the field type's own keys
//!   (`{"type":"string","description":…}`), as a hand-written schema has
//!   it;
//! - an `Option<T>` field is described as `T` and left out of `required`:
//!   leaving it out is how a caller says "none" (`null` is still read);
//! - a struct marked `#[facet(deny_unknown_fields)]`, whose decoder
//!   refuses a field it does not have, says so:
//!   `"additionalProperties": false`. Without it, unknown fields are
//!   ignored on decode and the schema leaves them open;
//! - a unit-only enum is a string, one of its variants; an enum with data
//!   is one of `"Unit"` or `{"Variant": …}` (externally tagged);
//! - any other `Option<T>` is `T` or `null`, a list or set an array, a map an object
//!   of its values, a pointer (`Box`, `Arc`) what it points to;
//! - scalars by kind; a scalar facet knows only as opaque (a path, a
//!   uuid, a timestamp) is a string, which is how facet-json writes them.
//!
//! Recursive types stop at a depth ([`MAX_DEPTH`]) with an unconstrained
//! `{}` rather than looping.

use std::fmt::Write as _;

use facet::{Def, NumericType, PrimitiveType, Shape, StructKind, TextualType, Type, UserType};

/// How deep a schema nests before it stops describing (a recursive type
/// would otherwise never end).
pub const MAX_DEPTH: usize = 24;

/// The JSON Schema of `shape`, as JSON text.
#[must_use]
pub fn json_schema(shape: &'static Shape) -> String {
    let mut out = String::new();
    write_shape(&mut out, shape, 0);
    out
}

fn write_shape(out: &mut String, shape: &'static Shape, depth: usize) {
    if depth > MAX_DEPTH {
        out.push_str("{}");
        return;
    }
    out.push('{');
    let mut first = true;
    if !shape.doc.is_empty() {
        key(out, &mut first, "description");
        string(out, &doc(shape.doc));
    }
    write_body(out, &mut first, shape, depth);
    out.push('}');
}

/// The schema's own keys for `shape` (inside its braces).
fn write_body(out: &mut String, first: &mut bool, shape: &'static Shape, depth: usize) {
    match shape.def {
        Def::Option(option) => {
            key(out, first, "anyOf");
            out.push('[');
            write_shape(out, option.t(), depth.saturating_add(1));
            out.push_str(",{\"type\":\"null\"}]");
            return;
        }
        Def::List(list) => return array(out, first, list.t(), depth),
        Def::Set(set) => return array(out, first, set.t(), depth),
        Def::Slice(slice) => return array(out, first, slice.t(), depth),
        Def::Array(fixed) => {
            array(out, first, fixed.t(), depth);
            key(out, first, "minItems");
            out.push_str(&fixed.n.to_string());
            key(out, first, "maxItems");
            out.push_str(&fixed.n.to_string());
            return;
        }
        Def::Map(map) => {
            key(out, first, "type");
            string(out, "object");
            key(out, first, "additionalProperties");
            write_shape(out, map.v, depth.saturating_add(1));
            return;
        }
        Def::Pointer(pointer) => {
            if let Some(pointee) = pointer.pointee() {
                // What a pointer points to is what is on the wire.
                key(out, first, "allOf");
                out.push('[');
                write_shape(out, pointee, depth.saturating_add(1));
                out.push(']');
            }
            return;
        }
        _ => {}
    }
    match shape.ty {
        Type::Primitive(PrimitiveType::Boolean) => {
            key(out, first, "type");
            string(out, "boolean");
        }
        Type::Primitive(PrimitiveType::Numeric(NumericType::Integer { signed })) => {
            key(out, first, "type");
            string(out, "integer");
            if !signed {
                key(out, first, "minimum");
                out.push('0');
            }
        }
        Type::Primitive(PrimitiveType::Numeric(NumericType::Float)) => {
            key(out, first, "type");
            string(out, "number");
        }
        Type::Primitive(PrimitiveType::Textual(TextualType::Char)) => {
            key(out, first, "type");
            string(out, "string");
            key(out, first, "minLength");
            out.push('1');
            key(out, first, "maxLength");
            out.push('1');
        }
        Type::Primitive(PrimitiveType::Textual(TextualType::Str)) => {
            key(out, first, "type");
            string(out, "string");
        }
        Type::User(UserType::Struct(st)) => match st.kind {
            StructKind::Unit => {
                key(out, first, "type");
                string(out, "null");
            }
            StructKind::TupleStruct if st.fields.len() == 1 => {
                // A newtype is its one field on the wire.
                if let Some(field) = st.fields.first() {
                    key(out, first, "allOf");
                    out.push('[');
                    write_shape(out, field.shape(), depth.saturating_add(1));
                    out.push(']');
                }
            }
            StructKind::TupleStruct | StructKind::Tuple => {
                key(out, first, "type");
                string(out, "array");
                key(out, first, "prefixItems");
                out.push('[');
                for (i, field) in st.fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_shape(out, field.shape(), depth.saturating_add(1));
                }
                out.push(']');
            }
            StructKind::Struct => {
                object(out, first, st.fields, depth);
                if shape.has_deny_unknown_fields_attr() {
                    key(out, first, "additionalProperties");
                    out.push_str("false");
                }
            }
        },
        Type::User(UserType::Enum(en)) => {
            let unit_only = en.variants.iter().all(|v| v.data.fields.is_empty());
            if unit_only {
                key(out, first, "type");
                string(out, "string");
                key(out, first, "enum");
                out.push('[');
                for (i, variant) in en.variants.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    string(out, variant.rename.unwrap_or(variant.name));
                }
                out.push(']');
                return;
            }
            key(out, first, "oneOf");
            out.push('[');
            for (i, variant) in en.variants.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let name = variant.rename.unwrap_or(variant.name);
                if variant.data.fields.is_empty() {
                    out.push_str("{\"const\":");
                    string(out, name);
                    out.push('}');
                    continue;
                }
                out.push_str("{\"type\":\"object\",\"additionalProperties\":false,\"required\":[");
                string(out, name);
                out.push_str("],\"properties\":{");
                string(out, name);
                out.push_str(":{");
                let mut inner = true;
                if !variant.doc.is_empty() {
                    key(out, &mut inner, "description");
                    string(out, &doc(variant.doc));
                }
                match variant.data.kind {
                    StructKind::TupleStruct if variant.data.fields.len() == 1 => {
                        if let Some(field) = variant.data.fields.first() {
                            key(out, &mut inner, "allOf");
                            out.push('[');
                            write_shape(out, field.shape(), depth.saturating_add(1));
                            out.push(']');
                        }
                    }
                    StructKind::TupleStruct | StructKind::Tuple => {
                        key(out, &mut inner, "type");
                        string(out, "array");
                        key(out, &mut inner, "prefixItems");
                        out.push('[');
                        for (j, field) in variant.data.fields.iter().enumerate() {
                            if j > 0 {
                                out.push(',');
                            }
                            write_shape(out, field.shape(), depth.saturating_add(1));
                        }
                        out.push(']');
                    }
                    _ => object(out, &mut inner, variant.data.fields, depth),
                }
                out.push_str("}}}");
            }
            out.push(']');
        }
        // A scalar facet knows only as opaque (a path, a uuid, a time):
        // facet-json writes it as a string.
        _ if matches!(shape.def, Def::Scalar) => {
            key(out, first, "type");
            string(out, "string");
        }
        // Anything else is not described: any value.
        _ => {}
    }
}

fn array(out: &mut String, first: &mut bool, item: &'static Shape, depth: usize) {
    key(out, first, "type");
    string(out, "array");
    key(out, first, "items");
    write_shape(out, item, depth.saturating_add(1));
}

fn object(out: &mut String, first: &mut bool, fields: &'static [facet::Field], depth: usize) {
    key(out, first, "type");
    string(out, "object");
    key(out, first, "properties");
    out.push('{');
    let mut required: Vec<&str> = Vec::new();
    let mut i: usize = 0;
    for field in flattened(fields) {
        let name = field.effective_name();
        if i > 0 {
            out.push(',');
        }
        i = i.saturating_add(1);
        string(out, name);
        out.push(':');
        write_field(out, field, depth.saturating_add(1));
        let optional = field.has_default() || matches!(field.shape().def, Def::Option(_));
        if !optional {
            required.push(name);
        }
    }
    out.push('}');
    if !required.is_empty() {
        key(out, first, "required");
        out.push('[');
        for (j, name) in required.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            string(out, name);
        }
        out.push(']');
    }
}

/// One field's schema: its type's (an `Option`'s inner type's, the
/// field being optional anyway), described by the field's doc over the
/// type's — what this one is FOR — in the same object.
fn write_field(out: &mut String, field: &'static facet::Field, depth: usize) {
    if depth > MAX_DEPTH {
        out.push_str("{}");
        return;
    }
    let shape = match field.shape().def {
        Def::Option(option) => option.t(),
        _ => field.shape(),
    };
    let doc_lines = if field.doc.is_empty() {
        shape.doc
    } else {
        field.doc
    };
    out.push('{');
    let mut first = true;
    if !doc_lines.is_empty() {
        key(out, &mut first, "description");
        string(out, &doc(doc_lines));
    }
    write_body(out, &mut first, shape, depth);
    out.push('}');
}

/// `fields` as they are written: a flattened struct's own fields in its
/// place, as if they were these. A flattened field of any other shape has
/// none to give, and is left out.
fn flattened(fields: &'static [facet::Field]) -> Vec<&'static facet::Field> {
    let mut out = Vec::new();
    for field in fields {
        if !field.is_flattened() {
            out.push(field);
            continue;
        }
        if let Type::User(UserType::Struct(st)) = field.shape().ty {
            out.extend(flattened(st.fields));
        }
    }
    out
}

fn key(out: &mut String, first: &mut bool, name: &str) {
    if !*first {
        out.push(',');
    }
    *first = false;
    string(out, name);
    out.push(':');
}

/// A doc comment's lines as one description.
fn doc(lines: &[&str]) -> String {
    lines
        .iter()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned()
}

/// `text` as a JSON string.
fn string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::json_schema;
    use facet::Facet;

    /// Which hits to list.
    #[derive(Facet)]
    struct Query {
        /// The track, by GUID.
        track: String,
        /// How many at most.
        limit: Option<u32>,
        bands: Vec<Band>,
        how: Pick,
    }

    #[derive(Facet)]
    #[repr(u8)]
    enum Band {
        Low,
        High,
    }

    #[derive(Facet)]
    #[repr(u8)]
    #[allow(dead_code)]
    enum Pick {
        All,
        Strongest { count: u32 },
    }

    /// Settings a call may carry.
    #[derive(Facet, Default)]
    struct Tuning {
        /// How far under, dB.
        under_db: Option<f64>,
    }

    /// A call with its settings written in among its own fields.
    #[derive(Facet)]
    struct Tuned {
        #[facet(flatten)]
        tuning: Tuning,
        from: f64,
    }

    // A call that refuses fields it does not take (no doc: the test reads
    // the schema whole).
    #[derive(Facet)]
    #[facet(deny_unknown_fields)]
    struct Strict {
        /// Who to greet.
        name: Option<String>,
    }

    /// A struct whose decoder refuses unknown fields says so; a
    /// documented optional field is its type, flat, beside its doc.
    #[test]
    fn a_strict_struct_closes_its_object() {
        let schema = json_schema(Strict::SHAPE);
        assert_eq!(
            schema,
            "{\"type\":\"object\",\"properties\":{\"name\":{\"description\":\"Who to greet.\",\"type\":\"string\"}},\"additionalProperties\":false}"
        );
        assert!(facet_json::from_str::<Strict>(r#"{"nme":"a"}"#).is_err());
        assert!(facet_json::from_str::<Strict>(r#"{"name":null}"#).is_ok());
    }

    /// A flattened struct's fields are the call's own, documented, and
    /// the struct itself is no field.
    #[test]
    fn a_flattened_struct_is_its_fields() {
        let schema = json_schema(Tuned::SHAPE);
        assert!(
            schema.contains("\"under_db\":{\"description\":\"How far under, dB.\""),
            "{schema}"
        );
        assert!(!schema.contains("\"tuning\""), "{schema}");
        assert!(schema.contains("\"required\":[\"from\"]"), "{schema}");
    }

    /// A struct is an object of its fields, the optional one not
    /// required; docs ride along; a unit enum is a string of its
    /// variants; a data enum is externally tagged.
    #[test]
    fn a_type_is_described_as_its_wire_form() {
        let schema = json_schema(Query::SHAPE);
        assert!(
            schema.starts_with("{\"description\":\"Which hits to list.\""),
            "{schema}"
        );
        assert!(
            schema.contains("\"required\":[\"track\",\"bands\",\"how\"]"),
            "{schema}"
        );
        assert!(
            schema.contains("\"description\":\"The track, by GUID.\""),
            "{schema}"
        );
        assert!(
            schema.contains(
                "\"limit\":{\"description\":\"How many at most.\",\"type\":\"integer\",\"minimum\":0}"
            ),
            "{schema}"
        );
        assert!(!schema.contains("allOf"), "{schema}");
        assert!(!schema.contains("anyOf"), "{schema}");
        assert!(
            !schema.contains("\"additionalProperties\":false}"),
            "{schema}"
        );
        assert!(schema.contains("\"enum\":[\"Low\",\"High\"]"), "{schema}");
        assert!(schema.contains("{\"const\":\"All\"}"), "{schema}");
        assert!(schema.contains("\"required\":[\"Strongest\"]"), "{schema}");
    }
}
