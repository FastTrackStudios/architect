//! A facet struct as a section (`#[derive(Settings)]`).
//!
//! The derive writes down only its attributes ([`SectionSpec`]); the rest is read
//! here from the struct's facet shape, the way `architect::schema` reads
//! it — a field's doc comment is its title (the first line) and its
//! description (the rest), its type picks the control, and the struct's
//! `Default` is every default.
//!
//! The type picks the control unless `control` says otherwise: `bool` a
//! switch; an enum of unit variants a select (or `segmented`, `cards`),
//! each variant's doc its label; an integer a stepper (or `slider`, or a
//! select of `choices`); a float a slider; a `String` a text field (or
//! `shortcut`, a recorder); a struct of such fields a titled block of its
//! own. A field of any other type is not on the page and keeps its
//! default.
//!
//! A setting's key is the section's id and the field's name
//! (`appearance.text_size`; a nested one `audio.input.gain`).

use facet::{Facet, NumericType, PrimitiveType, Shape, TextualType, Type, UserType};
use facet_value::{VNumber, VObject, Value as Tree};

use crate::model::{Choice, Control, Scope, Section, Setting, Value};
use crate::registry::Registry;
use crate::values::ScopedValues;

/// A section declared by a struct: `#[derive(Settings)]` writes [`spec`]
/// from the struct's attributes.
///
/// [`spec`]: Settings::spec
pub trait Settings: Facet<'static> + Default + Clone + 'static {
    /// The struct's `#[settings(..)]` and its fields' `#[setting(..)]`.
    fn spec() -> &'static SectionSpec;

    /// The section's id, the prefix of its keys.
    #[must_use]
    fn id() -> &'static str {
        Self::spec().id
    }

    /// The section, to register (`registry.register(Audio::section())`).
    #[must_use]
    fn section() -> Section {
        section_of::<Self>()
    }

    /// The value now: each field as `values` has it (the default where
    /// nothing is set, an org's lock where there is one).
    #[must_use]
    fn read(registry: &Registry, values: &ScopedValues) -> Self {
        read::<Self>(|key| values.get(registry, key))
    }

    /// Every field on the page as a key and its value.
    #[must_use]
    fn entries(&self) -> Vec<(String, Value)> {
        entries(self)
    }
}

/// A struct's `#[settings(..)]`, as the derive writes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionSpec {
    pub id: &'static str,
    /// The section's title; the struct's doc comment when `None`.
    pub title: Option<&'static str>,
    pub icon: Option<&'static str>,
    pub group: Option<&'static str>,
    pub scope: Scope,
    /// The fields with a `#[setting(..)]`.
    pub fields: &'static [FieldSpec],
}

impl SectionSpec {
    fn field(&self, name: &str) -> Option<&FieldSpec> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// A field's `#[setting(..)]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldSpec {
    pub name: &'static str,
    pub control: Option<ControlSpec>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub unit: &'static str,
    pub choices: &'static [ChoiceSpec],
    pub aliases: &'static [&'static str],
    pub placeholder: Option<&'static str>,
    pub skip: bool,
}

/// `control = ..`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlSpec {
    Switch,
    Select,
    Segmented,
    Cards,
    Stepper,
    Slider,
    Text,
    Shortcut,
}

/// One of `choices = [..]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChoiceSpec {
    Int(i64),
    Float(f64),
    Text(&'static str),
}

