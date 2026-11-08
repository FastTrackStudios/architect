//! Tab kinds: the tabs an app or an extension adds to the tab bar, and how
//! each draws.
//!
//! A [`TabKind`] is registered by the app (Rust, [`TabKinds::register`])
//! or an extension (`[[tab_kinds]]`). It is one of two things:
//!
//! - a **scope tab** kind ([`TabKind::scope`]): a tab of it is a deck
//!   ([`crate::Deck`]) on a context of the kind's type (a project, a
//!   song). Scope tabs sit in a group at the tab bar's leading edge, then a
//!   divider, then the tabs of the scope on show; activating one swaps the
//!   rest of the workspace to that scope's remembered state;
//! - a plain **custom tab** kind: panes of the kind's views
//!   ([`TabKind::views`]) draw their tab its way, inline with the others.
//!
//! What a tab of a kind shows is its [`TabFace`]: a title, a subtitle
//! made from its context's properties ([`TabKind::subtitle`], "G · 96
//! bpm"), an icon, a colour, a width and a badge. The `ui` feature draws a
//! face, or hands it to the kind's own renderer (`TabRenderers`). Pure
//! data: no Dioxus.

use facet::Facet;

use crate::deck::DeckContext;

/// How wide a tab of a kind is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum TabWidth {
    /// An icon and a short title (Architect's projects).
    #[default]
    Compact,
    /// A title over a subtitle (Studio's songs: key and tempo).
    Wide,
}

impl TabWidth {
    /// From a manifest's word (`compact`, `wide`); anything else compact.
    #[must_use]
    pub const fn parse(word: &str) -> Self {
        if word.eq_ignore_ascii_case("wide") {
            Self::Wide
        } else {
            Self::Compact
        }
    }

    /// As a manifest writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Wide => "wide",
        }
    }
}

/// One of a kind's own rows in its tab's menu: runs `action` with
/// `{context}` (a scope tab's) or `{pane}` (a custom tab's).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct TabKindItem {
    pub label: String,
    /// An Action List id.
    pub action: String,
}

/// A kind of tab, as the app or an extension registers it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct TabKind {
    /// Its id. A scope kind's is the context type its tabs are on
    /// (`project`, `song`).
    pub id: String,
    /// What a person calls one ("Project", "Song").
    pub title: String,
    /// A tab of it is a scope tab (a deck): activating it swaps the rest
    /// of the workspace to its own.
    #[facet(default)]
    pub scope: bool,
    #[facet(default)]
    pub width: TabWidth,
    /// A Lucide icon's name, when its context gives none.
    #[facet(default)]
    pub icon: Option<String>,
    /// A CSS colour, when its context gives none.
    #[facet(default)]
    pub colour: Option<String>,
    /// The subtitle, from its context's properties: `{key}` is property
    /// `key`'s value. Parts between ` · ` whose properties are missing
    /// drop out (`"{key} · {tempo} bpm"` is `G · 96 bpm`, or `96 bpm`).
    #[facet(default)]
    pub subtitle: Option<String>,
    /// A custom kind's views (`ext/view`): their panes' tabs draw this
    /// kind's way.
    #[facet(default)]
    pub views: Vec<String>,
    /// Its own rows in its tabs' menu, after the bar's own.
    #[facet(default)]
    pub menu: Vec<TabKindItem>,
    /// Who registered it: `app`, or an extension's id.
    #[facet(default)]
    pub by: String,
}

