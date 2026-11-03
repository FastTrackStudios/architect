//! The renderer (feature `ui`): a surface drawn with architect-ui
//! components, on Blitz.
//!
//! [`use_a2ui`] puts the app's [`Surfaces`] and Action List in context;
//! [`A2uiSurface`] draws one surface, live: a change to its document (an
//! agent's patch, another person's edit, the person typing) redraws it.
//! Each component is drawn by the catalog's architect-ui component
//! ([`draws`]); one the catalog does not list shows a labelled
//! placeholder, and none of its props are read. A field bound to the data
//! model writes into the document ([`Surfaces::set_data`]); a button runs
//! its action ([`crate::events::press`]) and, when it waits for the
//! person's yes, says so until they answer in the confirmation sheet.
//!
//! [`surfaces_view`] is a workspace view of every surface.

// Dioxus is single-threaded: its tasks are not `Send`.
#![allow(clippy::future_not_send)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use a2ui_base::catalog::function_api::FunctionImplementation;
use a2ui_base::model::component_context::ComponentContext;
use a2ui_base::model::component_model::ComponentModel;
use a2ui_base::model::surface_model::SurfaceModel;
use a2ui_base::protocol::common_types::{
    ChildList, DynamicBoolean, DynamicNumber, DynamicString, DynamicStringList,
};
use architect::action::ActionList;
use architect_ui::prelude::*;
use architect_workspace::View;
use dioxus::prelude::*;
use futures_util::StreamExt as _;
use serde_json::Value;

use crate::actions::VIEW;
use crate::catalog;
use crate::events::{self, Outcome};
use crate::store::Surfaces;

/// A press the person sees the state of under its button.
#[derive(Clone, PartialEq, Eq)]
enum Press {
    /// Waiting for the person's yes: the button is off until they answer.
    Waiting(String),
    /// Refused, or not approved: said, and the button can be pressed again.
    Refused(String),
}

/// What a surface's components read: the store, the list, and a count
/// that moves on every change to the document.
#[derive(Clone)]
pub struct A2ui {
    pub surfaces: Surfaces,
    pub list: ActionList,
    version: Signal<u64>,
    /// Per component (`surface/id`): a pending or refused press.
    status: Signal<HashMap<String, Press>>,
    functions: Rc<HashMap<String, Box<dyn FunctionImplementation>>>,
}

/// Puts `surfaces` and `list` in context for [`A2uiSurface`]s, redrawing
/// them on every change to the document.
pub fn use_a2ui(surfaces: Surfaces, list: ActionList) -> A2ui {
    let mut version = use_signal(|| 0_u64);
    // The subscription holds only a channel's sender, never the document;
    // the window wakes when something arrives, and never otherwise.
    let waiting = use_hook(|| Rc::new(RefCell::new(None)));
    let _kept = use_hook({
        let surfaces = surfaces.clone();
        let waiting = waiting.clone();
        move || {
            let (tx, rx) = futures_channel::mpsc::unbounded::<()>();
            *waiting.borrow_mut() = Some(rx);
            Rc::new(surfaces.subscribe(move || {
                let _ = tx.unbounded_send(());
            }))
        }
    });
    use_future(move || {
        let changes = waiting.borrow_mut().take();
        async move {
            let Some(mut changes) = changes else { return };
            while changes.next().await.is_some() {
                // Every change waiting now is one redraw.
                while changes.try_recv().is_ok() {}
                let now = *version.peek();
                version.set(now.wrapping_add(1));
            }
        }
    });
    let status = use_signal(HashMap::new);
    let functions = use_hook(|| Rc::new(events::functions()));
    use_context_provider(|| A2ui {
        surfaces,
        list,
        version,
        status,
        functions,
    })
}

/// The architect-ui component that draws catalog component `name`, if
/// the catalog has one: `None` is the placeholder. A test checks it
/// draws exactly the catalog.
#[must_use]
pub fn draws(name: &str) -> Option<fn(&Drawn) -> Element> {
    let draw: fn(&Drawn) -> Element = match name {
        "Text" => text,
        "Row" | "Column" | "List" => stack,
        "Card" => card,
        "Divider" => divider,
        "Button" => button,
        "CheckBox" => |d| rsx! { CheckNode { at: d.at.clone() } },
        "TextField" => |d| rsx! { FieldNode { at: d.at.clone() } },
        "Slider" => |d| rsx! { SliderNode { at: d.at.clone() } },
        "ChoicePicker" => |d| rsx! { ChoiceNode { at: d.at.clone() } },
        "Tabs" => |d| rsx! { TabsNode { at: d.at.clone() } },
        "Image" => image,
        "Icon" => icon,
        "Video" => video,
        "AudioPlayer" => audio,
        "Modal" => |d| rsx! { ModalNode { at: d.at.clone() } },
        "DateTimeInput" => |d| rsx! { DateNode { at: d.at.clone() } },
        _ => return None,
    };
    catalog::component(name).map(|_| draw)
}

