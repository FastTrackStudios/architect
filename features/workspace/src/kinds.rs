//! View kinds: what a pane's view draws *with*
//! (`docs/content/architecture/embedded-views.md`).
//!
//! A [`View`](crate::View) says *which* view a pane shows (its
//! [`ViewKind`](crate::ViewKind), `"session.mixer"`); its kind
//! ([`KindId`]) says what draws it: a Dioxus component (the built-in
//! [`DIOXUS`]), a GPU scene, a web view, a Frame surface, an agent's
//! generated UI. Every kind mounts through one contract, [`crate::ViewCtx`]
//! (feature `ctx`), and is declared once in a [`ViewKinds`] registry:
//! where it can be shown ([`Hosts`]), where it can be mounted
//! ([`Placements`]) and what it may do ([`Capabilities`]).
//!
//! Providers come from crates the app links (`frame-view` registers
//! `frame.surface`); architect defines the contract and never depends on
//! them. A kind nobody provided is refused with its reason
//! ([`KindError`]): an extension naming one does not load
//! (`architect-extensions`' `ViewHost::load`), and a pane of one shows the
//! reason in its place.
//!
//! This module is the model: pure data, no Dioxus, wasm-clean.

use std::collections::BTreeMap;
use std::fmt;

/// The built-in kind: a Dioxus component in the app's own document (a
/// workspace [`View`](crate::View)), or a process extension's, drawn
/// remotely in a pane.
pub const DIOXUS: &str = "dioxus";

/// The in-process paint kind: a scene the view records each frame, drawn
/// by Blitz's renderer in the pane's place (`PaintView`, feature `paint`).
///
/// The app registers it ([`KindSpec::paint`]); a GPU view, a Frame
/// surface or a chart draws through it.
pub const PAINT: &str = "paint";

/// The web view kind (#222): an extension's own HTML, CSS and JS in a pane.
///
/// Its package's files, in a native web view clipped to the pane on a desk
/// (wry; `WKWebView` on iOS) and a sandboxed `<iframe>` in a browser. Its code is
/// untrusted: it reaches the app only through the host bridge, every call
/// checked against the extension's capabilities
/// (`architect-extensions`' `webview`). The app registers it
/// ([`KindSpec::web`]) when it links a web view host.
pub const WEB: &str = "web";

/// A kind's id: `"dioxus"`, `"paint"`, `"frame.surface"`, `"genui"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KindId(pub String);

impl KindId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The built-in [`DIOXUS`] kind.
    #[must_use]
    pub fn dioxus() -> Self {
        Self::new(DIOXUS)
    }

    /// The [`PAINT`] kind.
    #[must_use]
    pub fn paint() -> Self {
        Self::new(PAINT)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for KindId {
    fn default() -> Self {
        Self::dioxus()
    }
}

impl fmt::Display for KindId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a view is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ViewHost {
    /// A desk app's window, drawn by Blitz.
    Blitz,
    /// A browser tab: the app built to wasm, drawn in the DOM.
    Web,
    /// A native web view (wry) inside a desk app's pane.
    WebView,
    /// A plugin's editor window (nice-plug), inside another program.
    Plugin,
}

impl ViewHost {
    pub const ALL: [Self; 4] = [Self::Blitz, Self::Web, Self::WebView, Self::Plugin];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Blitz => "blitz",
            Self::Web => "web",
            Self::WebView => "webview",
            Self::Plugin => "plugin",
        }
    }
}

/// The hosts a kind can be shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Hosts(u8);

impl Hosts {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(0b1111);

    const fn bit(host: ViewHost) -> u8 {
        match host {
            ViewHost::Blitz => 1,
            ViewHost::Web => 2,
            ViewHost::WebView => 4,
            ViewHost::Plugin => 8,
        }
    }

    #[must_use]
    pub const fn with(self, host: ViewHost) -> Self {
        Self(self.0 | Self::bit(host))
    }