impl TabKind {
    /// A custom (non-scope) kind called `title`.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            scope: false,
            width: TabWidth::Compact,
            icon: None,
            colour: None,
            subtitle: None,
            views: Vec::new(),
            menu: Vec::new(),
            by: "app".to_owned(),
        }
    }

    /// A scope kind over contexts of type `id`.
    pub fn scope(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            scope: true,
            ..Self::new(id, title)
        }
    }

    #[must_use]
    pub const fn width(mut self, width: TabWidth) -> Self {
        self.width = width;
        self
    }

    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    #[must_use]
    pub fn colour(mut self, colour: impl Into<String>) -> Self {
        self.colour = Some(colour.into());
        self
    }

    #[must_use]
    pub fn subtitle(mut self, template: impl Into<String>) -> Self {
        self.subtitle = Some(template.into());
        self
    }

    /// Its tabs are `view`'s panes (a custom kind).
    #[must_use]
    pub fn view(mut self, view: impl Into<String>) -> Self {
        self.views.push(view.into());
        self
    }

    #[must_use]
    pub fn item(mut self, label: impl Into<String>, action: impl Into<String>) -> Self {
        self.menu.push(TabKindItem {
            label: label.into(),
            action: action.into(),
        });
        self
    }

    #[must_use]
    pub fn by(mut self, by: impl Into<String>) -> Self {
        self.by = by.into();
        self
    }

    /// What a scope tab on `context` shows: its title, its subtitle from
    /// its properties, its icon and colour (its own, else the kind's), and
    /// `badge`.
    #[must_use]
    pub fn face(&self, context: &DeckContext, badge: Option<String>) -> TabFace {
        let props: Vec<(&str, &str)> = context
            .properties
            .iter()
            .map(|p| (p.key.as_str(), p.value.as_str()))
            .collect();
        TabFace {
            kind: self.id.clone(),
            title: context.title.clone(),
            subtitle: self.subtitle.as_deref().and_then(|t| fill(t, &props)),
            icon: context.icon.clone().or_else(|| self.icon.clone()),
            colour: context.colour.clone().or_else(|| self.colour.clone()),
            width: self.width,
            badge,
        }
    }

    /// What a custom tab called `title` shows.
    #[must_use]
    pub fn face_titled(&self, title: &str, badge: Option<String>) -> TabFace {
        TabFace {
            kind: self.id.clone(),
            title: title.to_owned(),
            subtitle: self.subtitle.as_deref().and_then(|t| fill(t, &[])),
            icon: self.icon.clone(),
            colour: self.colour.clone(),
            width: self.width,
            badge,
        }
    }
}

/// What one tab of a kind shows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct TabFace {
    /// The kind's id.
    pub kind: String,
    pub title: String,
    #[facet(default)]
    pub subtitle: Option<String>,
    #[facet(default)]
    pub icon: Option<String>,
    #[facet(default)]
    pub colour: Option<String>,
    #[facet(default)]
    pub width: TabWidth,
    #[facet(default)]
    pub badge: Option<String>,
}

impl TabFace {
    /// A scope tab on a context of a type no kind is registered for:
    /// compact, its context's title, icon and colour.
    #[must_use]
    pub fn plain(context: &DeckContext, badge: Option<String>) -> Self {
        Self {
            kind: context.kind.clone(),
            title: context.title.clone(),
            subtitle: None,
            icon: context.icon.clone(),
            colour: context.colour.clone(),
            width: TabWidth::Compact,
            badge,
        }
    }
}

/// `template` with each `{key}` its property's value; a ` · ` part whose
/// properties are not all there drops out. None when nothing is left.
#[must_use]
pub fn fill(template: &str, props: &[(&str, &str)]) -> Option<String> {
    let parts: Vec<String> = template
        .split(" · ")
        .filter_map(|part| fill_part(part, props))
        .filter(|p| !p.trim().is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn fill_part(part: &str, props: &[(&str, &str)]) -> Option<String> {
    let mut out = String::new();
    let mut rest = part;
    while let Some(open) = rest.find('{') {
        let (before, after) = rest.split_at(open);
        out.push_str(before);
        let close = after.find('}')?;
        let key = after.get(1..close)?.trim();
        let value = props
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.trim())
            .filter(|v| !v.is_empty())?;
        out.push_str(value);
        rest = after.get(close.saturating_add(1)..).unwrap_or("");
    }
    out.push_str(rest);
    Some(out)
}

/// Every tab kind an app has: its own and its extensions'. The first
/// registered for an id wins.
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct TabKinds {
    pub kinds: Vec<TabKind>,
}

impl TabKinds {
    #[must_use]
    pub const fn new() -> Self {
        Self { kinds: Vec::new() }
    }

    /// Adds `kind`; false (and nothing changes) when its id is taken.
    pub fn register(&mut self, kind: TabKind) -> bool {
        if self.get(&kind.id).is_some() {
            return false;
        }
        self.kinds.push(kind);
        true
    }

    #[must_use]
    pub fn with(mut self, kind: TabKind) -> Self {
        self.register(kind);
        self
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&TabKind> {
        self.kinds.iter().find(|k| k.id == id)
    }

    /// The scope kind over contexts of type `kind`.
    #[must_use]
    pub fn scope(&self, kind: &str) -> Option<&TabKind> {
        self.get(kind).filter(|k| k.scope)
    }

    /// The custom kind `view`'s panes are tabs of.
    #[must_use]
    pub fn for_view(&self, view: &str) -> Option<&TabKind> {
        self.kinds
            .iter()
            .find(|k| !k.scope && k.views.iter().any(|v| v == view))
    }

    /// What scope tab `context` shows: its kind's face, else a plain one.
    #[must_use]
    pub fn face(&self, context: &DeckContext, badge: Option<String>) -> TabFace {
        match self.scope(&context.kind) {
            Some(k) => k.face(context, badge),
            None => TabFace::plain(context, badge),
        }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}