/// Where a component is: its surface, its id, its data path (a template
/// item's, or `""`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct At {
    pub surface: String,
    pub id: String,
    pub base: String,
}

/// A component being drawn: where it is, its props, and its values read
/// through the data model.
pub struct Drawn {
    pub at: At,
    model: ComponentModel,
    texts: HashMap<&'static str, String>,
    numbers: HashMap<&'static str, f64>,
    children: Vec<(String, String)>,
    words: HashMap<&'static str, String>,
}

impl Drawn {
    fn text(&self, name: &str) -> String {
        self.texts.get(name).cloned().unwrap_or_default()
    }

    fn word(&self, name: &str) -> &str {
        self.words.get(name).map_or("", String::as_str)
    }

    fn kids(&self) -> Element {
        let surface = self.at.surface.clone();
        rsx! {
            for (id, base) in self.children.clone() {
                Node { key: "{id}{base}", surface: surface.clone(), id, base }
            }
        }
    }
}

/// Reads what the catalog says component `def` takes, through the data
/// model: the only props of it that are ever looked at.
fn read(at: At, model: &ComponentModel, cx: &ComponentContext<'_>) -> Drawn {
    let mut drawn = Drawn {
        at,
        model: model.clone(),
        texts: HashMap::new(),
        numbers: HashMap::new(),
        children: Vec::new(),
        words: HashMap::new(),
    };
    let Some(def) = catalog::component(&model.component_type) else {
        return drawn;
    };
    let base = cx.data_context.base_path().to_owned();
    for p in def.props {
        match p.prop {
            catalog::Prop::Text => {
                if let Some(ds) = model.get_property::<DynamicString>(p.name) {
                    drawn
                        .texts
                        .insert(p.name, cx.data_context.resolve_dynamic_string(&ds));
                }
            }
            catalog::Prop::Number => {
                if let Some(dn) = model.get_property::<DynamicNumber>(p.name) {
                    drawn
                        .numbers
                        .insert(p.name, cx.data_context.resolve_dynamic_number(&dn));
                }
            }
            catalog::Prop::OneOf(words) => {
                if let Some(w) = model
                    .get_property::<String>(p.name)
                    .filter(|w| words.contains(&w.as_str()))
                {
                    drawn.words.insert(p.name, w);
                }
            }
            catalog::Prop::Child => {
                if let Some(id) = model.get_property::<String>(p.name) {
                    drawn.children.push((id, base.clone()));
                }
            }
            catalog::Prop::Children => match model.children() {
                Some(ChildList::Static(ids)) => {
                    drawn
                        .children
                        .extend(ids.into_iter().map(|id| (id, base.clone())));
                }
                Some(ChildList::Template { component_id, path }) => {
                    if let Some(Value::Array(items)) = cx.data_context.get(&path) {
                        let at = cx.data_context.resolve_pointer(&path);
                        for i in 0..items.len() {
                            drawn
                                .children
                                .push((component_id.clone(), format!("{at}/{i}")));
                        }
                    }
                }
                None => {}
            },
            catalog::Prop::Boolean
            | catalog::Prop::TextList
            | catalog::Prop::Action
            | catalog::Prop::Options
            | catalog::Prop::Tabs => {}
        }
    }
    drawn
}

/// One component of a surface, drawn by its catalog entry.
#[component]
fn Node(surface: String, id: String, base: String) -> Element {
    let a2ui = use_context::<A2ui>();
    let model = use_context::<Signal<Option<Rc<SurfaceModel>>>>();
    let _ = (a2ui.version)();
    let Some(model) = model() else {
        return rsx! {};
    };
    let components = model.components.borrow();
    let data = model.data_model.borrow();
    let Some(c) = components.get(&id) else {
        // Not sent yet: A2UI draws what it has.
        return rsx! { div { style: "min-height:16px;" } };
    };
    let cx = ComponentContext::new(
        id.clone(),
        surface.clone(),
        &data,
        &components,
        &a2ui.functions,
        &base,
        None,
    );
    let at = At {
        surface,
        id: id.clone(),
        base,
    };
    draws(&c.component_type).map_or_else(
        || placeholder(&id, &c.component_type),
        |draw| draw(&read(at, c, &cx)),
    )
}