    #[must_use]
    pub const fn has(self, host: ViewHost) -> bool {
        self.0 & Self::bit(host) != 0
    }

    /// The hosts both can be shown in: a composite's, from its parts.
    #[must_use]
    pub const fn and(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub fn iter(self) -> impl Iterator<Item = ViewHost> {
        ViewHost::ALL.into_iter().filter(move |h| self.has(*h))
    }
}

/// Where a view is mounted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Placement {
    /// A workspace pane: a tab, a dock, a float, a pop-out.
    Pane,
    /// Over another view, in its coordinates (`architect-canvas`).
    Overlay,
    /// Inline in a document or a list (an embed in a note).
    Embed,
    /// A plugin's editor window, through nice-plug.
    PluginWindow,
}

impl Placement {
    pub const ALL: [Self; 4] = [Self::Pane, Self::Overlay, Self::Embed, Self::PluginWindow];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pane => "pane",
            Self::Overlay => "overlay",
            Self::Embed => "embed",
            Self::PluginWindow => "plugin window",
        }
    }
}

/// The placements a kind supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Placements(u8);

impl Placements {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(0b1111);

    const fn bit(placement: Placement) -> u8 {
        match placement {
            Placement::Pane => 1,
            Placement::Overlay => 2,
            Placement::Embed => 4,
            Placement::PluginWindow => 8,
        }
    }

    #[must_use]
    pub const fn with(self, placement: Placement) -> Self {
        Self(self.0 | Self::bit(placement))
    }

    #[must_use]
    pub const fn has(self, placement: Placement) -> bool {
        self.0 & Self::bit(placement) != 0
    }

    #[must_use]
    pub const fn and(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub fn iter(self) -> impl Iterator<Item = Placement> {
        Placement::ALL.into_iter().filter(move |p| self.has(*p))
    }
}

/// What a kind may do: the security model's row for it, read by a host
/// before it mounts one and shown on the extensions page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Capability {
    /// Takes pointer, wheel, touch, keys and focus.
    Input,
    /// Takes composed text (IME).
    Ime,
    /// Draws on the GPU itself (its own scene, a texture).
    Gpu,
    /// Edits a document ([`crate::ViewCtx`]'s `DocRef`), through its
    /// history.
    Document,
    /// Runs actions on the host's Action List (always as the view's
    /// caller, through the list's authorizer and confirmations).
    Actions,
    /// Reaches the network itself.
    Network,
    /// Runs code the app did not ship (a script, HTML, an agent's spec),
    /// inside a sandbox the kind names in its docs row.
    Untrusted,
}

impl Capability {
    pub const ALL: [Self; 7] = [
        Self::Input,
        Self::Ime,
        Self::Gpu,
        Self::Document,
        Self::Actions,
        Self::Network,
        Self::Untrusted,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Ime => "ime",
            Self::Gpu => "gpu",
            Self::Document => "document",
            Self::Actions => "actions",
            Self::Network => "network",
            Self::Untrusted => "untrusted",
        }
    }

    const fn bit(self) -> u8 {
        match self {
            Self::Input => 1,
            Self::Ime => 2,
            Self::Gpu => 4,
            Self::Document => 8,
            Self::Actions => 16,
            Self::Network => 32,
            Self::Untrusted => 64,
        }
    }
}

/// A kind's capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Capabilities(u8);

impl Capabilities {
    pub const NONE: Self = Self(0);

    #[must_use]
    pub const fn with(self, capability: Capability) -> Self {
        Self(self.0 | capability.bit())
    }

    #[must_use]
    pub const fn has(self, capability: Capability) -> bool {
        self.0 & capability.bit() != 0
    }

    /// Everything either may do: a composite's, from its parts.
    #[must_use]
    pub const fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn iter(self) -> impl Iterator<Item = Capability> {
        Capability::ALL.into_iter().filter(move |c| self.has(*c))
    }
}

