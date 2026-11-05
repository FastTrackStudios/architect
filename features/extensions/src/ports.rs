//! MIDI ports, OSC endpoints and control surfaces, for extensions (#311;
//! `docs/content/architecture/extensions.md`, "MIDI, OSC and control
//! surfaces").
//!
//! - **Ports.** The app gives the host its ports ([`Host::with_ports`]: an
//!   `architect_input::PortBackend` — the system's MIDI, OSC over UDP,
//!   `architect_input::VirtualPorts` in a test). An extension lists them
//!   (`ports`), opens one its `capabilities.ports` names (`open_port`),
//!   sends (`port_send`) and closes it (`close_port`); what the port hears
//!   is a `port.message` event to it alone (`{port, kind, name, message}`),
//!   so a background extension sleeps until its device says something. A
//!   message no hook of its took (none, or a run still going) waits, the
//!   last [`INBOX`] of them, for `port_receive`.
//! - **Control surfaces.** The app offers targets
//!   ([`Host::with_targets`]: `architect_input::Targets`, each kind a range,
//!   a count and read/write/observe), which declares point
//!   [`SURFACES`]. An extension contributes a surface to it
//!   (`[[contribute."control.surfaces"]]`, an
//!   `architect_input::SurfaceDesc`); the host opens its port (one the
//!   contributor's `capabilities.ports` names), runs it
//!   (`architect_input::Surface`: banks, feedback, touch, takeover) and
//!   closes it when the item goes. [`Host::surfaces`] says how each is.
//!
//! Every port an extension opened closes when it unloads.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use architect_input::{
    Inbox, OpenRequest, Port, PortBackend, PortKind, PortMessage, Surface, SurfaceDesc, Targets,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::host::Host;
use crate::hostcall::HostError;

/// The event a port's message is, to the extension that opened it.
pub const PORT_MESSAGE: &str = "port.message";
/// The point control surfaces are contributed to.
pub const SURFACES: &str = "control.surfaces";
/// The most messages a port keeps for `port_receive`: past it, the oldest
/// go.
pub const INBOX: usize = 1024;
/// The most instances' values `targets` gives a kind.
pub const VALUES: u32 = 128;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One port an extension has open.
struct Opened {
    owner: String,
    kind: PortKind,
    name: String,
    port: Arc<dyn Port>,
    waiting: Arc<Mutex<VecDeque<PortMessage>>>,
}

/// A control surface an extension contributed, as it runs.
struct Running {
    item: Value,
    title: String,
    port_name: String,
    engine: Option<Arc<Mutex<Surface>>>,
    port: Arc<Mutex<Option<Box<dyn Port>>>>,
    error: Option<String>,
}

impl Drop for Running {
    fn drop(&mut self) {
        // Its port's inbox holds the slot it is in: closed here, not when
        // the last of the two goes (never).
        lock(&self.port).take();
    }
}

/// How one control surface is (`Host::surfaces`, the `surfaces` call).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SurfaceStatus {
    /// Its id, as its extension gave it.
    pub id: String,
    /// The extension that contributed it.
    pub from: String,
    pub title: String,
    /// `midi:<name>` or `osc:<host>:<port>`.
    pub port: String,
    pub bank: u32,
    pub banks: u32,
    /// Why it is not running (a bad description, a port it may not open,
    /// one that would not open).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What the host keeps for ports and surfaces.
#[derive(Default)]
pub(crate) struct PortsState {
    backend: Mutex<Option<Arc<dyn PortBackend>>>,
    targets: Mutex<Option<Targets>>,
    open: Mutex<BTreeMap<String, Opened>>,
    surfaces: Mutex<BTreeMap<(String, String), Running>>,
    next: AtomicU64,
}

impl PortsState {
    /// Extension `id`'s ports close (it unloaded).
    pub(crate) fn forget(&self, id: &str) {
        lock(&self.open).retain(|_, o| o.owner != id);
    }
}

/// `midi:<name>` / `osc:<host>:<port>` as a kind and a name.
fn split_port(port: &str) -> Option<(PortKind, &str)> {
    let (kind, name) = port.split_once(':')?;
    let name = name.trim();
    (!name.is_empty()).then_some((PortKind::parse(kind)?, name))
}

impl Host {
    fn ports_state(&self) -> &PortsState {
        self.services().ports()
    }

    /// Opens extensions' ports through `backend` (`architect_input::Ports`
    /// for a MIDI backend and an OSC one together). Without one, a port
    /// call answers `unsupported`.
    #[must_use]
    pub fn with_ports(self, backend: impl PortBackend + 'static) -> Self {
        self.set_ports(Arc::new(backend));
        self
    }