fn placeholder(id: &str, name: &str) -> Element {
    let why = catalog::refusal(name);
    rsx! {
        div {
            "data-a2ui-placeholder": "{id}",
            style: "border:1px dashed var(--border, #888); border-radius:8px; padding:8px 10px; \
                    font-size:12px; color:var(--muted-foreground, #888);",
            "{why}"
        }
    }
}

fn text(d: &Drawn) -> Element {
    let words = d.text("text");
    match d.word("variant") {
        "h1" => rsx! { Heading { level: HeadingLevel::H1, "{words}" } },
        "h2" => rsx! { Heading { level: HeadingLevel::H2, "{words}" } },
        "h3" => rsx! { Heading { level: HeadingLevel::H3, "{words}" } },
        "h4" => rsx! { Heading { level: HeadingLevel::H4, "{words}" } },
        "h5" => rsx! { Heading { level: HeadingLevel::H5, "{words}" } },
        "caption" => rsx! { Text { variant: TextVariant::Muted, "{words}" } },
        _ => rsx! { Text { "{words}" } },
    }
}

fn flex(word: &str) -> &'static str {
    match word {
        "center" => "center",
        "end" => "flex-end",
        "spaceBetween" => "space-between",
        "spaceAround" => "space-around",
        "spaceEvenly" => "space-evenly",
        "stretch" => "stretch",
        _ => "flex-start",
    }
}

fn stack(d: &Drawn) -> Element {
    let row = d.model.component_type == "Row" || d.word("direction") == "horizontal";
    let style = format!(
        "display:flex; flex-direction:{}; gap:8px; justify-content:{}; align-items:{};",
        if row { "row" } else { "column" },
        flex(d.word("justify")),
        match d.word("align") {
            "" => "stretch",
            w => flex(w),
        },
    );
    let kind = &d.model.component_type;
    rsx! { div { "data-a2ui": "{kind}", style, {d.kids()} } }
}

fn card(d: &Drawn) -> Element {
    rsx! { Card { CardContent { {d.kids()} } } }
}

fn divider(d: &Drawn) -> Element {
    let orientation = if d.word("axis") == "vertical" {
        DividerOrientation::Vertical
    } else {
        DividerOrientation::Horizontal
    };
    rsx! { Divider { orientation } }
}

fn image(d: &Drawn) -> Element {
    let (url, alt) = (d.text("url"), d.text("description"));
    let fit = match d.word("fit") {
        "" => "contain",
        w => w,
    };
    rsx! { img { src: "{url}", alt: "{alt}", style: "max-width:100%; object-fit:{fit};" } }
}