/// What a field holds, as far as the page goes.
#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    Float,
    Text,
    /// A unit-only enum.
    Enum(&'static [facet::Variant]),
    /// A struct of settings: a block of its own.
    Group(&'static [facet::Field]),
}

fn kind(shape: &'static Shape) -> Option<Kind> {
    match shape.ty {
        Type::Primitive(PrimitiveType::Boolean) => Some(Kind::Bool),
        Type::Primitive(PrimitiveType::Numeric(NumericType::Integer { .. })) => {
            let (min, max) = int_range(shape.type_identifier);
            Some(Kind::Int { min, max })
        }
        Type::Primitive(PrimitiveType::Numeric(NumericType::Float)) => Some(Kind::Float),
        Type::Primitive(PrimitiveType::Textual(TextualType::Str)) => Some(Kind::Text),
        Type::User(UserType::Enum(en)) if en.variants.iter().all(|v| v.data.fields.is_empty()) => {
            Some(Kind::Enum(en.variants))
        }
        Type::User(UserType::Struct(st)) if st.kind == facet::StructKind::Struct => {
            Some(Kind::Group(st.fields))
        }
        _ if shape.type_identifier == "String" => Some(Kind::Text),
        _ => None,
    }
}

/// An integer type's range, as far as an `i64` holds it.
fn int_range(ty: &str) -> (i64, i64) {
    match ty {
        "u8" => (0, i64::from(u8::MAX)),
        "u16" => (0, i64::from(u16::MAX)),
        "u32" => (0, i64::from(u32::MAX)),
        "u64" | "u128" | "usize" => (0, i64::MAX),
        "i8" => (i64::from(i8::MIN), i64::from(i8::MAX)),
        "i16" => (i64::from(i16::MIN), i64::from(i16::MAX)),
        "i32" => (i64::from(i32::MIN), i64::from(i32::MAX)),
        _ => (i64::MIN, i64::MAX),
    }
}

/// The section `T` declares.
#[must_use]
pub fn section_of<T: Settings>() -> Section {
    let spec = T::spec();
    let shape = T::SHAPE;
    let title = spec
        .title
        .map(str::to_owned)
        .or_else(|| first_line(shape.doc))
        .unwrap_or_else(|| words(shape.type_identifier));
    let mut section = Section::new(spec.id, title);
    if let Some(icon) = spec.icon {
        section = section.icon(icon);
    }
    if let Some(group) = spec.group {
        section = section.group(group);
    }
    let defaults = tree_of(&T::default());
    let defaults = defaults.as_ref().and_then(Tree::as_object);
    let mut top = Vec::new();
    let mut groups = Vec::new();
    for field in top_fields::<T>() {
        let Some(k) = kind(field.shape()) else {
            continue;
        };
        let fs = spec.field(field.name);
        let default = defaults.and_then(|d| d.get(field.effective_name()));
        let key = format!("{}.{}", spec.id, field.name);
        match k {
            Kind::Group(fields) => {
                let inner = default.and_then(Tree::as_object);
                let settings: Vec<Setting> = fields
                    .iter()
                    .filter_map(|f| {
                        let k = kind(f.shape()).filter(|k| !matches!(k, Kind::Group(_)))?;
                        let d = inner.and_then(|o| o.get(f.effective_name()));
                        setting(&format!("{key}.{}", f.name), f, k, None, d, spec.scope)
                    })
                    .collect();
                if !settings.is_empty() {
                    let title = first_line(field.doc).unwrap_or_else(|| words(field.name));
                    groups.push((title, settings));
                }
            }
            k => top.extend(setting(&key, field, k, fs, default, spec.scope)),
        }
    }
    if !top.is_empty() {
        section = section.block(None, top);
    }
    for (title, settings) in groups {
        section = section.block(Some(&title), settings);
    }
    section
}

/// `T`'s fields that are settings (not `skip`ped).
fn top_fields<T: Settings>() -> impl Iterator<Item = &'static facet::Field> {
    let spec = T::spec();
    let fields: &'static [facet::Field] = match T::SHAPE.ty {
        Type::User(UserType::Struct(st)) => st.fields,
        _ => &[],
    };
    fields
        .iter()
        .filter(move |f| !spec.field(f.name).is_some_and(|s| s.skip))
}

fn setting(
    key: &str,
    field: &'static facet::Field,
    kind: Kind,
    spec: Option<&FieldSpec>,
    default: Option<&Tree>,
    scope: Scope,
) -> Option<Setting> {
    let control = control(kind, spec)?;
    let default = default
        .and_then(|d| from_tree(kind, &control, d))
        .unwrap_or_else(|| fallback(&control));
    let title = first_line(field.doc).unwrap_or_else(|| words(field.name));
    let mut s = Setting::new(key, title, control, default)
        .describe(rest(field.doc))
        .scope(scope);
    if let Some(spec) = spec {
        s = s.aliases(spec.aliases.iter().copied());
    }
    Some(s)
}

