//! A2UI JSON pages (feature `a2ui`): a declarative page, `<view>.a2ui.json`,
//! drawn through architect's A2UI catalog onto the same element tree a TSX
//! or Luau page renders.
//!
//! The file is A2UI v1.0 (`architect_a2ui`, over `a2ui-base`): either the
//! messages an agent sends (`createSurface`, `updateComponents`,
//! `updateDataModel`, as `A2UI_RENDER` takes them), or the page form —
//!
//! ```json
//! {
//!   "components": [
//!     { "id": "root", "component": "Column", "children": ["title", "pin"] },
//!     { "id": "title", "component": "Text", "text": { "path": "/board/title" }, "variant": "h2" },
//!     { "id": "pin", "component": "Button", "child": "pin-label",
//!       "action": { "event": { "name": "LAB_BOARD_PIN", "context": { "title": { "path": "/draft" } } } } },
//!     { "id": "pin-label", "component": "Text", "text": "Pin" }
//!   ],
//!   "data": { "draft": "" },
//!   "bind": { "/board": { "action": "LAB_BOARD_STATE", "every": 1000 } }
//! }
//! ```
//!
//! — its components (one is `root`), its data model, and its **bindings**
//! to its extension's data: each `bind` path holds what its action
//! answers (JSON), run as the page loads and again every `every` ms.
//! A Button's `action` runs its Action List action with its context read
//! through the data model, the page's capabilities its reach (as a TSX
//! page's `actions.run`); `wantResponse` with `responsePath` writes the
//! answer back. A TextField, CheckBox, Slider, ChoicePicker or Tabs bound
//! to a path (`{"path": "/draft"}`) writes it as the person changes it.
//!
//! Only the catalog is drawn (`architect_a2ui::catalog`, the security
//! boundary): a component outside it is a labelled placeholder, as on an
//! agent's surface, and only the props it lists are read. Values resolve
//! through `a2ui-base`'s data context (paths, templates, its functions).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use a2ui_base::catalog::function_api::FunctionImplementation;
use a2ui_base::model::component_context::ComponentContext;
use a2ui_base::model::component_model::ComponentModel;
use a2ui_base::model::components_model::SurfaceComponentsModel;
use a2ui_base::model::data_model::DataModel;
use a2ui_base::protocol::common_types::{
    Action, ChildList, DynamicBoolean, DynamicNumber, DynamicString,
};
use a2ui_base::protocol::server_to_client::A2uiPayload;
use architect_a2ui::a2ui_base;
use architect_a2ui::catalog;
use serde_json::{Map, Value};

use crate::api::{
    Attr, DataEvent, Element, Listener, PageEvent, PageHost as _, ThemeSnapshot, WorkerMessage,
};
use crate::diff::TEXT;
use crate::files::PageFile;
use crate::runner::{Engine, Inbox, Load, Services};

/// The [`Load`] for `.a2ui.json` pages.
#[must_use]
pub fn loader() -> Load {
    Arc::new(|page: &PageFile, services: &Services| {
        let engine: Box<dyn Engine> = Box::new(A2uiEngine::load(page, services)?);
        Ok(engine)
    })
}

/// How deep components may nest: past it, a cycle is assumed.
const DEPTH: usize = 64;

/// The id of the component a page draws first.
pub const ROOT: &str = "root";

/// A binding: `path` holds what `action` answers.
#[derive(Clone, Debug)]
struct Bind {
    path: String,
    action: String,
    args: Value,
    every: Option<Duration>,
    /// When it runs next; `None` while its call is out (or never again).
    next: Option<Instant>,
}

/// What a handler number of the last render does.
#[derive(Clone, Debug)]
enum Handler {
    /// A Button pressed: its component and template path.
    Press { id: String, base: String },
    /// A TextField typed in.
    Text { path: String, number: bool },
    /// A CheckBox clicked.
    Toggle { path: String, now: bool },
    /// A Slider moved.
    Slide { path: String },
    /// A ChoicePicker's option clicked.
    Choose {
        path: String,
        value: String,
        chosen: Vec<String>,
        many: bool,
    },
    /// A Tabs' title clicked: its index, kept at `path` or by the page.
    Tab {
        key: String,
        path: Option<String>,
        index: usize,
    },
}