/// The lucide icon for an A2UI basic icon name ([`catalog::ICONS`]).
fn glyph(name: &str) -> Element {
    use architect_ui::lucide_dioxus as l;
    let size = 20;
    match name {
        "accountCircle" => rsx! { l::CircleUser { size } },
        "add" => rsx! { l::Plus { size } },
        "arrowBack" => rsx! { l::ArrowLeft { size } },
        "arrowForward" => rsx! { l::ArrowRight { size } },
        "attachFile" => rsx! { l::Paperclip { size } },
        "calendarToday" => rsx! { l::Calendar { size } },
        "call" => rsx! { l::Phone { size } },
        "camera" => rsx! { l::Camera { size } },
        "check" => rsx! { l::Check { size } },
        "close" => rsx! { l::X { size } },
        "delete" => rsx! { l::Trash { size } },
        "download" => rsx! { l::Download { size } },
        "edit" => rsx! { l::Pencil { size } },
        "event" => rsx! { l::CalendarDays { size } },
        "error" => rsx! { l::CircleAlert { size } },
        "fastForward" => rsx! { l::FastForward { size } },
        "favorite" => rsx! { l::Heart { size } },
        "favoriteOff" => rsx! { l::HeartOff { size } },
        "folder" => rsx! { l::Folder { size } },
        "help" => rsx! { l::CircleQuestionMark { size } },
        "home" => rsx! { l::House { size } },
        "info" => rsx! { l::Info { size } },
        "locationOn" => rsx! { l::MapPin { size } },
        "lock" => rsx! { l::Lock { size } },
        "lockOpen" => rsx! { l::LockOpen { size } },
        "mail" => rsx! { l::Mail { size } },
        "menu" => rsx! { l::Menu { size } },
        "moreVert" => rsx! { l::EllipsisVertical { size } },
        "moreHoriz" => rsx! { l::Ellipsis { size } },
        "notificationsOff" => rsx! { l::BellOff { size } },
        "notifications" => rsx! { l::Bell { size } },
        "pause" => rsx! { l::Pause { size } },
        "payment" => rsx! { l::CreditCard { size } },
        "person" => rsx! { l::User { size } },
        "phone" => rsx! { l::Smartphone { size } },
        "photo" => rsx! { l::Image { size } },
        "play" => rsx! { l::Play { size } },
        "print" => rsx! { l::Printer { size } },
        "refresh" => rsx! { l::RefreshCw { size } },
        "rewind" => rsx! { l::Rewind { size } },
        "search" => rsx! { l::Search { size } },
        "send" => rsx! { l::Send { size } },
        "settings" => rsx! { l::Settings { size } },
        "share" => rsx! { l::Share2 { size } },
        "shoppingCart" => rsx! { l::ShoppingCart { size } },
        "skipNext" => rsx! { l::SkipForward { size } },
        "skipPrevious" => rsx! { l::SkipBack { size } },
        "star" => rsx! { l::Star { size } },
        "starHalf" => rsx! { l::StarHalf { size } },
        "starOff" => rsx! { l::StarOff { size } },
        "stop" => rsx! { l::Square { size } },
        "upload" => rsx! { l::Upload { size } },
        "visibility" => rsx! { l::Eye { size } },
        "visibilityOff" => rsx! { l::EyeOff { size } },
        "volumeDown" => rsx! { l::Volume1 { size } },
        "volumeMute" => rsx! { l::Volume { size } },
        "volumeOff" => rsx! { l::VolumeOff { size } },
        "volumeUp" => rsx! { l::Volume2 { size } },
        "warning" => rsx! { l::TriangleAlert { size } },
        // Not a name of the set (`read` keeps only those): nothing.
        _ => rsx! {},
    }
}

fn icon(d: &Drawn) -> Element {
    let name = d.word("name");
    rsx! {
        span { "data-a2ui": "Icon", role: "img", "aria-label": "{name}",
            style: "display:inline-flex; width:20px; height:20px;",
            {glyph(name)}
        }
    }
}

/// A browser host: it plays `<video>` and `<audio>` and has date and time
/// pickers. Blitz lays media out as empty boxes and edits only text, so
/// there media say what and where they are, and a date is typed.
const BROWSER: bool = cfg!(target_arch = "wasm32");

/// Media a host cannot play: what it is, and its URL.
fn unplayed(kind: &str, what: &str, url: &str) -> Element {
    rsx! {
        div { "data-a2ui": "{kind}",
            style: "display:flex; gap:10px; align-items:center; padding:10px 12px; \
                    border:1px solid var(--border, #888); border-radius:8px;",
            if kind == "Video" {
                architect_ui::lucide_dioxus::Film { size: 20 }
            } else {
                architect_ui::lucide_dioxus::AudioLines { size: 20 }
            }
            div { style: "display:flex; flex-direction:column; gap:2px; min-width:0;",
                Text { "{what}" }
                Text { variant: TextVariant::Muted, "{url}" }
            }
        }
    }
}

fn video(d: &Drawn) -> Element {
    let url = d.text("url");
    if !BROWSER {
        return unplayed("Video", "Video", &url);
    }
    let poster = Some(d.text("posterUrl")).filter(|p| !p.is_empty());
    rsx! { video { "data-a2ui": "Video", src: "{url}", poster, controls: true, style: "max-width:100%;" } }
}

fn audio(d: &Drawn) -> Element {
    let (url, what) = (d.text("url"), d.text("description"));
    if !BROWSER {
        let what = if what.is_empty() {
            "Audio".to_owned()
        } else {
            what
        };
        return unplayed("AudioPlayer", &what, &url);
    }
    rsx! {
        div { "data-a2ui": "AudioPlayer", style: "display:flex; flex-direction:column; gap:4px;",
            if !what.is_empty() {
                Text { variant: TextVariant::Small, "{what}" }
            }
            audio { src: "{url}", controls: true, "aria-label": "{what}" }
        }
    }
}

