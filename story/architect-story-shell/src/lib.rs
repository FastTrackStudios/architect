//! Interactive Lookbook-style shell.
//!
//! Renderer-agnostic Dioxus component. Drop into any Dioxus 0.7 app
//! (web/desktop/native/mobile) and it renders:
//!
//! - **Top bar** — branding plus a `header_extras` slot the host fills
//!   with theme controls, density toggle, etc.
//! - **Sidebar** — every story registered via
//!   [`STORIES`](architect_story_runtime::STORIES), grouped by `category`.
//! - **Preview pane** — the selected story rendered with the current
//!   knob values (and inheriting whatever theme the host provides).
//! - **Knob editor** — typed inputs above the preview, one per knob.
//!
//! ## Styling
//!
//! The shell does **not** ship its own colour palette. Every surface
//! uses the standard shadcn-style Tailwind tokens — `bg-background`,
//! `bg-card`, `text-foreground`, `text-muted-foreground`, `border`,
//! `border-border`, `bg-accent`, etc. — so when the host wraps the
//! Lookbook in a `ThemeProvider`, switching the preset or mode in the
//! top bar reskins the shell *and* every preview at once.
//!
//! Consumers must therefore have a Tailwind config / generated CSS that
//! defines those tokens. architect-ui ships that out of the box; any other
//! consumer (frame-ui, etc.) just needs the same `@theme` block in
//! their tailwind input file.

use std::collections::HashMap;

use dioxus::prelude::*;
use dioxus_router::{
    components::{Link, Outlet, Router},
    hooks::{use_navigator, use_route},
};
use architect_story_runtime::{render_fn, KnobKind, KnobSource, KnobValue, Story, STORIES};

// ── Routes ───────────────────────────────────────────────────────────────────

/// URL-shape:
/// - `/` → empty state, prompts the user to pick a component.
/// - `/c/Button` → renders all stories under category "Button".
///
/// On web the URL bar is shareable / refreshable; on desktop / native
/// the router uses an in-memory history so back / forward keys work
/// even though there is no address bar.
#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Chrome)]
        #[route("/")]
        Home {},
        #[route("/c/:category")]
        ComponentRoute { category: String },
}

// ── LookbookContext ─────────────────────────────────────────────────────────

/// Shared state lifted into context so router-rendered layout / route
/// components can read it without prop drilling.
#[derive(Clone)]
struct LookbookContext {
    header_extras: Option<Element>,
    /// Stories grouped by category, in sidebar render order.
    groups: Vec<(&'static str, Vec<&'static Story>)>,
    /// Resolved initial category from `initial_story` — Chrome
    /// navigates here on first mount.
    initial_category: Option<String>,
}