/// One part of a composite kind: a pane of its own, of a kind of its own,
/// sharing the composite's state per document (the Frame editor's
/// `canvas` part is a GPU kind, its `layers` part a Dioxus one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// Unique in the composite (`"canvas"`, `"layers"`).
    pub id: String,
    pub kind: KindId,
}

/// A kind, as its provider declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindSpec {
    pub id: KindId,
    /// Who provides it: a crate, or `"architect"` for the built-in one.
    pub provider: String,
    pub hosts: Hosts,
    pub placements: Placements,
    pub capabilities: Capabilities,
    /// A composite's parts (empty for any other kind).
    pub parts: Vec<Part>,
}

impl KindSpec {
    /// A kind `id` from `provider`, shown nowhere and mounted nowhere
    /// until it says where.
    pub fn new(id: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            id: KindId::new(id),
            provider: provider.into(),
            hosts: Hosts::NONE,
            placements: Placements::NONE,
            capabilities: Capabilities::NONE,
            parts: Vec::new(),
        }
    }

    /// The built-in [`DIOXUS`] kind: everywhere architect draws (a plugin
    /// window through nice-plug-dioxus), as a pane, an overlay or an
    /// embed; it takes input and runs actions.
    #[must_use]
    pub fn dioxus() -> Self {
        Self::new(DIOXUS, "architect")
            .host(ViewHost::Blitz)
            .host(ViewHost::Web)
            .host(ViewHost::Plugin)
            .placement(Placement::Pane)
            .placement(Placement::Overlay)
            .placement(Placement::Embed)
            .placement(Placement::PluginWindow)
            .capability(Capability::Input)
            .capability(Capability::Ime)
            .capability(Capability::Document)
            .capability(Capability::Actions)
    }

    /// The [`PAINT`] kind: a scene in Blitz's own renderer, as a pane, an
    /// overlay or an embed; it takes input (IME too), draws on the GPU
    /// and runs actions. A web host can register [`Self::paint_web`]
    /// instead; plugin windows do not yet mount this kind.
    #[must_use]
    pub fn paint() -> Self {
        Self::new(PAINT, "architect")
            .host(ViewHost::Blitz)
            .placement(Placement::Pane)
            .placement(Placement::Overlay)
            .placement(Placement::Embed)
            .capability(Capability::Input)
            .capability(Capability::Ime)
            .capability(Capability::Gpu)
            .capability(Capability::Actions)
    }

    /// The [`WEB`] kind: a pane (a tab, a dock, a float, a pop-out) on a
    /// desk's window and in a browser; it takes input in its own
    /// document, runs actions only through its bridge, and is untrusted.
    /// An app that hosts it on one of the two only registers
    /// `KindSpec::new(WEB, …)` with that host.
    #[must_use]
    pub fn web() -> Self {
        Self::new(WEB, "architect")
            .host(ViewHost::Blitz)
            .host(ViewHost::Web)
            .placement(Placement::Pane)
            .capability(Capability::Input)
            .capability(Capability::Ime)
            .capability(Capability::Actions)
            .capability(Capability::Untrusted)
    }

    /// The [`PAINT`] kind for a host that mounts a WebGL2 paint canvas.
    /// The host must forward input and schedule frames itself.
    #[must_use]
    pub fn paint_web() -> Self {
        Self::new(PAINT, "architect")
            .host(ViewHost::Web)
            .placement(Placement::Pane)
            .placement(Placement::Overlay)
            .placement(Placement::Embed)
            .capability(Capability::Input)
            .capability(Capability::Ime)
            .capability(Capability::Gpu)
            .capability(Capability::Actions)
    }

    #[must_use]
    pub const fn host(mut self, host: ViewHost) -> Self {
        self.hosts = self.hosts.with(host);
        self
    }

    #[must_use]
    pub const fn placement(mut self, placement: Placement) -> Self {
        self.placements = self.placements.with(placement);
        self
    }

    #[must_use]
    pub const fn capability(mut self, capability: Capability) -> Self {
        self.capabilities = self.capabilities.with(capability);
        self
    }

    /// Adds part `id` of `kind`: this kind becomes a composite, shown
    /// where all of its parts can be and able to do what any of them may
    /// ([`ViewKinds::register`] works that out).
    #[must_use]
    pub fn part(mut self, id: impl Into<String>, kind: impl Into<String>) -> Self {
        self.parts.push(Part {
            id: id.into(),
            kind: KindId::new(kind),
        });
        self
    }

    #[must_use]
    pub const fn is_composite(&self) -> bool {
        !self.parts.is_empty()
    }
}