fn status_key(at: &At) -> String {
    format!("{}/{}{}", at.surface, at.id, at.base)
}

fn button(d: &Drawn) -> Element {
    let variant = match d.word("variant") {
        "primary" => ButtonVariant::Primary,
        "borderless" => ButtonVariant::Ghost,
        _ => ButtonVariant::Secondary,
    };
    rsx! { ButtonNode { at: d.at.clone(), variant, label: d.kids() } }
}

#[component]
fn ButtonNode(at: At, variant: ButtonVariant, label: Element) -> Element {
    let a2ui = use_context::<A2ui>();
    let key = status_key(&at);
    let state = a2ui.status.read().get(&key).cloned();
    let busy = matches!(state, Some(Press::Waiting(_)));
    let said = state.map(|(Press::Waiting(w) | Press::Refused(w))| w);
    let named = at.id.clone();
    rsx! {
        div { style: "display:flex; flex-direction:column; gap:4px;",
            "data-a2ui-button": "{named}",
            Button {
                variant,
                disabled: busy,
                on_click: move |_| {
                    let (a2ui, at) = (a2ui.clone(), at.clone());
                    spawn(async move { pressed(a2ui, at).await });
                },
                {label}
            }
            if let Some(said) = said {
                Text { variant: TextVariant::Muted, "{said}" }
            }
        }
    }
}

/// Runs a press, and — when it waits for the person's yes — waits for
/// their answer, then runs it again with the ticket.
async fn pressed(a2ui: A2ui, at: At) {
    let key = status_key(&at);
    let mut status = a2ui.status;
    let mut since = a2ui.list.latest();
    let mut ticket: Option<String> = None;
    loop {
        let out = events::press(
            &a2ui.list,
            &a2ui.surfaces,
            (&at.surface, &at.id, &at.base),
            ticket.as_deref(),
        )
        .await;
        match out {
            Outcome::Ran(_) => {
                status.write().remove(&key);
                return;
            }
            Outcome::Refused(why) => {
                status.write().insert(key, Press::Refused(why));
                return;
            }
            Outcome::Waiting { id, ticket: t } => {
                status.write().insert(
                    key.clone(),
                    Press::Waiting(format!("Waiting for your yes to run {id}…")),
                );
                loop {
                    futures_timer::Delay::new(Duration::from_millis(120)).await;
                    let (seen, next) = a2ui.list.events_since(since, 256, Duration::ZERO);
                    since = next;
                    match events::answered(&seen, &t) {
                        Some(true) => break,
                        Some(false) => {
                            status
                                .write()
                                .insert(key, Press::Refused(format!("{id} was not approved")));
                            return;
                        }
                        None => {}
                    }
                }
                ticket = Some(t);
            }
        }
    }
}

/// The data path a prop is bound to, if it is (`{"path": …}`).
fn bound(model: &ComponentModel, prop: &str, base: &str) -> Option<String> {
    let path = model.get_raw(prop)?.get("path")?.as_str()?.to_owned();
    Some(if path.starts_with('/') || base.is_empty() {
        if path.starts_with('/') {
            path
        } else {
            format!("/{path}")
        }
    } else {
        format!("{base}/{path}")
    })
}

/// A component's model and its data context, read in a hook-holding
/// child: `f` gets the component (when it is still there) and resolves
/// what it needs.
fn with_model<R>(
    at: &At,
    f: impl FnOnce(&ComponentModel, &ComponentContext<'_>) -> R,
) -> Option<R> {
    let a2ui = use_context::<A2ui>();
    let model = use_context::<Signal<Option<Rc<SurfaceModel>>>>();
    let _ = (a2ui.version)();
    let model = model()?;
    let components = model.components.borrow();
    let data = model.data_model.borrow();
    let c = components.get(&at.id)?;
    let cx = ComponentContext::new(
        at.id.clone(),
        at.surface.clone(),
        &data,
        &components,
        &a2ui.functions,
        &at.base,
        None,
    );
    Some(f(c, &cx))
}

/// A signal of a bound value: follows the document, and writes to it
/// when the person changes it.
fn use_bound<T: Clone + PartialEq + 'static>(
    at: &At,
    now: T,
    path: Option<String>,
    to_json: fn(&T) -> Value,
) -> Signal<T> {
    let a2ui = use_context::<A2ui>();
    let mut value = use_signal(|| now.clone());
    let mut seen = use_signal(|| now.clone());
    if *seen.peek() != now {
        seen.set(now.clone());
        value.set(now);
    }
    let surface = at.surface.clone();
    use_effect(move || {
        let v = value();
        if v != *seen.peek() {
            seen.set(v.clone());
            if let Some(path) = &path {
                let _ = a2ui.surfaces.set_data(&surface, path, Some(&to_json(&v)));
            }
        }
    });
    value
}