    /// [`Self::with_ports`] on a host already shared: running surfaces
    /// open their ports again through it.
    pub fn set_ports(&self, backend: Arc<dyn PortBackend>) {
        *lock(&self.ports_state().backend) = Some(backend);
        lock(&self.ports_state().surfaces).clear();
        self.sync_surfaces();
    }

    /// Offers `targets` to control surfaces, and declares their point
    /// ([`SURFACES`]).
    #[must_use]
    pub fn with_targets(self, targets: Targets) -> Self {
        self.set_targets(targets);
        self
    }

    /// [`Self::with_targets`] on a host already shared.
    pub fn set_targets(&self, targets: Targets) {
        let weak = self.downgrade();
        targets.listen(move |kind, index, _value| {
            if let Some(host) = weak.upgrade() {
                host.target_changed(kind, index);
            }
        });
        *lock(&self.ports_state().targets) = Some(targets);
        let schema = r#"{"type": "object", "required": ["id", "port", "controls"]}"#;
        let _ = self.points().declare(
            "control",
            &[crate::interop::PointDecl {
                id: SURFACES.to_owned(),
                title: "Control surfaces".to_owned(),
                description: "A device's faders, knobs, buttons, encoders, displays and LEDs, \
                              mapped to the app's targets (architect_input::SurfaceDesc)"
                    .to_owned(),
                schema: Some(toml::Value::String(schema.to_owned())),
                deliver: None,
            }],
        );
        self.sync_surfaces();
    }

    /// The targets the app offers surfaces.
    #[must_use]
    pub fn targets(&self) -> Option<Targets> {
        lock(&self.ports_state().targets).clone()
    }

    /// Every control surface contributed, and how it is.
    #[must_use]
    pub fn surfaces(&self) -> Vec<SurfaceStatus> {
        let targets = self.targets().unwrap_or_default();
        lock(&self.ports_state().surfaces)
            .iter()
            .map(|((from, id), r)| {
                let (bank, banks) = r.engine.as_ref().map_or((0, 0), |e| {
                    let e = lock(e);
                    (e.bank(), e.banks(&targets))
                });
                SurfaceStatus {
                    id: id.clone(),
                    from: from.clone(),
                    title: r.title.clone(),
                    port: r.port_name.clone(),
                    bank,
                    banks,
                    error: r.error.clone(),
                }
            })
            .collect()
    }

    /// Extension `from`'s surface `id` on bank `bank` (within its banks):
    /// the bank it is on.
    ///
    /// # Errors
    ///
    /// It has no such surface running.
    pub fn set_surface_bank(&self, from: &str, id: &str, bank: u32) -> Result<u32, String> {
        let targets = self.targets().unwrap_or_default();
        let (engine, port, error) = lock(&self.ports_state().surfaces)
            .get(&(from.to_owned(), id.to_owned()))
            .map(|r| (r.engine.clone(), Arc::clone(&r.port), r.error.clone()))
            .ok_or_else(|| format!("{from} has no control surface {id}"))?;
        let engine = engine.ok_or_else(|| error.unwrap_or_default())?;
        let mut e = lock(&engine);
        let out = e.set_bank(&targets, bank);
        if let Some(p) = lock(&port).as_ref() {
            for m in &out {
                let _ = p.send(m);
            }
        }
        Ok(e.bank())
    }

    /// Instance `index` of target kind `kind` changed: each surface on it
    /// shows it (but one moving it now, whose fader is where it is).
    pub fn target_changed(&self, kind: &str, index: u32) {
        let Some(targets) = self.targets() else {
            return;
        };
        let running: Vec<_> = lock(&self.ports_state().surfaces)
            .values()
            .filter_map(|r| Some((r.engine.clone()?, Arc::clone(&r.port))))
            .collect();
        for (engine, port) in running {
            // Locked: it is the surface writing this value (a target that
            // tells of its own writes) — its control is already there.
            let Ok(mut e) = engine.try_lock() else {
                continue;
            };
            let out = e.changed(&targets, kind, index);
            if let Some(p) = lock(&port).as_ref() {
                for m in &out {
                    let _ = p.send(m);
                }
            }
        }
    }