/// A host call out, by its number.
#[derive(Clone, Debug)]
enum Call {
    Bind(usize),
    Press {
        key: String,
        action: String,
        respond: Option<String>,
    },
}

/// An A2UI page's state, on its thread.
pub struct A2uiEngine {
    components: SurfaceComponentsModel,
    data: DataModel,
    functions: HashMap<String, Box<dyn FunctionImplementation>>,
    binds: Vec<Bind>,
    handlers: Vec<Handler>,
    calls: BTreeMap<u32, Call>,
    next_call: u32,
    /// What a pressed Button says under it (running, failed), by its key.
    said: BTreeMap<String, String>,
    /// The tab on show of a Tabs not bound to a path.
    tabs: BTreeMap<String, usize>,
    services: Services,
    dirty: bool,
}

impl std::fmt::Debug for A2uiEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("A2uiEngine")
            .field("components", &self.components.len())
            .field("binds", &self.binds)
            .finish_non_exhaustive()
    }
}

/// `text`'s JSON, a mistake said with its file, line and column.
fn json(file: &str, text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("{file}:{}:{}: {e}", e.line(), e.column()))
}

impl A2uiEngine {
    /// Loads `page`: its components, data and bindings.
    ///
    /// # Errors
    ///
    /// It is not JSON (with its line and column), not an A2UI page, or it
    /// has no `root` component.
    pub fn load(page: &PageFile, services: &Services) -> Result<Self, String> {
        let file = page
            .path
            .file_name()
            .map_or_else(|| "page".to_owned(), |n| n.to_string_lossy().into_owned());
        let text = std::fs::read_to_string(&page.path).map_err(|e| e.to_string())?;
        let mut engine = Self {
            components: SurfaceComponentsModel::new(),
            data: DataModel::new(),
            functions: architect_a2ui::events::functions(),
            binds: Vec::new(),
            handlers: Vec::new(),
            calls: BTreeMap::new(),
            next_call: 0,
            said: BTreeMap::new(),
            tabs: BTreeMap::new(),
            services: services.clone(),
            dirty: true,
        };
        engine.read(&file, &json(&file, &text)?)?;
        if !engine.components.contains(ROOT) {
            return Err(format!(
                "{file}: it has no component \"{ROOT}\" (the one a page draws first)"
            ));
        }
        Ok(engine)
    }

    /// The file's JSON, either form.
    fn read(&mut self, file: &str, value: &Value) -> Result<(), String> {
        match value {
            Value::Array(messages) => self.messages(file, messages),
            Value::Object(o) => {
                if let Some(messages) = o.get("messages") {
                    let messages = messages.as_array().ok_or_else(|| {
                        format!("{file}: \"messages\" is a list of A2UI messages")
                    })?;
                    self.messages(file, messages)?;
                }
                if let Some(components) = o.get("components") {
                    let components = components
                        .as_array()
                        .ok_or_else(|| format!("{file}: \"components\" is a list"))?;
                    self.components(file, components)?;
                }
                if let Some(data) = o.get("data") {
                    self.data.replace_all(data.clone());
                }
                if let Some(bind) = o.get("bind") {
                    self.binds(file, bind)?;
                }
                let known = ["messages", "components", "data", "bind"];
                if let Some(other) = o.keys().find(|k| !known.contains(&k.as_str())) {
                    return Err(format!(
                        "{file}: no field \"{other}\" (a page has components, data, bind, or messages)"
                    ));
                }
                Ok(())
            }
            _ => Err(format!(
                "{file}: an A2UI page is an object (components, data, bind) or a list of A2UI messages"
            )),
        }
    }

    fn messages(&mut self, file: &str, messages: &[Value]) -> Result<(), String> {
        for (i, m) in messages.iter().enumerate() {
            let message = architect_a2ui::store::message(m.clone())
                .map_err(|e| format!("{file}: message {}: {e}", i.saturating_add(1)))?;
            match message.payload {
                A2uiPayload::UpdateComponents(u) => {
                    self.components(file, &u.update_components.components)?;
                }
                A2uiPayload::UpdateDataModel(u) => {
                    let d = u.update_data_model;
                    let path = d.path.unwrap_or_else(|| "/".to_owned());
                    let value = d.value.unwrap_or(Value::Null);
                    if path == "/" || path.is_empty() {
                        self.data.replace_all(value);
                    } else {
                        self.data.set(&path, value);
                    }
                }
                // One surface per page: its id and catalog are the page's;
                // what it starts with is read.
                A2uiPayload::CreateSurface(c) => {
                    let c = c.create_surface;
                    if let Some(components) = c.components {
                        self.components(file, &components)?;
                    }
                    if let Some(data) = c.data_model {
                        self.data.replace_all(data);
                    }
                }
                A2uiPayload::DeleteSurface(_)
                | A2uiPayload::CallFunction(_)
                | A2uiPayload::ActionResponse(_) => {}
            }
        }
        Ok(())
    }