#[component]
fn CheckNode(at: At) -> Element {
    let read = with_model(&at, |c, cx| {
        let label = c
            .get_property::<DynamicString>("label")
            .map(|d| cx.data_context.resolve_dynamic_string(&d))
            .unwrap_or_default();
        let on = c
            .get_property::<DynamicBoolean>("value")
            .is_some_and(|d| cx.data_context.resolve_dynamic_boolean(&d));
        (label, on, bound(c, "value", &at.base))
    });
    let (label, on, path) = read.unwrap_or_default();
    let checked = use_bound(&at, on, path, |b| Value::Bool(*b));
    rsx! {
        label { "data-a2ui": "CheckBox", style: "display:flex; gap:8px; align-items:center;",
            Checkbox { checked }
            Text { "{label}" }
        }
    }
}

#[component]
fn FieldNode(at: At) -> Element {
    let read = with_model(&at, |c, cx| {
        let s = |k: &str| {
            c.get_property::<DynamicString>(k)
                .map(|d| cx.data_context.resolve_dynamic_string(&d))
                .unwrap_or_default()
        };
        let variant = c.get_property::<String>("variant").unwrap_or_default();
        (s("label"), s("value"), variant, bound(c, "value", &at.base))
    });
    let (label, now, variant, path) = read.unwrap_or_default();
    let numeric = variant == "number";
    let value = use_bound(
        &at,
        now,
        path,
        if numeric {
            number_json
        } else {
            |s: &String| Value::String(s.clone())
        },
    );
    let input_type = match variant.as_str() {
        "number" => Some("number".to_owned()),
        "obscured" => Some("password".to_owned()),
        _ => None,
    };
    rsx! {
        label { "data-a2ui": "TextField", style: "display:flex; flex-direction:column; gap:4px;",
            Text { variant: TextVariant::Small, "{label}" }
            if variant == "longText" {
                Textarea { value }
            } else {
                Input { value, input_type }
            }
        }
    }
}

// A `use_bound` converter: a `fn(&String)`, the bound value's type.
#[allow(clippy::ptr_arg)]
fn number_json(s: &String) -> Value {
    s.trim()
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map_or_else(|| Value::String(s.clone()), Value::Number)
}

