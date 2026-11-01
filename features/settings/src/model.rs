//! What a setting is, as data: so an extension can declare one at run
//! time as well as the app at build time.

use facet::Facet;

/// A setting's value.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(C)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl Value {
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }

    /// As shown in a row or a phone's list (`On`, `65 %`, `Dark`).
    #[must_use]
    pub fn display(&self, control: &Control) -> String {
        if let Some(choices) = control.choices() {
            if let Some(c) = choices.iter().find(|c| &c.value == self) {
                return c.label.clone();
            }
        }
        let unit = control.unit().map(|u| format!(" {u}")).unwrap_or_default();
        match self {
            Self::Bool(true) => "On".to_owned(),
            Self::Bool(false) => "Off".to_owned(),
            Self::Int(i) => format!("{i}{unit}"),
            Self::Float(f) => format!("{f}{unit}"),
            Self::Text(s) => s.clone(),
        }
    }
}

/// One choice of a select, a segmented control or cards.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Choice {
    pub value: Value,
    pub label: String,
    /// Under the label (cards).
    pub detail: String,
}

impl Choice {
    pub fn new(value: Value, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            detail: String::new(),
        }
    }

    /// A text choice whose value is its id.
    pub fn text(id: &str, label: impl Into<String>) -> Self {
        Self::new(Value::Text(id.to_owned()), label)
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
}

pub use control::Control;

// facet's derive does arithmetic its own way.
#[allow(clippy::arithmetic_side_effects)]
mod control {
    use facet::Facet;

    use super::Choice;

    /// How a setting is changed.
    #[derive(Debug, Clone, PartialEq, Facet)]
    #[repr(C)]
    pub enum Control {
        Switch,
        Select(Vec<Choice>),
        Segmented(Vec<Choice>),
        /// A few visual choices side by side (Light / Dark / System).
        Cards(Vec<Choice>),
        Slider {
            min: f64,
            max: f64,
            step: f64,
            unit: String,
        },
        Stepper {
            min: i64,
            max: i64,
            step: i64,
            unit: String,
        },
        Text {
            placeholder: String,
        },
        /// A key chord, recorded by pressing it (`⇧⌘P`).
        Shortcut,
    }

    impl Control {
        #[must_use]
        pub fn choices(&self) -> Option<&[Choice]> {
            match self {
                Self::Select(c) | Self::Segmented(c) | Self::Cards(c) => Some(c),
                _ => None,
            }
        }

        #[must_use]
        pub fn unit(&self) -> Option<&str> {
            match self {
                Self::Slider { unit, .. } | Self::Stepper { unit, .. } if !unit.is_empty() => {
                    Some(unit)
                }
                _ => None,
            }
        }
    }
}

/// Where a value lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum Scope {
    /// This device.
    #[default]
    Device,
    /// The person's, on every device.
    Synced,
    /// The open project.
    Project,
}

/// Who declared a section: the app, or an extension (whose sections go
/// when it does).
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
#[repr(C)]
pub enum Owner {
    #[default]
    App,
    Extension(String),
}

/// One setting.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Setting {
    /// Unique across the app (`appearance.theme`); an extension's are
    /// prefixed with its id.
    pub key: String,
    pub title: String,
    pub description: String,
    /// Other words search finds it by.
    pub aliases: Vec<String>,
    pub control: Control,
    pub default: Value,
    pub scope: Scope,
}

impl Setting {
    pub fn new(
        key: impl Into<String>,
        title: impl Into<String>,
        control: Control,
        default: Value,
    ) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            description: String::new(),
            aliases: Vec::new(),
            control,
            default,
            scope: Scope::Device,
        }
    }

    #[must_use]
    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    #[must_use]
    pub fn aliases<I: IntoIterator<Item = S>, S: Into<String>>(mut self, words: I) -> Self {
        self.aliases.extend(words.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub const fn scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// A titled run of settings inside a section.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Block {
    pub title: Option<String>,
    pub settings: Vec<Setting>,
}

/// A page of settings: one entry in the settings' list.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Section {
    pub id: String,
    pub title: String,
    /// A lucide icon's name (`palette`), drawn by the page.
    pub icon: String,
    /// The heading it sits under in the list (`Studio`); none for the
    /// first, untitled group.
    pub group: Option<String>,
    pub owner: Owner,
    pub blocks: Vec<Block>,
}

impl Section {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            icon: "settings".to_owned(),
            group: None,
            owner: Owner::App,
            blocks: Vec::new(),
        }
    }

    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
        self
    }

    #[must_use]
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    /// Declared by extension `id`: removed with it.
    #[must_use]
    pub fn owned_by(mut self, id: impl Into<String>) -> Self {
        self.owner = Owner::Extension(id.into());
        self
    }

    /// A block of settings, titled or not.
    #[must_use]
    pub fn block(mut self, title: Option<&str>, settings: Vec<Setting>) -> Self {
        self.blocks.push(Block {
            title: title.map(str::to_owned),
            settings,
        });
        self
    }

    /// Every setting, in order.
    pub fn settings(&self) -> impl Iterator<Item = &Setting> {
        self.blocks.iter().flat_map(|b| b.settings.iter())
    }
}