    fn components(&mut self, file: &str, components: &[Value]) -> Result<(), String> {
        for (i, c) in components.iter().enumerate() {
            let model = ComponentModel::from_json(c)
                .map_err(|e| format!("{file}: component {}: {e}", i.saturating_add(1)))?;
            self.components.upsert(model);
        }
        Ok(())
    }

    fn binds(&mut self, file: &str, bind: &Value) -> Result<(), String> {
        let bind = bind
            .as_object()
            .ok_or_else(|| format!("{file}: \"bind\" maps data paths to actions"))?;
        let now = Instant::now();
        for (path, spec) in bind {
            let (action, args, every) = match spec {
                Value::String(action) => (action.clone(), Value::Null, None),
                Value::Object(o) => (
                    o.get("action")
                        .and_then(Value::as_str)
                        .ok_or_else(|| format!("{file}: bind \"{path}\" names no action"))?
                        .to_owned(),
                    o.get("args").cloned().unwrap_or(Value::Null),
                    o.get("every")
                        .and_then(Value::as_u64)
                        .map(|ms| Duration::from_millis(ms.max(100))),
                ),
                _ => {
                    return Err(format!(
                        "{file}: bind \"{path}\" is an action id or {{action, args, every}}"
                    ));
                }
            };
            self.binds.push(Bind {
                path: pointer(path),
                action,
                args,
                every,
                next: Some(now),
            });
        }
        Ok(())
    }

    /// Starts `action` off the page's thread; its answer comes as `call`.
    fn start(&mut self, call: Call, action: String, args: &Value) {
        let id = self.next_call;
        self.next_call = self.next_call.wrapping_add(1);
        self.calls.insert(id, call);
        let (host, inbox) = (Arc::clone(&self.services.host), self.services.inbox.clone());
        let args = if args.is_null() {
            String::new()
        } else {
            args.to_string()
        };
        architect_scripting::vm::spawn(move || {
            let result = futures::executor::block_on(host.run(action, args));
            let _ = inbox.send(Inbox::Answer {
                call: id,
                result: result.map_err(|e| e.to_string()),
            });
        });
    }

    fn set(&mut self, path: &str, value: Value) {
        if path == "/" || path.is_empty() {
            self.data.replace_all(value);
        } else {
            self.data.set(path, value);
        }
        self.dirty = true;
    }

    /// A Button's action: its name, its context read at `base`, and where
    /// its answer goes.
    fn action(&self, id: &str, base: &str) -> Result<(String, Value, Option<String>), String> {
        let c = self
            .components
            .get(id)
            .ok_or_else(|| format!("no component \"{id}\""))?;
        match c.action() {
            Some(Action::Event { event }) => {
                let cx = self.context(id, base);
                let mut args = Map::new();
                let mut keys: Vec<&String> = event.context.keys().collect();
                keys.sort();
                for key in keys {
                    if let Some(v) = event.context.get(key) {
                        args.insert(key.clone(), cx.data_context.resolve_dynamic_value(v));
                    }
                }
                // A2UI spells them `wantResponse` and `responsePath`.
                let raw = c.get_raw("action").and_then(|a| a.get("event"));
                let field = |camel: &str| raw.and_then(|e| e.get(camel));
                let wants = event.want_response
                    || field("wantResponse").and_then(Value::as_bool) == Some(true);
                let respond = wants
                    .then(|| {
                        event.response_path.clone().or_else(|| {
                            field("responsePath")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                    })
                    .flatten()
                    .map(|p| at(base, &p));
                Ok((event.name, Value::Object(args), respond))
            }
            Some(Action::FunctionCall { function_call }) => Err(format!(
                "\"{}\" is a local function; a page runs Action List actions",
                function_call.call
            )),
            None => Err(format!("\"{id}\" has no action")),
        }
    }

    fn context<'a>(&'a self, id: &str, base: &str) -> ComponentContext<'a> {
        ComponentContext::new(
            id.to_owned(),
            "page".to_owned(),
            &self.data,
            &self.components,
            &self.functions,
            base,
            None,
        )
    }