impl PartialEq for LookbookContext {
    fn eq(&self, _other: &Self) -> bool {
        // Stories are `&'static`, so identity comparison would suffice;
        // but we never re-create the context, so equality is moot. We
        // implement it manually because `Element` doesn't impl `Eq`.
        true
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct LookbookProps {
    /// Optional content rendered into the sticky top bar — typically a
    /// theme picker, density toggle, or other globally-scoped control.
    /// Anything dispatched from here should propagate through Dioxus
    /// context so the previews below pick it up.
    #[props(default)]
    pub header_extras: Option<Element>,
    /// Optional starting story by name. Useful for snapshot harnesses
    /// that boot the binary into a specific preview. With chrome this
    /// determines the initial route.
    #[props(default)]
    pub initial_story: Option<String>,
    /// When `false`, render only the preview (no sidebar / top bar /
    /// header / knob editor). Used by the parity harness so the
    /// captured pixels are just the component, not the chrome.
    #[props(default = true)]
    pub chrome: bool,
}

#[component]
pub fn Lookbook(props: LookbookProps) -> Element {
    let stories: Vec<&'static Story> = {
        let mut v: Vec<&'static Story> = STORIES.iter().copied().collect();
        v.sort_by_key(|s| (s.category.unwrap_or("zzz"), s.name));
        v
    };

    let groups: Vec<(&'static str, Vec<&'static Story>)> = {
        let mut g: Vec<(&'static str, Vec<&'static Story>)> = Vec::new();
        for story in &stories {
            let cat = story.category.unwrap_or("Uncategorised");
            match g.last_mut() {
                Some((c, list)) if *c == cat => list.push(*story),
                _ => g.push((cat, vec![*story])),
            }
        }
        g
    };

    if !props.chrome {
        // Headless / snapshot mode: skip the router entirely. The
        // parity harness selects a specific story by name; we render
        // it directly so the captured pixels are deterministic.
        let snapshot_story = props
            .initial_story
            .as_deref()
            .and_then(|h| stories.iter().find(|s| s.name == h).copied())
            .or_else(|| stories.first().copied());
        return rsx! {
            div { class: "min-h-screen bg-background text-foreground p-6",
                "data-fts-story-name": snapshot_story.map(|s| s.name).unwrap_or_default(),
                if let Some(story) = snapshot_story {
                    ChromelessPreview { story }
                }
            }
        };
    }

    // Resolve the initial route from `initial_story` (which may be a
    // category name or a legacy story name).
    let initial_category = props.initial_story.as_deref().and_then(|h| {
        groups
            .iter()
            .find(|(c, _)| *c == h)
            .map(|(c, _)| (*c).to_string())
            .or_else(|| {
                stories
                    .iter()
                    .find(|s| s.name == h)
                    .and_then(|s| s.category.map(|c| c.to_string()))
            })
    });

    use_context_provider(|| LookbookContext {
        header_extras: props.header_extras.clone(),
        groups: groups.clone(),
        initial_category,
    });

    rsx! {
        Router::<Route> {}
    }
}

/// Layout component wrapping every route — top bar, sidebar (with
/// search filter), and an `Outlet` for the active route's content.
#[component]
fn Chrome() -> Element {
    let ctx: LookbookContext = use_context();
    let groups = ctx.groups.clone();
    let total_stories: usize = groups.iter().map(|(_, s)| s.len()).sum();
    let mut search = use_signal(String::new);

    // Apply the host-provided `initial_story` once, after Router is
    // mounted (the navigator isn't available before that).
    let initial = ctx.initial_category.clone();
    let nav = use_navigator();
    let mut applied_initial = use_signal(|| false);
    use_effect(move || {
        if !applied_initial() {
            if let Some(cat) = initial.clone() {
                nav.replace(Route::ComponentRoute { category: cat });
            }
            applied_initial.set(true);
        }
    });

    rsx! {
        div { class: "min-h-screen bg-background text-foreground",
            header { class: "sticky top-0 z-20 flex items-center gap-4 h-12 px-4 border-b border-border bg-card text-card-foreground",
                div { class: "flex items-baseline gap-3",
                    span { class: "text-xs font-semibold uppercase tracking-wider", "architect-story" }
                    span { class: "text-[11px] text-muted-foreground",
                        "{groups.len()} components · {total_stories} stories"
                    }
                }
                if let Some(extras) = ctx.header_extras.clone() {
                    div { class: "ml-auto flex items-center gap-2",
                        {extras}
                    }
                }
            }
            div { class: "grid grid-cols-[240px_1fr] min-h-[calc(100vh-3rem)]",
                aside { class: "sticky top-12 self-start max-h-[calc(100vh-3rem)] w-60 overflow-y-auto border-r border-border bg-card text-card-foreground flex flex-col",
                    div { class: "px-3 py-2 border-b border-border",
                        input {
                            r#type: "text",
                            class: "w-full h-7 bg-input/30 border border-border rounded-md px-2 text-xs text-foreground outline-none focus:border-ring placeholder:text-muted-foreground",
                            placeholder: "Search components…",
                            value: "{search.read()}",
                            oninput: move |e| search.set(e.value()),
                        }
                    }
                    SidebarTree { groups: groups.clone(), search }
                }
                main { class: "p-6 overflow-y-auto",
                    Outlet::<Route> {}
                }
            }
        }
    }
}

#[component]
fn Home() -> Element {
    rsx! { EmptyState {} }
}

#[component]
fn ComponentRoute(category: String) -> Element {
    let ctx: LookbookContext = use_context();
    let stories: Vec<&'static Story> = ctx
        .groups
        .iter()
        .find(|(c, _)| *c == category)
        .map(|(_, s)| s.clone())
        .unwrap_or_default();

    if stories.is_empty() {
        return rsx! {
            div { class: "max-w-md text-muted-foreground text-sm",
                h2 { class: "text-foreground text-base font-semibold mb-2", "Unknown component" }
                p {
                    "No stories registered under category "
                    code { class: "rounded bg-accent/40 px-1 py-0.5 text-[12px]", "\"{category}\"" }
                    "."
                }
            }
        };
    }

    // Leak the category to get a `&'static str` for the inner views.
    // Categories come from STORIES which already use `&'static str`,
    // so the leak only happens when the URL parameter is mistyped.
    let category_static: &'static str = ctx
        .groups
        .iter()
        .find(|(c, _)| *c == category)
        .map(|(c, _)| *c)
        .unwrap_or_else(|| Box::leak(category.into_boxed_str()));

    rsx! {
        ComponentView { category: category_static, stories }
    }
}

#[component]
fn SidebarTree(
    groups: Vec<(&'static str, Vec<&'static Story>)>,
    search: Signal<String>,
) -> Element {
    let route: Route = use_route();
    let active_category = match &route {
        Route::ComponentRoute { category } => Some(category.clone()),
        Route::Home {} => None,
    };

    // Filter sidebar by typed query — case-insensitive substring on
    // the category name. Empty query shows everything.
    let q = search.read().to_lowercase();
    let filtered: Vec<&(&'static str, Vec<&'static Story>)> = groups
        .iter()
        .filter(|(c, _)| q.is_empty() || c.to_lowercase().contains(&q))
        .collect();

    rsx! {
        nav { class: "flex flex-col py-2",
            if filtered.is_empty() {
                div { class: "px-4 py-3 text-[11px] text-muted-foreground",
                    "No matches."
                }
            }
            ul { class: "flex flex-col",
                for (category, items) in filtered.iter() {
                    {
                        let cat = *category;
                        let active = active_category.as_deref() == Some(cat);
                        let count = items.len();
                        let class = if active {
                            "flex w-full items-center justify-between gap-2 text-left px-4 py-1.5 text-sm border-l-2 border-primary bg-accent/40 text-foreground font-medium"
                        } else {
                            "flex w-full items-center justify-between gap-2 text-left px-4 py-1.5 text-sm border-l-2 border-transparent hover:bg-accent/20 text-muted-foreground hover:text-foreground"
                        };
                        let route = Route::ComponentRoute { category: cat.to_string() };
                        rsx! {
                            li { key: "{cat}",
                                Link {
                                    to: route,
                                    class,
                                    span { "{cat}" }
                                    span { class: "text-[11px] text-muted-foreground tabular-nums",
                                        "{count}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Renders every story under one category as a stack of preview
/// sections. The sidebar selects the category; this view scrolls
/// through each story belonging to it.
#[component]
fn ComponentView(category: &'static str, stories: Vec<&'static Story>) -> Element {
    rsx! {
        div { key: "{category}", class: "flex flex-col gap-6 max-w-5xl",
            header { class: "pb-3 border-b border-border",
                div { class: "text-xs uppercase tracking-wider text-muted-foreground",
                    "Component"
                }
                h1 { class: "text-2xl font-semibold tracking-tight", "{category}" }
                p { class: "mt-1 text-sm text-muted-foreground",
                    "{stories.len()} stories"
                }
            }
            for story in stories {
                // Stable per-story key on the loop item itself —
                // without this, Dioxus reuses StoryView scopes by
                // position, so hook indices from one story leak into
                // another (panic: "Unable to retrieve the hook that
                // was initialized at this index"). Story render-fns
                // call hooks inline, so the count varies per story.
                StoryView {
                    key: "{story.category.unwrap_or(\"\")}-{story.name}",
                    story,
                }
            }
        }
    }
}

/// Chromeless preview used by the parity / snapshot harness — renders
/// the story with its declared default knobs, no editor or header.
#[component]
fn ChromelessPreview(story: &'static Story) -> Element {
    let initial: HashMap<&'static str, KnobValue> = story
        .knobs
        .iter()
        .filter_map(|spec| spec.default.as_ref().map(|v| (spec.name, clone_knob(v))))
        .collect();
    let knob_state = use_signal(|| initial);
    rsx! {
        StoryPreview { story, knob_state }
    }
}

#[component]
fn StoryView(story: &'static Story) -> Element {
    let initial: HashMap<&'static str, KnobValue> = story
        .knobs
        .iter()
        .filter_map(|spec| spec.default.as_ref().map(|v| (spec.name, clone_knob(v))))
        .collect();
    let knob_state = use_signal(|| initial);
    let anchor = format!(
        "{}-{}",
        story.category.unwrap_or("uncategorised"),
        story.name
    );

    rsx! {
        section { key: "{anchor}", id: "{anchor}", class: "flex flex-col gap-3",
            StoryHeader { story }
            if !story.knobs.is_empty() {
                KnobEditor { story, knob_state }
            }
            div { class: "rounded-lg border border-border bg-card text-card-foreground p-6 min-h-[14rem]",
                // Key on StoryPreview ensures Dioxus mounts a fresh
                // scope per story. Story render-fns call hooks
                // (`use_signal`, etc.) inline; without a stable key,
                // navigating between stories with different hook counts
                // panics with "Unable to retrieve the hook that was
                // initialized at this index".
                StoryPreview { key: "{anchor}", story, knob_state }
            }
        }
    }
}

#[component]
fn StoryHeader(story: &'static Story) -> Element {
    rsx! {
        header { class: "flex flex-col gap-1",
            h2 { class: "text-base font-semibold tracking-tight text-foreground",
                "{story.name}"
            }
            if !story.description.is_empty() {
                p { class: "text-sm text-muted-foreground", "{story.description}" }
            }
            if !story.source.is_empty() {
                p { class: "text-[11px] font-mono text-muted-foreground",
                    "{story.source}"
                }
            }
        }
    }
}

#[component]
fn StoryPreview(
    story: &'static Story,
    knob_state: Signal<HashMap<&'static str, KnobValue>>,
) -> Element {
    // SAFETY: `story.render` is a `RenderFn` produced via the
    // `const_story` builder or the `#[story]` proc-macro.
    let render = unsafe { render_fn(story) };
    let snapshot = knob_state.read().clone();
    render(&MapKnobs(&snapshot))
}

#[component]
fn KnobEditor(
    story: &'static Story,
    knob_state: Signal<HashMap<&'static str, KnobValue>>,
) -> Element {
    rsx! {
        section { class: "rounded-lg border border-border bg-card text-card-foreground p-4 grid gap-3 grid-cols-[repeat(auto-fill,minmax(13rem,1fr))]",
            for spec in story.knobs.iter() {
                {
                    let name = spec.name;
                    let doc = spec.doc;
                    rsx! {
                        div { class: "flex flex-col gap-1",
                            label { class: "text-[11px] font-semibold uppercase tracking-wider text-muted-foreground",
                                "{name}"
                            }
                            if !doc.is_empty() {
                                p { class: "text-[11px] text-muted-foreground", "{doc}" }
                            }
                            KnobControl { spec, knob_state }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn KnobControl(
    spec: &'static architect_story_runtime::KnobSpec,
    knob_state: Signal<HashMap<&'static str, KnobValue>>,
) -> Element {
    let name = spec.name;
    let current = knob_state
        .read()
        .get(name)
        .or(spec.default.as_ref())
        .map(clone_knob);

    let input_class = "h-8 rounded border border-input bg-background px-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

    match &spec.kind {
        KnobKind::Bool => {
            let checked = matches!(current, Some(KnobValue::Bool(true)));
            rsx! {
                input {
                    r#type: "checkbox",
                    class: "size-4 accent-primary",
                    checked,
                    onchange: move |e| {
                        let v = e.value() == "true";
                        knob_state.write().insert(name, KnobValue::Bool(v));
                    },
                }
            }
        }
        KnobKind::Number { .. } => {
            let value_str = match &current {
                Some(KnobValue::Int(v)) => v.to_string(),
                Some(KnobValue::Float(v)) => v.to_string(),
                _ => String::new(),
            };
            rsx! {
                input {
                    r#type: "number",
                    class: input_class,
                    value: "{value_str}",
                    oninput: move |e| {
                        let raw = e.value();
                        let value = if raw.contains('.') {
                            raw.parse::<f64>().ok().map(KnobValue::Float)
                        } else {
                            raw.parse::<i64>().ok().map(KnobValue::Int)
                        };
                        if let Some(v) = value {
                            knob_state.write().insert(name, v);
                        }
                    },
                }
            }
        }
        KnobKind::String { multiline } => {
            let value_str = match &current {
                Some(KnobValue::Str(s)) => s.to_string(),
                _ => String::new(),
            };
            // Free-form text edits leak via Box::leak so the resulting
            // &'static str can flow into KnobValue::Str. This is bounded
            // and fine for an interactive dev tool; the snapshot
            // harness uses defaults verbatim.
            let on_change = move |e: FormEvent| {
                let leaked: &'static str = Box::leak(e.value().into_boxed_str());
                knob_state.write().insert(name, KnobValue::Str(leaked));
            };
            if *multiline {
                rsx! {
                    textarea {
                        class: "{input_class} h-20 font-mono",
                        value: "{value_str}",
                        oninput: on_change,
                    }
                }
            } else {
                rsx! {
                    input {
                        r#type: "text",
                        class: input_class,
                        value: "{value_str}",
                        oninput: on_change,
                    }
                }
            }
        }
        KnobKind::Enum { variants } => {
            let selected = match &current {
                Some(KnobValue::EnumVariant(v)) => v.to_string(),
                _ => variants.first().map(|s| s.to_string()).unwrap_or_default(),
            };
            rsx! {
                select {
                    class: input_class,
                    onchange: move |e| {
                        let leaked: &'static str = Box::leak(e.value().into_boxed_str());
                        knob_state.write().insert(name, KnobValue::EnumVariant(leaked));
                    },
                    for v in variants.iter() {
                        option { value: "{v}", selected: selected == *v, "{v}" }
                    }
                }
            }
        }
        KnobKind::Color | KnobKind::Opaque => rsx! {
            span { class: "text-[11px] text-muted-foreground italic",
                "(no editor — uses default)"
            }
        },
    }
}

fn clone_knob(v: &KnobValue) -> KnobValue {
    v.clone()
}

#[component]
fn EmptyState() -> Element {
    rsx! {
        div { class: "max-w-md text-muted-foreground text-sm",
            h2 { class: "text-foreground text-base font-semibold mb-2", "No stories registered" }
            p {
                "Add "
                code { class: "rounded bg-accent/40 px-1 py-0.5 text-[12px]", "#[story]" }
                " to a Dioxus component, or hand-roll a "
                code { class: "rounded bg-accent/40 px-1 py-0.5 text-[12px]", "Story" }
                " value and register it via "
                code { class: "rounded bg-accent/40 px-1 py-0.5 text-[12px]", "#[linkme::distributed_slice(STORIES)]" }
                "."
            }
        }
    }
}

/// `KnobSource` over a borrowed map. Cheap to construct per-render so
/// `StoryPreview` doesn't have to plumb signals through the trait.
struct MapKnobs<'a>(&'a HashMap<&'static str, KnobValue>);

impl KnobSource for MapKnobs<'_> {
    fn get(&self, name: &'static str) -> Option<&KnobValue> {
        self.0.get(name)
    }
}