    /// Opens and closes control surfaces to match [`SURFACES`]' items.
    pub(crate) fn sync_surfaces(&self) {
        let state = self.ports_state();
        let Some(targets) = self.targets() else {
            return;
        };
        let items: Vec<(String, Value)> = self
            .points()
            .items(SURFACES)
            .into_iter()
            .map(|i| (i.from, i.value))
            .collect();
        let mut surfaces = lock(&state.surfaces);
        surfaces.retain(|(from, id), r| {
            items.iter().any(|(f, v)| {
                f == from && v.get("id").and_then(Value::as_str) == Some(id) && *v == r.item
            })
        });
        for (from, item) in items {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let key = (from.clone(), id);
            if surfaces.contains_key(&key) {
                continue;
            }
            let running = self.start_surface(&from, item, &targets);
            if let Some(why) = &running.error {
                tracing::warn!(ext.id = %from, surface.id = %key.1, error = %why, "a control surface did not start");
                self.logs().push(
                    &from,
                    crate::logs::Level::Warn,
                    "surface",
                    format!("{}: {why}", key.1),
                );
            }
            surfaces.insert(key, running);
        }
    }

    fn start_surface(&self, from: &str, item: Value, targets: &Targets) -> Running {
        let mut running = Running {
            title: String::new(),
            port_name: String::new(),
            item: item.clone(),
            engine: None,
            port: Arc::new(Mutex::new(None)),
            error: None,
        };
        let desc: SurfaceDesc = match serde_json::from_value(item) {
            Ok(d) => d,
            Err(e) => {
                running.error = Some(format!("not a control surface: {e}"));
                return running;
            }
        };
        running.title.clone_from(&desc.title);
        running.port_name.clone_from(&desc.port);
        let Some((kind, name)) = split_port(&desc.port) else {
            running.error = Some(format!(
                "port {:?}: midi:<name> or osc:<host>:<port>",
                desc.port
            ));
            return running;
        };
        let name = name.to_owned();
        let listen = desc.listen;
        if !self
            .capabilities(from)
            .is_some_and(|c| c.may_open_port(kind.name(), &name, listen))
        {
            running.error = Some(format!(
                "it may not open {kind} port {name} (capabilities.ports)"
            ));
            return running;
        }
        let engine = match Surface::new(desc) {
            Ok(e) => Arc::new(Mutex::new(e)),
            Err(e) => {
                running.error = Some(e);
                return running;
            }
        };
        let Some(backend) = lock(&self.ports_state().backend).clone() else {
            running.error = Some("this app has no ports".to_owned());
            return running;
        };
        let (e, p, t) = (
            Arc::clone(&engine),
            Arc::clone(&running.port),
            targets.clone(),
        );
        let inbox: Inbox = Arc::new(move |m: PortMessage| {
            let out = lock(&e).input(&t, &m);
            if let Some(port) = lock(&p).as_ref() {
                for m in &out {
                    let _ = port.send(m);
                }
            }
        });
        match backend.open(&OpenRequest { kind, name, listen }, inbox) {
            Ok(port) => {
                let shown = lock(&engine).refresh(targets);
                for m in &shown {
                    let _ = port.send(m);
                }
                *lock(&running.port) = Some(port);
                running.engine = Some(engine);
            }
            Err(e) => running.error = Some(e.to_string()),
        }
        running
    }

    // ── The host service's port calls ───────────────────────────────

    fn backend(&self) -> Result<Arc<dyn PortBackend>, HostError> {
        lock(&self.ports_state().backend)
            .clone()
            .ok_or_else(|| HostError::Unsupported {
                what: "MIDI ports and OSC endpoints: this app has none (Host::with_ports)"
                    .to_owned(),
            })
    }

    /// The ports there are, of `kind` or every one.
    pub(crate) fn list_ports(&self, kind: Option<&str>) -> Result<Value, HostError> {
        let ports: Vec<_> = self
            .backend()?
            .list()
            .into_iter()
            .filter(|p| kind.is_none_or(|k| p.kind.name() == k))
            .collect();
        Ok(serde_json::to_value(ports).unwrap_or(Value::Null))
    }