/// Why a kind was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KindError {
    /// Nobody in this app provides it.
    #[error("no provider for the view kind “{kind}” in this app (it has: {})", known.join(", "))]
    Unknown { kind: KindId, known: Vec<String> },
    /// It is provided, but not for this host.
    #[error("the view kind “{kind}” is not shown in a {} host (only: {})", host.name(), names(hosts.iter().map(ViewHost::name)))]
    Host {
        kind: KindId,
        host: ViewHost,
        hosts: Hosts,
    },
    /// It is provided, but cannot be mounted there.
    #[error("the view kind “{kind}” is not mounted as {} (only: {})", placement.name(), names(placements.iter().map(Placement::name)))]
    Placement {
        kind: KindId,
        placement: Placement,
        placements: Placements,
    },
    /// Another provider registered it first.
    #[error("the view kind “{kind}” is already provided by {provider}")]
    Taken { kind: KindId, provider: String },
    /// A composite's part is of a kind not registered (yet), is itself a
    /// composite, or shares another part's id.
    #[error("composite view kind “{kind}”, part “{part}”: {why}")]
    Part {
        kind: KindId,
        part: String,
        why: String,
    },
}

fn names<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let all: Vec<&str> = names.collect();
    if all.is_empty() {
        "nowhere".to_owned()
    } else {
        all.join(", ")
    }
}

/// The view kinds an app provides: the built-in [`DIOXUS`] one and
/// whatever the crates it links registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewKinds {
    kinds: BTreeMap<KindId, KindSpec>,
}

impl Default for ViewKinds {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewKinds {
    /// The built-in [`DIOXUS`] kind alone.
    #[must_use]
    pub fn new() -> Self {
        let mut kinds = BTreeMap::new();
        let dioxus = KindSpec::dioxus();
        kinds.insert(dioxus.id.clone(), dioxus);
        Self { kinds }
    }