fn control(kind: Kind, spec: Option<&FieldSpec>) -> Option<Control> {
    let asked = spec.and_then(|s| s.control);
    let unit = spec.map(|s| s.unit).unwrap_or_default().to_owned();
    let choices = spec.map(|s| s.choices).unwrap_or_default();
    let pick = |list: Vec<Choice>| match asked {
        Some(ControlSpec::Segmented) => Control::Segmented(list),
        Some(ControlSpec::Cards) => Control::Cards(list),
        _ => Control::Select(list),
    };
    Some(match kind {
        Kind::Bool => Control::Switch,
        Kind::Enum(variants) => pick(
            variants
                .iter()
                .map(|v| {
                    Choice::new(
                        Value::Text(v.effective_name().to_owned()),
                        first_line(v.doc).unwrap_or_else(|| words(v.name)),
                    )
                    .detail(rest(v.doc))
                })
                .collect(),
        ),
        Kind::Int { .. } | Kind::Float | Kind::Text if !choices.is_empty() => pick(
            choices
                .iter()
                .map(|c| {
                    let (value, shown) = match *c {
                        ChoiceSpec::Int(i) => (Value::Int(i), i.to_string()),
                        ChoiceSpec::Float(f) => (Value::Float(f), f.to_string()),
                        ChoiceSpec::Text(t) => (Value::Text(t.to_owned()), t.to_owned()),
                    };
                    let label = if unit.is_empty() {
                        shown
                    } else {
                        format!("{shown} {unit}")
                    };
                    Choice::new(value, label)
                })
                .collect(),
        ),
        Kind::Int { min, max } if asked != Some(ControlSpec::Slider) => {
            let lo = spec.and_then(|s| s.min).map_or(min, |m| to_int(m).max(min));
            let hi = spec.and_then(|s| s.max).map_or(max, |m| to_int(m).min(max));
            let step = spec.and_then(|s| s.step).map_or(1, to_int).max(1);
            Control::Stepper {
                min: lo,
                max: hi,
                step,
                unit,
            }
        }
        Kind::Int { .. } | Kind::Float => Control::Slider {
            min: spec.and_then(|s| s.min).unwrap_or(0.0),
            max: spec.and_then(|s| s.max).unwrap_or(1.0),
            step: spec.and_then(|s| s.step).unwrap_or(match kind {
                Kind::Float => 0.01,
                _ => 1.0,
            }),
            unit,
        },
        Kind::Text if asked == Some(ControlSpec::Shortcut) => Control::Shortcut,
        Kind::Text => Control::Text {
            placeholder: spec
                .and_then(|s| s.placeholder)
                .unwrap_or_default()
                .to_owned(),
        },
        Kind::Group(_) => return None,
    })
}

/// What a control holds when the struct's default could not be read.
fn fallback(control: &Control) -> Value {
    match control {
        Control::Switch => Value::Bool(false),
        Control::Select(c) | Control::Segmented(c) | Control::Cards(c) => c
            .first()
            .map_or_else(|| Value::Text(String::new()), |c| c.value.clone()),
        Control::Slider { min, .. } => Value::Float(*min),
        Control::Stepper { min, .. } => Value::Int(*min),
        Control::Text { .. } | Control::Shortcut => Value::Text(String::new()),
    }
}

/// A field's value as a setting holds it (a slider holds a float, even
/// over an integer field).
fn from_tree(kind: Kind, control: &Control, tree: &Tree) -> Option<Value> {
    let slider = matches!(control, Control::Slider { .. });
    match kind {
        Kind::Bool => tree.as_bool().map(Value::Bool),
        Kind::Int { .. } if slider => tree.as_number().map(|n| Value::Float(n.to_f64_lossy())),
        Kind::Int { .. } => tree.as_number().and_then(VNumber::to_i64).map(Value::Int),
        Kind::Float => tree.as_number().map(|n| Value::Float(n.to_f64_lossy())),
        Kind::Text | Kind::Enum(_) => tree.as_string().map(|s| Value::Text(s.as_str().to_owned())),
        Kind::Group(_) => None,
    }
}

/// A setting's value as the field takes it.
fn to_tree(kind: Kind, value: &Value) -> Option<Tree> {
    match (kind, value) {
        (Kind::Bool, Value::Bool(b)) => Some(Tree::from(*b)),
        (Kind::Int { min, max }, Value::Int(i)) => {
            Some(Tree::from(VNumber::from_i64((*i).clamp(min, max))))
        }
        (Kind::Int { min, max }, Value::Float(f)) => {
            Some(Tree::from(VNumber::from_i64(to_int(*f).clamp(min, max))))
        }
        (Kind::Float, Value::Float(f)) => Some(Tree::from(*f)),
        (Kind::Text | Kind::Enum(_), Value::Text(t)) => Some(Tree::from(t.as_str())),
        _ => None,
    }
}