    fn on(&mut self, element: &mut Element, event: &str, handler: Handler) {
        let n = u32::try_from(self.handlers.len()).unwrap_or(u32::MAX);
        self.handlers.push(handler);
        element.on.push(Listener {
            event: event.to_owned(),
            handler: n,
        });
    }

    /// Component `id` at template path `base`, as elements.
    fn node(&mut self, id: &str, base: &str, depth: usize) -> Element {
        if depth > DEPTH {
            return placeholder(id, &format!("“{id}” nests past {DEPTH} deep: a cycle?"));
        }
        let Some(c) = self.components.get(id).cloned() else {
            // Not there (yet): A2UI draws what it has.
            return el("div", "min-height: 16px;");
        };
        let Some(def) = catalog::component(&c.component_type) else {
            return placeholder(id, &catalog::refusal(&c.component_type));
        };
        let kind = def.name;
        let mut node = match kind {
            "Text" => self.text(&c, base),
            "Row" | "Column" | "List" => self.stack(&c, base, depth),
            "Card" => {
                let mut card = el(
                    "div",
                    "padding: 12px 14px; border: 1px solid var(--border, #8884); border-radius: 10px; \
                     background: var(--card, transparent); color: var(--card-foreground, inherit);",
                );
                if let Some(child) = c.child() {
                    card.children.push(self.node(&child, base, depth + 1));
                }
                card
            }
            "Divider" => {
                let vertical = word(&c, "axis") == "vertical";
                el(
                    "div",
                    if vertical {
                        "align-self: stretch; width: 1px; background: var(--border, #8884);"
                    } else {
                        "height: 1px; background: var(--border, #8884); margin: 4px 0;"
                    },
                )
            }
            "Button" => self.button(&c, base, depth),
            "CheckBox" => self.checkbox(&c, base),
            "TextField" => self.field(&c, base),
            "Slider" => self.slider(&c, base),
            "ChoicePicker" => self.choices(&c, base),
            "Tabs" => self.tabs(&c, base, depth),
            "Image" => {
                let cx = self.context(&c.id, base);
                let s = |k: &str| string(&c, &cx, k);
                let fit = match word(&c, "fit") {
                    "" => "contain",
                    w => w,
                };
                let mut img = el("img", &format!("max-width: 100%; object-fit: {fit};"));
                attr(&mut img, "src", &s("url"));
                attr(&mut img, "alt", &s("description"));
                img
            }
            _ => placeholder(id, &catalog::refusal(kind)),
        };
        node.key = Some(format!("{id}{base}"));
        // Its catalog name, as an agent's surface marks it, and a class
        // for a selector or a style (`.a2ui-Button`).
        attr(&mut node, "data-a2ui", kind);
        attr(&mut node, "class", &format!("a2ui-{kind}"));
        node
    }

    fn text(&self, c: &ComponentModel, base: &str) -> Element {
        let cx = self.context(&c.id, base);
        let words = string(c, &cx, "text");
        let (tag, style) = match word(c, "variant") {
            "h1" => ("h1", "font-size: 26px; font-weight: 700; margin: 0;"),
            "h2" => ("h2", "font-size: 20px; font-weight: 650; margin: 0;"),
            "h3" => ("h3", "font-size: 16px; font-weight: 650; margin: 0;"),
            "h4" => ("h4", "font-size: 14px; font-weight: 650; margin: 0;"),
            "h5" => ("h5", "font-size: 13px; font-weight: 650; margin: 0;"),
            "caption" => (
                "span",
                "font-size: 12px; color: var(--muted-foreground, #888);",
            ),
            _ => ("span", ""),
        };
        let mut t = el(tag, style);
        t.children.push(text(&words));
        t
    }

    fn stack(&mut self, c: &ComponentModel, base: &str, depth: usize) -> Element {
        let row = c.component_type == "Row" || word(c, "direction") == "horizontal";
        let style = format!(
            "display: flex; flex-direction: {}; gap: 8px; justify-content: {}; align-items: {};",
            if row { "row" } else { "column" },
            flex(word(c, "justify")),
            match word(c, "align") {
                "" => "stretch",
                w => flex(w),
            },
        );
        let mut stack = el("div", &style);
        for (child, at) in self.children(c, base) {
            stack.children.push(self.node(&child, &at, depth + 1));
        }
        stack
    }