#[component]
fn SliderNode(at: At) -> Element {
    let read = with_model(&at, |c, cx| {
        let n = |k: &str, or: f64| {
            c.get_property::<DynamicNumber>(k)
                .map_or(or, |d| cx.data_context.resolve_dynamic_number(&d))
        };
        let label = c
            .get_property::<DynamicString>("label")
            .map(|d| cx.data_context.resolve_dynamic_string(&d))
            .unwrap_or_default();
        (
            label,
            n("value", 0.0),
            n("min", 0.0),
            n("max", 100.0),
            bound(c, "value", &at.base),
        )
    });
    let (label, now, min, max, path) = read.unwrap_or((String::new(), 0.0, 0.0, 100.0, None));
    let value = use_bound(&at, now, path, |v| {
        serde_json::Number::from_f64(*v).map_or(Value::Null, Value::Number)
    });
    let shown = value();
    rsx! {
        div { "data-a2ui": "Slider", style: "display:flex; flex-direction:column; gap:6px;",
            Text { variant: TextVariant::Small, "{label}: {shown}" }
            Slider { value, min, max }
        }
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[component]
fn ChoiceNode(at: At) -> Element {
    let read = with_model(&at, |c, cx| {
        let label = c
            .get_property::<DynamicString>("label")
            .map(|d| cx.data_context.resolve_dynamic_string(&d))
            .unwrap_or_default();
        let options: Vec<(String, String)> = c
            .get_raw("options")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|o| {
                        let value = o.get("value")?.as_str()?.to_owned();
                        let label = o
                            .get("label")
                            .and_then(|l| serde_json::from_value::<DynamicString>(l.clone()).ok())
                            .map_or_else(
                                || value.clone(),
                                |d| cx.data_context.resolve_dynamic_string(&d),
                            );
                        Some((label, value))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let chosen: Vec<String> = match c.get_property::<DynamicStringList>("value") {
            Some(DynamicStringList::Literal(v)) => v,
            Some(DynamicStringList::Binding(b)) => strings(cx.data_context.get(&b.path).as_ref()),
            Some(DynamicStringList::Function(f)) => {
                strings(Some(&cx.data_context.resolve_dynamic_value(
                    &a2ui_base::protocol::common_types::DynamicValue::Function(f),
                )))
            }
            None => Vec::new(),
        };
        let many = c.get_property::<String>("variant").as_deref() == Some("multipleSelection");
        (label, options, chosen, many, bound(c, "value", &at.base))
    });
    let (label, options, now, many, path) = read.unwrap_or_default();
    let mut chosen = use_bound(&at, now, path, |v: &Vec<String>| {
        Value::Array(v.iter().cloned().map(Value::String).collect())
    });
    rsx! {
        div { "data-a2ui": "ChoicePicker", style: "display:flex; flex-direction:column; gap:6px;",
            Text { variant: TextVariant::Small, "{label}" }
            div { style: "display:flex; flex-wrap:wrap; gap:6px;",
                for (label, value) in options {
                    {
                        let on = chosen.read().contains(&value);
                        rsx! {
                            Button {
                                key: "{value}",
                                variant: if on { ButtonVariant::Primary } else { ButtonVariant::Outline },
                                on_click: move |_| {
                                    let mut now = chosen.peek().clone();
                                    if many {
                                        if let Some(i) = now.iter().position(|v| *v == value) {
                                            now.remove(i);
                                        } else {
                                            now.push(value.clone());
                                        }
                                    } else {
                                        now = vec![value.clone()];
                                    }
                                    chosen.set(now);
                                },
                                "{label}"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TabsNode(at: At) -> Element {
    let read = with_model(&at, |c, cx| {
        let tabs: Vec<(String, String)> = c
            .get_raw("tabs")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|t| {
                        let child = t.get("child")?.as_str()?.to_owned();
                        let title = t
                            .get("title")
                            .and_then(|l| serde_json::from_value::<DynamicString>(l.clone()).ok())
                            .map(|d| cx.data_context.resolve_dynamic_string(&d))
                            .unwrap_or_default();
                        Some((title, child))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let active = c
            .get_property::<DynamicNumber>("activeTab")
            .map_or(0.0, |d| cx.data_context.resolve_dynamic_number(&d));
        (tabs, active, bound(c, "activeTab", &at.base))
    });
    let (tabs, now, path) = read.unwrap_or_default();
    let mut active = use_bound(&at, now, path, |v| {
        serde_json::Number::from_f64(*v).map_or(Value::Null, Value::Number)
    });
    let last = tabs.len().saturating_sub(1);
    let at_index = |i: usize| f64::from(u32::try_from(i).unwrap_or(u32::MAX));
    let want = active().round();
    let shown = (0..=last).rev().find(|&i| at_index(i) <= want).unwrap_or(0);
    let child = tabs.get(shown).map(|(_, c)| c.clone());
    let At { surface, base, .. } = at;
    rsx! {
        div { "data-a2ui": "Tabs", style: "display:flex; flex-direction:column; gap:8px;",
            div { style: "display:flex; gap:4px;",
                for (i, (title, _)) in tabs.into_iter().enumerate() {
                    Button {
                        key: "{i}",
                        variant: if i == shown { ButtonVariant::Secondary } else { ButtonVariant::Ghost },
                        on_click: move |_| active.set(at_index(i)),
                        "{title}"
                    }
                }
            }
            if let Some(id) = child {
                Node { surface, id, base }
            }
        }
    }
}

/// A Modal: its trigger in place; a press anywhere on it (a Button's
/// action runs too) opens a dialog of its content, closed by its ✕,
/// Escape or a press outside.
#[component]
fn ModalNode(at: At) -> Element {
    let read = with_model(&at, |c, _| {
        (
            c.get_property::<String>("trigger"),
            c.get_property::<String>("content"),
        )
    });
    let (trigger, content) = read.unwrap_or_default();
    let mut open = use_signal(|| false);
    let At { surface, id, base } = at;
    rsx! {
        div { "data-a2ui": "Modal",
            div { "data-a2ui-modal": "{id}", onclick: move |_| open.set(true),
                if let Some(id) = trigger {
                    Node { surface: surface.clone(), id, base: base.clone() }
                }
            }
            Dialog { open: open(), on_close: move |()| open.set(false),
                DialogClose { on_click: move |()| open.set(false) }
                div { "data-a2ui-modal-content": "{id}",
                    if let Some(id) = content {
                        Node { surface, id, base }
                    }
                }
            }
        }
    }
}

/// What an ISO 8601 date, time or both is typed as where the host has no
/// picker for it (Blitz edits only text).
fn iso_hint(kind: &str) -> &'static str {
    match kind {
        "date" => "YYYY-MM-DD",
        "time" => "HH:MM",
        _ => "YYYY-MM-DDTHH:MM",
    }
}

#[component]
fn DateNode(at: At) -> Element {
    let read = with_model(&at, |c, cx| {
        let s = |k: &str| {
            c.get_property::<DynamicString>(k)
                .map(|d| cx.data_context.resolve_dynamic_string(&d))
                .unwrap_or_default()
        };
        let b = |k: &str| {
            c.get_property::<DynamicBoolean>(k)
                .is_some_and(|d| cx.data_context.resolve_dynamic_boolean(&d))
        };
        (
            s("label"),
            s("value"),
            (b("enableDate"), b("enableTime")),
            (s("min"), s("max")),
            bound(c, "value", &at.base),
        )
    });
    let (label, now, (date, time), (min, max), path) = read.unwrap_or_default();
    let value = use_bound(&at, now, path, |s: &String| Value::String(s.clone()));
    let kind = match (date, time) {
        (true, false) => "date",
        (false, true) => "time",
        _ => "datetime-local",
    };
    let some = |s: String| Some(s).filter(|s| !s.is_empty());
    // A browser draws its picker; Blitz a text field that says the form.
    let input_type = Some(if BROWSER { kind } else { "text" }.to_owned());
    rsx! {
        label { "data-a2ui": "DateTimeInput", style: "display:flex; flex-direction:column; gap:4px;",
            Text { variant: TextVariant::Small, "{label}" }
            Input {
                value,
                input_type,
                placeholder: iso_hint(kind),
                min: some(min),
                max: some(max),
            }
        }
    }
}

/// One surface, drawn live from its document: `root` first.
#[component]
pub fn A2uiSurface(id: String) -> Element {
    let a2ui = use_context::<A2ui>();
    let mut model = use_context_provider(|| Signal::new(None::<Rc<SurfaceModel>>));
    let version = (a2ui.version)();
    let mut drawn = use_signal(|| None::<(String, u64)>);
    if drawn.peek().as_ref() != Some(&(id.clone(), version)) {
        drawn.set(Some((id.clone(), version)));
        model.set(a2ui.surfaces.get(&id).map(|s| Rc::new(s.model())));
    }
    let present = model.peek().is_some();
    rsx! {
        style { {architect_ui::THEME_CSS} }
        style { {architect_ui::UTILITIES_CSS} }
        div { "data-a2ui-surface": "{id}", style: "display:flex; flex-direction:column; gap:12px; padding:12px;",
            if present {
                Node { surface: id.clone(), id: "root".to_owned(), base: String::new() }
            } else {
                Text { variant: TextVariant::Muted, "No surface “{id}”." }
            }
        }
    }
}

/// Every surface, newest last: the workspace view's body.
#[component]
pub fn SurfacesPane() -> Element {
    let a2ui = use_context::<A2ui>();
    let _ = (a2ui.version)();
    let ids = a2ui.surfaces.ids();
    rsx! {
        div { style: "display:flex; flex-direction:column; flex:1; min-height:0; overflow:scroll;",
            if ids.is_empty() {
                div { style: "padding:24px; text-align:center;",
                    Text { variant: TextVariant::Muted,
                        "No agent UI yet. An agent shows one with A2UI_RENDER."
                    }
                }
            }
            for id in ids {
                A2uiSurface { key: "{id}", id }
            }
        }
    }
}

/// The workspace view of every surface (`a2ui`), for an app's [`Views`]
/// (it reads [`use_a2ui`]'s context).
///
/// [`Views`]: architect_workspace::Views
#[must_use]
pub fn surfaces_view() -> View {
    View::new(VIEW, "Agent UI", |_| rsx! { SurfacesPane {} })
}