/// `f` rounded to an integer, saturating.
#[allow(
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    reason = "a float to an integer saturates in Rust, which is what's wanted"
)]
const fn to_int(f: f64) -> i64 {
    f.round() as i64
}

fn tree_of<T: Facet<'static>>(value: &T) -> Option<Tree> {
    facet_value::to_value(value).ok()
}

/// `T` as `get` has it: each field on the page from `get`, every other
/// from `T::default()`. A field `get` holds something the type will not
/// take keeps its default.
pub fn read<T: Settings>(get: impl Fn(&str) -> Option<Value>) -> T {
    let Some(mut tree) = tree_of(&T::default()) else {
        return T::default();
    };
    let id = T::spec().id;
    let mut edits: Vec<(Vec<&'static str>, Tree)> = Vec::new();
    for field in top_fields::<T>() {
        let Some(k) = kind(field.shape()) else {
            continue;
        };
        let key = format!("{id}.{}", field.name);
        if let Kind::Group(fields) = k {
            for f in fields {
                let Some(fk) = kind(f.shape()).filter(|k| !matches!(k, Kind::Group(_))) else {
                    continue;
                };
                if let Some(t) = get(&format!("{key}.{}", f.name)).and_then(|v| to_tree(fk, &v)) {
                    edits.push((vec![field.effective_name(), f.effective_name()], t));
                }
            }
        } else if let Some(t) = get(&key).and_then(|v| to_tree(k, &v)) {
            edits.push((vec![field.effective_name()], t));
        }
    }
    // All at once; when the type refuses that, one at a time, keeping
    // each it takes.
    let all = apply(&tree, &edits);
    if let Some(v) = all.and_then(|t| facet_value::from_value::<T>(t).ok()) {
        return v;
    }
    for edit in &edits {
        if let Some(next) = apply(&tree, std::slice::from_ref(edit)) {
            if facet_value::from_value::<T>(next.clone()).is_ok() {
                tree = next;
            }
        }
    }
    facet_value::from_value::<T>(tree).unwrap_or_default()
}

fn apply(tree: &Tree, edits: &[(Vec<&'static str>, Tree)]) -> Option<Tree> {
    let mut root = tree.clone();
    for (path, value) in edits {
        let obj = root.as_object_mut()?;
        match path.as_slice() {
            [name] => {
                obj.insert(*name, value.clone());
            }
            [outer, name] => {
                let inner = obj.get_mut(outer)?.as_object_mut()?;
                inner.insert(*name, value.clone());
            }
            _ => {}
        }
    }
    Some(root)
}

/// Every field of `value` on the page, as a key and its value.
pub fn entries<T: Settings>(value: &T) -> Vec<(String, Value)> {
    let Some(tree) = tree_of(value) else {
        return Vec::new();
    };
    let Some(obj) = tree.as_object() else {
        return Vec::new();
    };
    let spec = T::spec();
    let mut out = Vec::new();
    for field in top_fields::<T>() {
        let Some(k) = kind(field.shape()) else {
            continue;
        };
        let key = format!("{}.{}", spec.id, field.name);
        let Some(t) = obj.get(field.effective_name()) else {
            continue;
        };
        if let Kind::Group(fields) = k {
            let Some(inner) = t.as_object() else { continue };
            out.extend(group_entries(&key, fields, inner));
        } else if let Some(control) = control(k, spec.field(field.name)) {
            out.extend(from_tree(k, &control, t).map(|v| (key, v)));
        }
    }
    out
}

fn group_entries(
    key: &str,
    fields: &'static [facet::Field],
    obj: &VObject,
) -> Vec<(String, Value)> {
    fields
        .iter()
        .filter_map(|f| {
            let k = kind(f.shape()).filter(|k| !matches!(k, Kind::Group(_)))?;
            let control = control(k, None)?;
            let v = from_tree(k, &control, obj.get(f.effective_name())?)?;
            Some((format!("{key}.{}", f.name), v))
        })
        .collect()
}

fn first_line(doc: &[&str]) -> Option<String> {
    doc.iter()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(str::to_owned)
}

/// A doc comment after its first line, as one paragraph.
fn rest(doc: &[&str]) -> String {
    doc.iter()
        .map(|l| l.trim())
        .skip_while(|l| l.is_empty())
        .skip(1)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `text_size` or `TextSize` → `Text size`.
fn words(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c == '_' {
            out.push(' ');
        } else if c.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}