    /// A container's children: its ids, or its template over a bound
    /// list, each with its template path.
    fn children(&self, c: &ComponentModel, base: &str) -> Vec<(String, String)> {
        match c.children() {
            Some(ChildList::Static(ids)) => {
                ids.into_iter().map(|id| (id, base.to_owned())).collect()
            }
            Some(ChildList::Template { component_id, path }) => {
                let cx = self.context(&c.id, base);
                match cx.data_context.get(&path) {
                    Some(Value::Array(items)) => {
                        let at = cx.data_context.resolve_pointer(&path);
                        (0..items.len())
                            .map(|i| (component_id.clone(), format!("{at}/{i}")))
                            .collect()
                    }
                    _ => Vec::new(),
                }
            }
            None => Vec::new(),
        }
    }

    fn button(&mut self, c: &ComponentModel, base: &str, depth: usize) -> Element {
        let key = format!("{}{base}", c.id);
        let style = match word(c, "variant") {
            "primary" => {
                "background: var(--primary, #2563eb); color: var(--primary-foreground, #fff); border: none;"
            }
            "borderless" => "background: transparent; color: inherit; border: none;",
            _ => {
                "background: var(--secondary, #8882); color: var(--secondary-foreground, inherit); \
                 border: 1px solid var(--border, #8884);"
            }
        };
        let mut b = el(
            "button",
            &format!(
                "{style} padding: 6px 12px; border-radius: 8px; font: inherit; cursor: pointer;"
            ),
        );
        if let Some(child) = c.child() {
            b.children.push(self.node(&child, base, depth + 1));
        }
        self.on(
            &mut b,
            "click",
            Handler::Press {
                id: c.id.clone(),
                base: base.to_owned(),
            },
        );
        let mut wrap = el(
            "div",
            "display: flex; flex-direction: column; gap: 4px; align-items: flex-start;",
        );
        wrap.children.push(b);
        if let Some(said) = self.said.get(&key) {
            let mut note = el(
                "span",
                "font-size: 12px; color: var(--muted-foreground, #888);",
            );
            note.children.push(text(said));
            wrap.children.push(note);
        }
        wrap
    }

    fn checkbox(&mut self, c: &ComponentModel, base: &str) -> Element {
        let cx = self.context(&c.id, base);
        let label = string(c, &cx, "label");
        let on = c
            .get_property::<DynamicBoolean>("value")
            .is_some_and(|d| cx.data_context.resolve_dynamic_boolean(&d));
        let path = bound(c, "value", base);
        let mut row = el(
            "div",
            "display: flex; gap: 8px; align-items: center; cursor: pointer;",
        );
        let mut tick = el(
            "span",
            &format!(
                "display: inline-block; width: 16px; height: 16px; line-height: 16px; text-align: center; \
                 border-radius: 4px; font-size: 12px; border: 1px solid var(--border, #8884); {}",
                if on {
                    "background: var(--primary, #2563eb); color: var(--primary-foreground, #fff);"
                } else {
                    ""
                }
            ),
        );
        tick.children.push(text(if on { "✓" } else { "" }));
        row.children.push(tick);
        row.children.push(text(&label));
        attr(&mut row, "aria-checked", if on { "true" } else { "false" });
        if let Some(path) = path {
            self.on(&mut row, "click", Handler::Toggle { path, now: on });
        }
        row
    }

    fn field(&mut self, c: &ComponentModel, base: &str) -> Element {
        let cx = self.context(&c.id, base);
        let label = string(c, &cx, "label");
        let value = string(c, &cx, "value");
        let variant = word(c, "variant");
        let path = bound(c, "value", base);
        let mut wrap = el("label", "display: flex; flex-direction: column; gap: 4px;");
        let mut l = el(
            "span",
            "font-size: 12px; color: var(--muted-foreground, #888);",
        );
        l.children.push(text(&label));
        wrap.children.push(l);
        let long = variant == "longText";
        let mut input = el(
            if long { "textarea" } else { "input" },
            "padding: 6px 8px; border-radius: 6px; font: inherit; color: inherit; \
             background: var(--background, transparent); border: 1px solid var(--input, var(--border, #8884));",
        );
        attr(&mut input, "value", &value);
        match variant {
            "number" => attr(&mut input, "type", "number"),
            "obscured" => attr(&mut input, "type", "password"),
            _ => {}
        }
        if let Some(path) = path {
            self.on(
                &mut input,
                "input",
                Handler::Text {
                    path,
                    number: variant == "number",
                },
            );
        }
        wrap.children.push(input);
        wrap
    }