    /// No kinds at all, not even [`DIOXUS`]: a host that draws none of
    /// its own (a headless one, a test).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            kinds: BTreeMap::new(),
        }
    }

    /// Registers `spec`. A composite's hosts and placements become those
    /// all its parts share (within what it declared, when it declared
    /// any), its capabilities what any part has.
    ///
    /// # Errors
    ///
    /// The id is taken, or a part is unknown, a composite, or named twice.
    pub fn register(&mut self, spec: KindSpec) -> Result<(), KindError> {
        if let Some(have) = self.kinds.get(&spec.id) {
            return Err(KindError::Taken {
                kind: spec.id,
                provider: have.provider.clone(),
            });
        }
        let spec = self.composed(spec)?;
        self.kinds.insert(spec.id.clone(), spec);
        Ok(())
    }

    /// `spec` with a composite's reach worked out from its parts.
    fn composed(&self, mut spec: KindSpec) -> Result<KindSpec, KindError> {
        if !spec.is_composite() {
            return Ok(spec);
        }
        let mut hosts = if spec.hosts == Hosts::NONE {
            Hosts::ALL
        } else {
            spec.hosts
        };
        let mut placements = if spec.placements == Placements::NONE {
            Placements::ALL
        } else {
            spec.placements
        };
        let mut capabilities = spec.capabilities;
        let mut seen = std::collections::BTreeSet::new();
        for part in &spec.parts {
            let refuse = |why: &str| KindError::Part {
                kind: spec.id.clone(),
                part: part.id.clone(),
                why: why.to_owned(),
            };
            if !seen.insert(part.id.as_str()) {
                return Err(refuse("named twice"));
            }
            let Some(of) = self.kinds.get(&part.kind) else {
                return Err(refuse(&format!(
                    "its kind “{}” is not registered (register it first)",
                    part.kind
                )));
            };
            if of.is_composite() {
                return Err(refuse("a part cannot itself be a composite"));
            }
            hosts = hosts.and(of.hosts);
            placements = placements.and(of.placements);
            capabilities = capabilities.or(of.capabilities);
        }
        spec.hosts = hosts;
        spec.placements = placements;
        spec.capabilities = capabilities;
        Ok(spec)
    }

    /// Kind `id`, as registered.
    ///
    /// # Errors
    ///
    /// [`KindError::Unknown`], with the kinds there are.
    pub fn resolve(&self, id: &str) -> Result<&KindSpec, KindError> {
        self.kinds
            .get(&KindId::new(id))
            .ok_or_else(|| KindError::Unknown {
                kind: KindId::new(id),
                known: self.ids(),
            })
    }

    /// Kind `id`, if it can be shown in `host` mounted as `placement`.
    ///
    /// # Errors
    ///
    /// It is unknown, or not for that host or placement.
    pub fn resolve_for(
        &self,
        id: &str,
        host: ViewHost,
        placement: Placement,
    ) -> Result<&KindSpec, KindError> {
        let spec = self.resolve(id)?;
        if !spec.hosts.has(host) {
            return Err(KindError::Host {
                kind: spec.id.clone(),
                host,
                hosts: spec.hosts,
            });
        }
        if !spec.placements.has(placement) {
            return Err(KindError::Placement {
                kind: spec.id.clone(),
                placement,
                placements: spec.placements,
            });
        }
        Ok(spec)
    }

    /// Whether kind `id` is registered.
    #[must_use]
    pub fn has(&self, id: &str) -> bool {
        self.kinds.contains_key(&KindId::new(id))
    }

    /// Every kind's id, sorted.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        self.kinds.keys().map(|k| k.0.clone()).collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &KindSpec> {
        self.kinds.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dioxus_is_built_in() {
        let kinds = ViewKinds::new();
        let d = kinds.resolve(DIOXUS).expect("dioxus");
        assert_eq!(d.provider, "architect");
        assert!(d.hosts.has(ViewHost::Blitz) && d.hosts.has(ViewHost::Web));
        assert!(!d.hosts.has(ViewHost::WebView));
        assert!(
            kinds
                .resolve_for(DIOXUS, ViewHost::Plugin, Placement::PluginWindow)
                .is_ok()
        );
        assert!(ViewKinds::empty().ids().is_empty());
    }

    #[test]
    fn the_web_kind_is_an_untrusted_pane_on_a_desk_and_in_a_browser() {
        let mut kinds = ViewKinds::new();
        kinds.register(KindSpec::web()).expect("register");
        let w = kinds
            .resolve_for(WEB, ViewHost::Blitz, Placement::Pane)
            .expect("a desk pane");
        assert!(w.capabilities.has(Capability::Untrusted));
        assert!(!w.capabilities.has(Capability::Document));
        assert!(
            kinds
                .resolve_for(WEB, ViewHost::Web, Placement::Pane)
                .is_ok()
        );
        // Never inline in a note, and never in a plugin's window.
        assert!(
            kinds
                .resolve_for(WEB, ViewHost::Blitz, Placement::Embed)
                .is_err()
        );
        assert!(
            kinds
                .resolve_for(WEB, ViewHost::Plugin, Placement::Pane)
                .is_err()
        );
    }

    #[test]
    fn paint_is_the_gpu_kind_an_app_registers() {
        let mut kinds = ViewKinds::new();
        assert!(!kinds.has(PAINT));
        kinds.register(KindSpec::paint()).expect("register");
        let p = kinds
            .resolve_for(PAINT, ViewHost::Blitz, Placement::Pane)
            .expect("paint in a pane");
        assert!(p.capabilities.has(Capability::Gpu) && p.capabilities.has(Capability::Ime));
        assert!(!p.capabilities.has(Capability::Document));
        assert!(
            kinds
                .resolve_for(PAINT, ViewHost::Web, Placement::Pane)
                .is_err()
        );
        assert_eq!(KindId::paint().as_str(), PAINT);
    }

    #[test]
    fn web_paint_is_an_explicit_host_registration() {
        let mut kinds = ViewKinds::new();
        kinds.register(KindSpec::paint_web()).expect("register");
        let spec = kinds
            .resolve_for(PAINT, ViewHost::Web, Placement::Pane)
            .expect("web canvas");
        assert!(spec.capabilities.has(Capability::Gpu));
        assert!(
            kinds
                .resolve_for(PAINT, ViewHost::Plugin, Placement::PluginWindow)
                .is_err()
        );
    }

    #[test]
    fn an_unknown_kind_says_why() {
        let err = ViewKinds::new().resolve("frame.surface").unwrap_err();
        assert_eq!(
            err.to_string(),
            "no provider for the view kind “frame.surface” in this app (it has: dioxus)"
        );
    }

    #[test]
    fn a_kind_refuses_hosts_and_placements_it_lacks() {
        let mut kinds = ViewKinds::new();
        kinds
            .register(
                KindSpec::new("webview", "test")
                    .host(ViewHost::Blitz)
                    .placement(Placement::Pane)
                    .capability(Capability::Untrusted),
            )
            .expect("register");
        let host = kinds
            .resolve_for("webview", ViewHost::Plugin, Placement::Pane)
            .unwrap_err();
        assert_eq!(
            host.to_string(),
            "the view kind “webview” is not shown in a plugin host (only: blitz)"
        );
        let place = kinds
            .resolve_for("webview", ViewHost::Blitz, Placement::Embed)
            .unwrap_err();
        assert!(matches!(place, KindError::Placement { .. }), "{place}");
        let taken = kinds
            .register(KindSpec::new("webview", "other"))
            .unwrap_err();
        assert!(matches!(taken, KindError::Taken { ref provider, .. } if provider == "test"));
    }

    #[test]
    fn a_composite_shares_its_parts_reach() {
        let mut kinds = ViewKinds::new();
        kinds
            .register(
                KindSpec::new("gpu", "test")
                    .host(ViewHost::Blitz)
                    .host(ViewHost::Web)
                    .host(ViewHost::Plugin)
                    .placement(Placement::Pane)
                    .capability(Capability::Gpu),
            )
            .expect("gpu");
        kinds
            .register(
                KindSpec::new("frame.editor", "frame-editor")
                    .part("canvas", "gpu")
                    .part("layers", DIOXUS),
            )
            .expect("composite");
        let editor = kinds.resolve("frame.editor").expect("editor");
        assert_eq!(
            editor.hosts.iter().collect::<Vec<_>>(),
            [ViewHost::Blitz, ViewHost::Web, ViewHost::Plugin]
        );
        assert_eq!(
            editor.placements.iter().collect::<Vec<_>>(),
            [Placement::Pane]
        );
        assert!(editor.capabilities.has(Capability::Gpu));
        assert!(editor.capabilities.has(Capability::Actions));

        let missing = kinds
            .register(KindSpec::new("x", "t").part("a", "nope"))
            .unwrap_err();
        assert!(matches!(missing, KindError::Part { .. }), "{missing}");
        let nested = kinds
            .register(KindSpec::new("y", "t").part("a", "frame.editor"))
            .unwrap_err();
        assert!(nested.to_string().contains("cannot itself be a composite"));
        let twice = kinds
            .register(KindSpec::new("z", "t").part("a", DIOXUS).part("a", "gpu"))
            .unwrap_err();
        assert!(twice.to_string().contains("named twice"));
    }
}
