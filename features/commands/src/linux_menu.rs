//! The menu bar on a Linux desktop (feature `native`, Linux): the
//! [`MenuBar`] exported on the session bus as `com.canonical.dbusmenu`
//! (Session's `bus.rs`, moved here), where a shell's global menu shows
//! it. When no shell takes it (no session bus, or no
//! `com.canonical.AppMenu.Registrar` took the window), the app's
//! [`InWindowMenus`](crate::InWindowMenus) draws it instead.
//!
//! The commands live on the UI's thread and the bus on a thread of its
//! own: the UI resolves the bar (when the commands change, or the Action
//! Lists' events move on) and hands the numbered tree over, the bus tells
//! the shell the layout changed, and a click comes back as a [`Pick`] the
//! UI's loop runs.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use dioxus::prelude::*;
use zbus::fdo;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{Array, Dict, ObjectPath, OwnedValue, Signature, StructureBuilder, Value};

use crate::dbusmenu::{Prop, props};
use crate::menu::{MenuBar, Pick, Tree, without_platform};
use crate::ui::try_use_commands;

/// Where the menu is served: the path Qt's menubars use, which is what a
/// shell looks for.
pub const PATH: &str = "/MenuBar/1";

/// How often a click, or an Action List event, is looked for.
const POLL: Duration = Duration::from_millis(30);

/// What the bus thread knows of the shell.
const SERVING: u8 = 0;
/// A registrar took the window's menu: the shell shows it.
const TAKEN: u8 = 1;
/// Served, but nothing said it shows it; or not served at all.
const NOT_TAKEN: u8 = 2;

/// The tree a [`DbusMenu`] serves, and its revision (bumped with each new
/// tree).
pub struct Shared {
    pub tree: Tree,
    pub revision: u32,
}

impl Shared {
    /// Serves `tree` from now on; the new revision to announce
    /// ([`DbusMenu::announce`]) when it is not the tree served already.
    pub fn set(&mut self, tree: Tree) -> Option<u32> {
        if self.tree == tree {
            return None;
        }
        self.tree = tree;
        self.revision = self.revision.wrapping_add(1);
        Some(self.revision)
    }
}

/// The UI's end of the bus thread.
struct Bus {
    shared: Arc<Mutex<Shared>>,
    /// A new revision to announce; dropped with the component, which ends
    /// the thread.
    announce: tokio::sync::mpsc::UnboundedSender<u32>,
    picks: mpsc::Receiver<Pick>,
    state: Arc<AtomicU8>,
}

/// Exports `bar` on the session bus while mounted (once, at the root).
#[component]
pub fn NativeMenus(bar: MenuBar) -> Element {
    let commands = try_use_commands();
    let bus = use_hook(|| Rc::new(start()));
    if let Some(c) = commands {
        let tree = Tree::new(&without_platform(&bar.resolve(&c.all())));
        let revision = bus
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set(tree);
        if let Some(revision) = revision {
            let _ = bus.announce.send(revision);
        }
    }
    use_future(move || {
        let bus = bus.clone();
        async move {
            let mut heard = commands.map_or(0, |c| c.heard());
            loop {
                futures_timer::Delay::new(POLL).await;
                let Some(c) = commands else {
                    continue;
                };
                while let Ok(pick) = bus.picks.try_recv() {
                    let _ = c.run_pick(&pick);
                }
                match bus.state.load(Ordering::Acquire) {
                    TAKEN => c.set_global_menu(true),
                    NOT_TAKEN => c.set_global_menu(false),
                    _ => {}
                }
                let now = c.heard();
                if now != heard {
                    heard = now;
                    c.refresh();
                }
            }
        }
    });
    use_drop(move || {
        if let Some(c) = commands {
            c.set_global_menu(false);
        }
    });
    rsx! {}
}

/// Starts the bus thread, serving an empty bar until the UI hands it one.
fn start() -> Bus {
    let shared = Arc::new(Mutex::new(Shared {
        tree: Tree::new(&[]),
        revision: 1,
    }));
    let (announce, announced) = tokio::sync::mpsc::unbounded_channel();
    let (picked, picks) = mpsc::channel();
    let state = Arc::new(AtomicU8::new(SERVING));
    let serve = Serve {
        shared: Arc::clone(&shared),
        announced,
        picked,
        state: Arc::clone(&state),
    };
    let spawned = std::thread::Builder::new()
        .name("app-menu".into())
        .spawn(move || serve.run());
    if let Err(error) = spawned {
        state.store(NOT_TAKEN, Ordering::Release);
        tracing::warn!(app_menu.outcome = "no_thread", error = %error, "app menu: not served");
    }
    Bus {
        shared,
        announce,
        picks,
        state,
    }
}