    fn slider(&mut self, c: &ComponentModel, base: &str) -> Element {
        let cx = self.context(&c.id, base);
        let label = string(c, &cx, "label");
        let n = |k: &str, or: f64| {
            c.get_property::<DynamicNumber>(k)
                .map_or(or, |d| cx.data_context.resolve_dynamic_number(&d))
        };
        let (value, min, max) = (n("value", 0.0), n("min", 0.0), n("max", 100.0));
        let path = bound(c, "value", base);
        let mut wrap = el("label", "display: flex; flex-direction: column; gap: 4px;");
        let mut l = el(
            "span",
            "font-size: 12px; color: var(--muted-foreground, #888);",
        );
        l.children
            .push(text(&format!("{label}: {}", number(value))));
        wrap.children.push(l);
        let mut input = el("input", "");
        attr(&mut input, "type", "range");
        attr(&mut input, "min", &number(min));
        attr(&mut input, "max", &number(max));
        attr(&mut input, "value", &number(value));
        if let Some(path) = path {
            self.on(&mut input, "input", Handler::Slide { path });
        }
        wrap.children.push(input);
        wrap
    }

    fn choices(&mut self, c: &ComponentModel, base: &str) -> Element {
        let cx = self.context(&c.id, base);
        let label = string(c, &cx, "label");
        let options: Vec<(String, String)> = c
            .get_raw("options")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
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
        let chosen = strings(c.get_raw("value").and_then(|v| match v.get("path") {
            Some(Value::String(p)) => cx.data_context.get(p),
            _ => Some(v.clone()),
        }));
        let many = word(c, "variant") == "multipleSelection";
        let path = bound(c, "value", base);
        let mut wrap = el("div", "display: flex; flex-direction: column; gap: 4px;");
        let mut l = el(
            "span",
            "font-size: 12px; color: var(--muted-foreground, #888);",
        );
        l.children.push(text(&label));
        wrap.children.push(l);
        let mut row = el("div", "display: flex; flex-wrap: wrap; gap: 6px;");
        for (label, value) in options {
            let on = chosen.contains(&value);
            let mut b = el(
                "button",
                &format!(
                    "padding: 4px 10px; border-radius: 999px; font: inherit; cursor: pointer; \
                     border: 1px solid var(--border, #8884); {}",
                    if on {
                        "background: var(--primary, #2563eb); color: var(--primary-foreground, #fff);"
                    } else {
                        "background: transparent; color: inherit;"
                    }
                ),
            );
            b.key = Some(value.clone());
            b.children.push(text(&label));
            if let Some(path) = &path {
                self.on(
                    &mut b,
                    "click",
                    Handler::Choose {
                        path: path.clone(),
                        value,
                        chosen: chosen.clone(),
                        many,
                    },
                );
            }
            row.children.push(b);
        }
        wrap.children.push(row);
        wrap
    }