    /// Opens a port for extension `owner`, its capability already checked.
    pub(crate) fn open_port(
        &self,
        owner: &str,
        kind: &str,
        name: &str,
        listen: Option<u16>,
    ) -> Result<Value, HostError> {
        let kind = PortKind::parse(kind).ok_or_else(|| HostError::Failed {
            message: format!("{kind:?} is not a kind of port"),
        })?;
        let backend = self.backend()?;
        let state = self.ports_state();
        let handle = format!(
            "port{}",
            state.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
        );
        let waiting: Arc<Mutex<VecDeque<PortMessage>>> = Arc::default();
        let inbox: Inbox = {
            let (weak, waiting) = (self.downgrade(), Arc::clone(&waiting));
            let (owner, handle, name) = (owner.to_owned(), handle.clone(), name.to_owned());
            Arc::new(move |m: PortMessage| {
                let detail = json!({
                    "port": handle,
                    "kind": kind,
                    "name": name,
                    "message": m.to_json(),
                });
                let started = weak
                    .upgrade()
                    .map_or(0, |host| host.emit_to(Some(&owner), PORT_MESSAGE, &detail));
                // A hook took it; or it waits for `port_receive`.
                if started == 0 {
                    let mut w = lock(&waiting);
                    if w.len() >= INBOX {
                        w.pop_front();
                    }
                    w.push_back(m);
                }
            })
        };
        let port = backend
            .open(
                &OpenRequest {
                    kind,
                    name: name.to_owned(),
                    listen,
                },
                inbox,
            )
            .map_err(|e| HostError::Failed {
                message: e.to_string(),
            })?;
        lock(&state.open).insert(
            handle.clone(),
            Opened {
                owner: owner.to_owned(),
                kind,
                name: name.to_owned(),
                port: Arc::from(port),
                waiting,
            },
        );
        tracing::info!(ext.id = %owner, port.kind = %kind, port.name = %name, port.handle = %handle, "a port opened");
        Ok(json!({ "port": handle, "kind": kind, "name": name }))
    }

    fn not_its(port: &str) -> HostError {
        HostError::NotFound {
            what: format!("port {port} (one it opened)"),
        }
    }

    /// Sends `message` out of `owner`'s port `port`.
    pub(crate) fn port_send(
        &self,
        owner: &str,
        port: &str,
        message: &Value,
    ) -> Result<Value, HostError> {
        let (kind, name, out) = lock(&self.ports_state().open)
            .get(port)
            .filter(|o| o.owner == owner)
            .map(|o| (o.kind, o.name.clone(), Arc::clone(&o.port)))
            .ok_or_else(|| Self::not_its(port))?;
        let message = PortMessage::from_json(kind, message).map_err(|e| HostError::Failed {
            message: e.to_string(),
        })?;
        out.send(&message).map_err(|e| HostError::Failed {
            message: format!("{kind} {name}: {e}"),
        })?;
        Ok(Value::Null)
    }

    /// What `owner`'s port `port` heard and no hook took, at most `max`.
    pub(crate) fn port_receive(
        &self,
        owner: &str,
        port: &str,
        max: Option<u32>,
    ) -> Result<Value, HostError> {
        let waiting = lock(&self.ports_state().open)
            .get(port)
            .filter(|o| o.owner == owner)
            .map(|o| Arc::clone(&o.waiting))
            .ok_or_else(|| Self::not_its(port))?;
        let mut w = lock(&waiting);
        let n = max.map_or(w.len(), |m| {
            usize::try_from(m).unwrap_or(usize::MAX).min(w.len())
        });
        Ok(Value::Array(w.drain(..n).map(|m| m.to_json()).collect()))
    }

    /// Closes `owner`'s port `port`: whether it had it open.
    pub(crate) fn close_port(&self, owner: &str, port: &str) -> bool {
        let mut open = lock(&self.ports_state().open);
        if open.get(port).is_some_and(|o| o.owner == owner) {
            open.remove(port);
            true
        } else {
            false
        }
    }

    /// The ports extension `owner` has open: `[{port, kind, name}]`.
    #[must_use]
    pub fn open_ports(&self, owner: &str) -> Vec<Value> {
        lock(&self.ports_state().open)
            .iter()
            .filter(|(_, o)| o.owner == owner)
            .map(|(h, o)| json!({ "port": h, "kind": o.kind, "name": o.name }))
            .collect()
    }

    /// The target kinds, as JSON: each with its instances' `values` (the
    /// first [`VALUES`]).
    pub(crate) fn target_kinds(&self) -> Value {
        let Some(targets) = self.targets() else {
            return Value::Array(Vec::new());
        };
        Value::Array(
            targets
                .kinds()
                .into_iter()
                .map(|k| {
                    let values: Vec<Option<f64>> = targets.get(&k.id).map_or_else(Vec::new, |t| {
                        (0..k.count.min(VALUES)).map(|i| t.read(i)).collect()
                    });
                    let mut v = serde_json::to_value(&k).unwrap_or(Value::Null);
                    if let Value::Object(map) = &mut v {
                        map.insert("values".to_owned(), json!(values));
                    }
                    v
                })
                .collect(),
        )
    }

    /// Extension `owner`'s surfaces, as JSON.
    pub(crate) fn surfaces_of(&self, owner: &str) -> Value {
        serde_json::to_value(
            self.surfaces()
                .into_iter()
                .filter(|s| s.from == owner)
                .collect::<Vec<_>>(),
        )
        .unwrap_or(Value::Null)
    }
}