/// The bus thread's half.
struct Serve {
    shared: Arc<Mutex<Shared>>,
    announced: tokio::sync::mpsc::UnboundedReceiver<u32>,
    picked: mpsc::Sender<Pick>,
    state: Arc<AtomicU8>,
}

impl Serve {
    /// Serves until the UI's end is dropped; with no session bus it says
    /// so once and gives up (the in-window bar shows instead).
    fn run(self) {
        let state = Arc::clone(&self.state);
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                state.store(NOT_TAKEN, Ordering::Release);
                tracing::warn!(app_menu.outcome = "no_runtime", error = %error, "app menu: not served");
                return;
            }
        };
        let outcome = runtime.block_on(self.serve());
        if let Err(error) = outcome {
            state.store(NOT_TAKEN, Ordering::Release);
            tracing::warn!(app_menu.outcome = "not_served", error = %error, "app menu: not served");
        }
    }

    async fn serve(self) -> zbus::Result<()> {
        let Self {
            shared,
            mut announced,
            picked,
            state,
        } = self;
        let connection = zbus::Connection::session().await?;
        connection
            .object_server()
            .at(PATH, DbusMenu::new(Arc::clone(&shared), picked))
            .await?;
        let taken = register(&connection).await;
        state.store(if taken { TAKEN } else { NOT_TAKEN }, Ordering::Release);
        tracing::info!(
            app_menu.outcome = "served",
            app_menu.path = PATH,
            app_menu.registrar = taken,
            "app menu served"
        );
        let emitter = SignalEmitter::new(&connection, PATH)?;
        while let Some(revision) = announced.recv().await {
            DbusMenu::announce(&emitter, revision).await?;
        }
        Ok(())
    }
}

/// Tell a registrar, if there is one, which object is the window's menu
/// (window id 0: Wayland has none to give; a shell matches by pid).
/// Whether one took it.
async fn register(connection: &zbus::Connection) -> bool {
    let Ok(path) = ObjectPath::try_from(PATH) else {
        return false;
    };
    connection
        .call_method(
            Some("com.canonical.AppMenu.Registrar"),
            "/com/canonical/AppMenu/Registrar",
            Some("com.canonical.AppMenu.Registrar"),
            "RegisterWindow",
            &(0_u32, path),
        )
        .await
        .is_ok()
}

/// A node as `GetLayout` gives it: `(ia{sv}av)`.
type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

/// A numbered menu as `com.canonical.dbusmenu`: the object a shell talks to.
///
/// It is served at a path of the app's connection. The menu bar's is at
/// [`PATH`]; a tray item (`architect-presence`'s `StatusNotifierItem`)
/// serves one of its own. A click comes back as a [`Pick`] on `picked`.
pub struct DbusMenu {
    shared: Arc<Mutex<Shared>>,
    picked: mpsc::Sender<Pick>,
}

impl DbusMenu {
    /// Serves what `shared` holds; picks go to `picked`.
    pub const fn new(shared: Arc<Mutex<Shared>>, picked: mpsc::Sender<Pick>) -> Self {
        Self { shared, picked }
    }

    /// Tells the shell the menu at `emitter`'s path is now `revision`.
    pub async fn announce(emitter: &SignalEmitter<'_>, revision: u32) -> zbus::Result<()> {
        Self::layout_updated(emitter, revision, 0).await
    }

    /// The tree and its revision now, copied out so no lock is held while
    /// a reply is built.
    fn now(&self) -> (Tree, u32) {
        let shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        (shared.tree.clone(), shared.revision)
    }

    /// Hand the item `id`'s pick to the UI, if it is a row that can run;
    /// `Err` when there is no such node.
    fn click(&self, tree: &Tree, id: i32) -> fdo::Result<()> {
        exists(tree, id)?;
        if let Some(pick) = tree.pick(id) {
            let _ = self.picked.send(pick.clone());
        }
        Ok(())
    }
}

fn exists(tree: &Tree, id: i32) -> fdo::Result<&Tree> {
    tree.find(id)
        .ok_or_else(|| fdo::Error::InvalidArgs(format!("no menu item {id}")))
}