    fn tabs(&mut self, c: &ComponentModel, base: &str, depth: usize) -> Element {
        let key = format!("{}{base}", c.id);
        let cx = self.context(&c.id, base);
        let tabs: Vec<(String, String)> = c
            .get_raw("tabs")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
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
        let path = bound(c, "activeTab", base);
        let active = match &path {
            Some(_) => c
                .get_property::<DynamicNumber>("activeTab")
                .map(|d| cx.data_context.resolve_dynamic_number(&d)),
            None => self.tabs.get(&key).map(|&i| i as f64),
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let shown = active.unwrap_or(0.0).max(0.0) as usize;
        let shown = shown.min(tabs.len().saturating_sub(1));
        let mut wrap = el("div", "display: flex; flex-direction: column; gap: 8px;");
        let mut bar = el(
            "div",
            "display: flex; gap: 4px; border-bottom: 1px solid var(--border, #8884);",
        );
        for (i, (title, _)) in tabs.iter().enumerate() {
            let mut t = el(
                "button",
                &format!(
                    "padding: 6px 10px; font: inherit; cursor: pointer; background: transparent; color: inherit; \
                     border: none; border-bottom: 2px solid {};",
                    if i == shown {
                        "var(--primary, #2563eb)"
                    } else {
                        "transparent"
                    }
                ),
            );
            t.key = Some(format!("tab{i}"));
            t.children.push(text(title));
            self.on(
                &mut t,
                "click",
                Handler::Tab {
                    key: key.clone(),
                    path: path.clone(),
                    index: i,
                },
            );
            bar.children.push(t);
        }
        wrap.children.push(bar);
        if let Some((_, child)) = tabs.get(shown) {
            let child = child.clone();
            wrap.children.push(self.node(&child, base, depth + 1));
        }
        wrap
    }

    /// Runs the bindings that are due; how soon the next one is.
    fn due(&mut self) -> Option<Duration> {
        let now = Instant::now();
        for i in 0..self.binds.len() {
            let Some(bind) = self.binds.get_mut(i) else {
                continue;
            };
            if bind.next.is_some_and(|at| at <= now) {
                bind.next = None;
                let (action, args) = (bind.action.clone(), bind.args.clone());
                self.start(Call::Bind(i), action, &args);
            }
        }
        self.binds
            .iter()
            .filter_map(|b| b.next)
            .min()
            .map(|at| at.saturating_duration_since(now))
    }
}

impl Engine for A2uiEngine {
    fn render(&mut self) -> Result<Vec<Element>, String> {
        self.dirty = false;
        self.handlers.clear();
        Ok(vec![self.node(ROOT, "", 0)])
    }

    fn event(&mut self, event: PageEvent) -> Result<(), String> {
        let Some(handler) = usize::try_from(event.handler)
            .ok()
            .and_then(|i| self.handlers.get(i))
            .cloned()
        else {
            return Ok(());
        };
        self.dirty = true;
        match handler {
            Handler::Press { id, base } => {
                let key = format!("{id}{base}");
                match self.action(&id, &base) {
                    Ok((action, args, respond)) => {
                        self.said.insert(key.clone(), format!("Running {action}…"));
                        let call = Call::Press {
                            key,
                            action: action.clone(),
                            respond,
                        };
                        self.start(call, action, &args);
                    }
                    Err(why) => {
                        self.said.insert(key, why.clone());
                        return Err(why);
                    }
                }
            }
            Handler::Text { path, number } => {
                let typed = event.value.unwrap_or_default();
                let value = if number {
                    typed
                        .trim()
                        .parse::<f64>()
                        .ok()
                        .and_then(serde_json::Number::from_f64)
                        .map_or(Value::Null, Value::Number)
                } else {
                    Value::String(typed)
                };
                self.set(&path, value);
            }
            Handler::Toggle { path, now } => self.set(&path, Value::Bool(!now)),
            Handler::Slide { path } => {
                if let Some(n) = event
                    .value
                    .and_then(|v| v.trim().parse::<f64>().ok())
                    .and_then(serde_json::Number::from_f64)
                {
                    self.set(&path, Value::Number(n));
                }
            }
            Handler::Choose {
                path,
                value,
                mut chosen,
                many,
            } => {
                if many {
                    if let Some(at) = chosen.iter().position(|c| *c == value) {
                        chosen.remove(at);
                    } else {
                        chosen.push(value);
                    }
                } else {
                    chosen = vec![value];
                }
                self.set(
                    &path,
                    Value::Array(chosen.into_iter().map(Value::String).collect()),
                );
            }
            Handler::Tab { key, path, index } => match path {
                Some(path) => self.set(&path, Value::from(index)),
                None => {
                    self.tabs.insert(key, index);
                }
            },
        }
        Ok(())
    }