#[expect(
    clippy::needless_pass_by_value,
    clippy::unused_self,
    reason = "the D-Bus method's shape: zbus decodes owned arguments and calls through `self`"
)]
#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    /// The tree under `parent_id`, `recursion_depth` levels down (-1:
    /// all of it), with `property_names` (none: every one).
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> fdo::Result<(u32, Layout)> {
        let (tree, revision) = self.now();
        let node = exists(&tree, parent_id)?;
        Ok((revision, layout(node, recursion_depth, &property_names)))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let (tree, _) = self.now();
        let nodes: Vec<&Tree> = if ids.is_empty() {
            tree.walk().collect()
        } else {
            ids.iter().filter_map(|id| tree.find(*id)).collect()
        };
        nodes
            .into_iter()
            .map(|node| (node.id, wanted_dict(node, &property_names)))
            .collect()
    }

    fn get_property(&self, id: i32, name: &str) -> fdo::Result<OwnedValue> {
        let (tree, _) = self.now();
        let node = exists(&tree, id)?;
        props(node)
            .into_iter()
            .find(|(key, _)| *key == name)
            .and_then(|(_, prop)| owned(prop_value(prop)))
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("menu item {id} has no {name}")))
    }

    /// `clicked` runs the item; anything else (`hovered`, `opened`,
    /// `closed`) needs nothing.
    fn event(&self, id: i32, event_id: &str, data: OwnedValue, timestamp: u32) -> fdo::Result<()> {
        // What was clicked with, and when: nothing a click here needs.
        drop((data, timestamp));
        let (tree, _) = self.now();
        if event_id == "clicked" {
            self.click(&tree, id)
        } else {
            exists(&tree, id).map(|_| ())
        }
    }

    /// Each of `events` as [`Self::event`]; the ids that were not there.
    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        let (tree, _) = self.now();
        events
            .into_iter()
            .filter_map(|(id, event_id, _, _)| {
                let done = if event_id == "clicked" {
                    self.click(&tree, id)
                } else {
                    exists(&tree, id).map(|_| ())
                };
                done.is_err().then_some(id)
            })
            .collect()
    }

    /// Nothing to bring up to date before a menu opens: the UI hands a new
    /// tree over as soon as anything in it changes, and says so.
    fn about_to_show(&self, id: i32) -> fdo::Result<bool> {
        let (tree, _) = self.now();
        exists(&tree, id).map(|_| false)
    }

    /// No updates needed; the ids that were not there.
    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        let (tree, _) = self.now();
        let missing = ids
            .into_iter()
            .filter(|id| tree.find(*id).is_none())
            .collect();
        (Vec::new(), missing)
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    const fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".to_owned()
    }

    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".to_owned()
    }

    #[zbus(property)]
    const fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }
}

/// `node` as `GetLayout` gives it, `depth` levels of children down.
fn layout(node: &Tree, depth: i32, names: &[String]) -> Layout {
    let children = child_nodes(node, depth)
        .filter_map(|(child, deeper)| owned(Value::Structure(structure(child, deeper, names)?)))
        .collect();
    (node.id, wanted_dict(node, names), children)
}

/// The children a layout `depth` levels deep includes, each with the depth
/// left under it.
fn child_nodes(node: &Tree, depth: i32) -> impl Iterator<Item = (&Tree, i32)> {
    let deeper = if depth < 0 {
        -1
    } else {
        depth.saturating_sub(1)
    };
    node.children
        .iter()
        .filter(move |_| depth != 0)
        .map(move |child| (child, deeper))
}

/// A child of a layout: the same `(ia{sv}av)`, as a structure whose
/// props and children are variants.
fn structure(
    node: &Tree,
    depth: i32,
    names: &[String],
) -> Option<zbus::zvariant::Structure<'static>> {
    let mut dict = Dict::new(&Signature::Str, &Signature::Variant);
    for (key, prop) in wanted(node, names) {
        dict.append(Value::from(key), Value::Value(Box::new(prop_value(prop))))
            .ok()?;
    }
    let mut children = Array::new(&Signature::Variant);
    for (child, deeper) in child_nodes(node, depth) {
        let child = structure(child, deeper, names)?;
        children
            .append(Value::Value(Box::new(Value::Structure(child))))
            .ok()?;
    }
    StructureBuilder::new()
        .add_field(node.id)
        .append_field(Value::Dict(dict))
        .append_field(Value::Array(children))
        .build()
        .ok()
}

/// A node's properties as a dbusmenu dict, only `names` (none: all).
fn wanted_dict(node: &Tree, names: &[String]) -> HashMap<String, OwnedValue> {
    wanted(node, names)
        .into_iter()
        .filter_map(|(key, prop)| Some((key.to_owned(), owned(prop_value(prop))?)))
        .collect()
}

/// A node's properties among `names` (none: all).
fn wanted(node: &Tree, names: &[String]) -> Vec<(&'static str, Prop)> {
    props(node)
        .into_iter()
        .filter(|(key, _)| names.is_empty() || names.iter().any(|n| n == key))
        .collect()
}

fn prop_value(prop: Prop) -> Value<'static> {
    match prop {
        Prop::Str(s) => Value::from(s),
        Prop::Bool(b) => Value::from(b),
        Prop::Int(i) => Value::from(i),
        Prop::Keys(keys) => Value::from(keys),
    }
}

/// A value without borrows; `None` only for a file descriptor, which no
/// menu property is.
fn owned(value: Value<'static>) -> Option<OwnedValue> {
    OwnedValue::try_from(value).ok()
}