    fn answer(&mut self, call: u32, result: Result<String, String>) -> Result<(), String> {
        let Some(call) = self.calls.remove(&call) else {
            return Ok(());
        };
        self.dirty = true;
        let value = |text: &str| serde_json::from_str::<Value>(text).unwrap_or(Value::Null);
        match call {
            Call::Bind(i) => {
                let Some(bind) = self.binds.get_mut(i) else {
                    return Ok(());
                };
                bind.next = bind.every.and_then(|d| Instant::now().checked_add(d));
                let path = bind.path.clone();
                match result {
                    Ok(text) => {
                        self.set(&path, value(&text));
                        Ok(())
                    }
                    Err(why) => Err(format!("bind {path}: {why}")),
                }
            }
            Call::Press {
                key,
                action,
                respond,
            } => match result {
                Ok(text) => {
                    self.said.remove(&key);
                    if let Some(path) = respond {
                        self.set(&path, value(&text));
                    }
                    // What it changed shows now: its bindings read again.
                    let now = Instant::now();
                    for (i, bind) in self.binds.iter_mut().enumerate() {
                        let out = self
                            .calls
                            .values()
                            .any(|c| matches!(c, Call::Bind(b) if *b == i));
                        if !out {
                            bind.next = Some(now);
                        }
                    }
                    Ok(())
                }
                Err(why) => {
                    self.said.insert(key, format!("{action}: {why}"));
                    Err(format!("{action}: {why}"))
                }
            },
        }
    }

    fn shared(&mut self, _event: &DataEvent) -> Result<(), String> {
        Ok(())
    }

    fn worker(&mut self, _message: &WorkerMessage) -> Result<(), String> {
        Ok(())
    }

    fn theme(&mut self, _theme: &ThemeSnapshot) -> Result<(), String> {
        // Its colours are the theme's variables: a new frame is enough.
        self.dirty = true;
        Ok(())
    }

    fn tick(&mut self) -> Result<Option<Duration>, String> {
        Ok(self.due())
    }

    fn dirty(&self) -> bool {
        self.dirty
    }
}

/// `path` as a JSON pointer (`/board`).
fn pointer(path: &str) -> String {
    if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

/// `path` (relative, or absolute) read at template path `base`; `""` is
/// `base` itself (a template item).
fn at(base: &str, path: &str) -> String {
    if path.is_empty() && !base.is_empty() {
        base.to_owned()
    } else if path.starts_with('/') || base.is_empty() {
        pointer(path)
    } else {
        format!("{base}/{path}")
    }
}

/// The data path a prop is bound to, if it is (`{"path": …}`).
fn bound(c: &ComponentModel, prop: &str, base: &str) -> Option<String> {
    let path = c.get_raw(prop)?.get("path")?.as_str()?;
    Some(at(base, path))
}

/// One of a prop's words, if the catalog allows it.
fn word<'a>(c: &'a ComponentModel, name: &str) -> &'a str {
    let allowed = catalog::component(&c.component_type)
        .and_then(|d| d.props.iter().find(|p| p.name == name))
        .and_then(|p| match p.prop {
            catalog::Prop::OneOf(words) => Some(words),
            _ => None,
        });
    c.get_raw(name)
        .and_then(Value::as_str)
        .filter(|w| allowed.is_some_and(|a| a.contains(w)))
        .unwrap_or("")
}

fn string(c: &ComponentModel, cx: &ComponentContext<'_>, name: &str) -> String {
    c.get_property::<DynamicString>(name)
        .map(|d| cx.data_context.resolve_dynamic_string(&d))
        .unwrap_or_default()
}

fn strings(value: Option<Value>) -> Vec<String> {
    match value {
        Some(Value::Array(list)) => list
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Some(Value::String(s)) => vec![s],
        _ => Vec::new(),
    }
}

fn number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{n:.0}")
    } else {
        n.to_string()
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

fn el(tag: &str, style: &str) -> Element {
    let mut e = Element {
        tag: tag.to_owned(),
        ..Element::default()
    };
    if !style.is_empty() {
        attr(&mut e, "style", style);
    }
    e
}

fn attr(e: &mut Element, name: &str, value: &str) {
    e.attrs.push(Attr {
        name: name.to_owned(),
        value: value.to_owned(),
    });
}

fn text(s: &str) -> Element {
    Element {
        tag: TEXT.to_owned(),
        text: Some(s.to_owned()),
        ..Element::default()
    }
}

/// What stands for a component the catalog does not draw.
fn placeholder(id: &str, why: &str) -> Element {
    let mut p = el(
        "div",
        "border: 1px dashed var(--border, #888); border-radius: 8px; padding: 8px 10px; \
         font-size: 12px; color: var(--muted-foreground, #888);",
    );
    attr(&mut p, "data-a2ui-placeholder", id);
    attr(&mut p, "class", "a2ui-placeholder");
    p.children.push(text(why));
    p
}
