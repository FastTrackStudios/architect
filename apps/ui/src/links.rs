//! Deep links into actions (#186).
//!
//! `architect://<route>?<args>` and the universal link `https://<the
//! app's domain>/a/<route>?<args>` run one of the app's actions, from the
//! OS (the iPhone's Shortcuts, a widget, a link in another app, a second
//! launch on a desk).
//!
//! Only a **declared** route runs: a link never names an action id
//! itself, so a web page cannot drive the app through it. A route names
//! the action and the arguments it fixes ([`Route`]); the link's query
//! fills the rest, as strings (`args=<a JSON object>` for anything else).
//! The app declares [`ROUTES`]; a distribution or an extension's crate
//! adds its own with [`declare`].
//!
//! A link runs as a caller from outside (`Principal::Service { name:
//! "link" }`) that asks the person before any write
//! (`ActionCaller::confirming_writes`): the app's confirmation sheet
//! shows what the link would do, and it runs only on their yes. A
//! destructive action is never run from a link, and an action the
//! caller may not run is refused as any outside caller's is.
//!
//! Share links (`architect://open?url=…`, `https://<node>/s/<code>/`) are
//! the Shared tab's (`crate::guest`), not routes.

use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use architect::action::{ActionCaller, ActionError, ActionList, Effect, Principal};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use serde_json::{Map, Value};

/// The app's scheme.
pub const SCHEME: &str = architect_spaces::guest::APP_SCHEME;

/// The path a universal link's route is under (`https://<domain>/a/<route>`).
pub const UNIVERSAL_PREFIX: &str = "/a/";

/// The caller a link runs as (`link:<scheme or domain>`).
pub const LINK_CALLER: &str = "link";

/// The domains a universal link (`https://<domain>/a/<route>`) may come
/// from: the app's own (`Architect.entitlements`' associated domains). A
/// distribution on a node of its own adds its with [`declare_domain`].
pub const DOMAINS: [&str; 1] = ["architect.fasttrackstudio.app"];

fn domains() -> &'static Mutex<Vec<String>> {
    static D: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(DOMAINS.iter().map(|d| (*d).to_owned()).collect()))
}

/// Adds `host` to the domains a universal link may come from.
pub fn declare_domain(host: &str) {
    let host = host.trim().to_ascii_lowercase();
    let mut d = domains().lock().unwrap_or_else(PoisonError::into_inner);
    if !d.contains(&host) {
        d.push(host);
    }
}

/// One declared route: the link's path, the action it runs and the
/// arguments it fixes (a link's query never overrides them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// `capture` for `architect://capture`, `capture/add` for
    /// `architect://capture/add`.
    pub path: String,
    pub action: String,
    /// A JSON object.
    pub args: Value,
}

impl Route {
    #[must_use]
    pub fn new(path: &str, action: &str) -> Self {
        Self {
            path: path.to_owned(),
            action: action.to_owned(),
            args: Value::Object(Map::new()),
        }
    }

    #[must_use]
    pub fn with_args(mut self, args: Value) -> Self {
        self.args = args;
        self
    }
}

/// The app's own routes: `architect://capture` opens the composer
/// (`?text=` fills it, `?target=` picks what it becomes);
/// `architect://capture/add?text=…` captures at once (a write: the person
/// is asked first).
pub const ROUTES: [(&str, &str); 2] = [("capture", "CAPTURE_OPEN"), ("capture/add", "CAPTURE_ADD")];

fn table() -> &'static Mutex<Vec<Route>> {
    static TABLE: OnceLock<Mutex<Vec<Route>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        Mutex::new(
            ROUTES
                .iter()
                .map(|(path, action)| {
                    let route = Route::new(path, action);
                    if *action == "CAPTURE_ADD" {
                        route.with_args(serde_json::json!({ "source": "link" }))
                    } else {
                        route
                    }
                })
                .collect(),
        )
    })
}

/// Adds `route` (or replaces the one at its path).
pub fn declare(route: Route) {
    let mut t = table().lock().unwrap_or_else(PoisonError::into_inner);
    t.retain(|r| r.path != route.path);
    t.push(route);
}

/// Every route declared now.
#[must_use]
pub fn routes() -> Vec<Route> {
    table()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Why a link does not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a link of the app's (another scheme, or a web link outside
    /// `/a/`), or a share link (the Shared tab's).
    NotARoute,
    /// A web link from a domain that is not the app's.
    Domain(String),
    /// No route is declared at that path.
    Unknown(String),
    /// `args=` is not a JSON object.
    BadArgs(String),
    /// The route's action is not on the list.
    NoAction(String),
    /// The route's action deletes or replaces things: never from a link.
    Destructive(String),
    /// The action refused, or the person said no.
    Failed(String),
}

impl Refused {
    /// Its kind, for a log line: never the link's words.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotARoute => "not_a_route",
            Self::Domain(_) => "domain",
            Self::Unknown(_) => "unknown",
            Self::BadArgs(_) => "bad_args",
            Self::NoAction(_) => "no_action",
            Self::Destructive(_) => "destructive",
            Self::Failed(_) => "failed",
        }
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotARoute => write!(f, "not a link to one of the app's routes"),
            Self::Domain(d) => write!(f, "{d} is not one of the app's domains"),
            Self::Unknown(p) => write!(f, "no route {p:?}"),
            Self::BadArgs(why) => write!(f, "args is not a JSON object: {why}"),
            Self::NoAction(a) => write!(f, "no action {a}"),
            Self::Destructive(a) => write!(f, "{a} deletes or replaces things: never from a link"),
            Self::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// What a link asks for: its route, and the arguments to run it with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Followed {
    pub route: Route,
    pub args: Value,
    /// Where it came from: the app's scheme, or the domain of a
    /// universal link (the confirmation sheet's caller says it).
    pub from: String,
}

/// The route `raw` names, its arguments merged (the route's own win).
///
/// # Errors
///
/// Why it names none.
pub fn resolve(raw: &str) -> Result<Followed, Refused> {
    let url = url::Url::parse(raw.trim()).map_err(|_| Refused::NotARoute)?;
    let from;
    let path = match url.scheme() {
        s if s == SCHEME => {
            from = SCHEME.to_owned();
            let host = url.host_str().unwrap_or_default();
            let rest = url.path().trim_matches('/');
            if rest.is_empty() {
                host.to_owned()
            } else {
                format!("{host}/{rest}")
            }
        }
        "https" => {
            let rest = url
                .path()
                .strip_prefix(UNIVERSAL_PREFIX)
                .map(|p| p.trim_matches('/').to_owned())
                .ok_or(Refused::NotARoute)?;
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            let ours = domains()
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&host);
            if !ours {
                return Err(Refused::Domain(host));
            }
            from = host;
            rest
        }
        _ => return Err(Refused::NotARoute),
    };
    // A share link (`architect://open?url=…`) is the Shared tab's.
    if path.is_empty() || path == "open" {
        return Err(Refused::NotARoute);
    }
    let route = routes()
        .into_iter()
        .find(|r| r.path == path)
        .ok_or(Refused::Unknown(path))?;
    let mut args = Map::new();
    for (k, v) in url.query_pairs() {
        if k == "args" {
            match serde_json::from_str::<Value>(&v) {
                Ok(Value::Object(m)) => args.extend(m),
                Ok(_) => return Err(Refused::BadArgs("not an object".to_owned())),
                Err(e) => return Err(Refused::BadArgs(e.to_string())),
            }
        } else {
            args.insert(k.into_owned(), Value::String(v.into_owned()));
        }
    }
    if let Value::Object(fixed) = &route.args {
        args.extend(fixed.clone());
    }
    Ok(Followed {
        route,
        args: Value::Object(args),
        from,
    })
}

/// The caller a link from `from` runs as (`link:architect`,
/// `link:<domain>`): from outside, asking before any write; the
/// confirmation sheet shows it.
#[must_use]
pub fn caller(from: &str) -> ActionCaller {
    ActionCaller::new(Principal::Service {
        name: format!("{LINK_CALLER}:{from}"),
    })
    .confirming_writes()
}

/// How long a link waits for the person's answer.
const ASK_FOR: Duration = Duration::from_secs(120);

/// Follows `raw` on `list`: its route's action, run as [`caller`]; when
/// the list asks the person first, it waits for their answer.
///
/// # Errors
///
/// Why it did not run.
pub async fn follow(list: &ActionList, raw: &str) -> Result<Value, Refused> {
    let followed = resolve(raw)?;
    let action = &followed.route.action;
    let info = list
        .info(action)
        .ok_or_else(|| Refused::NoAction(action.clone()))?;
    if info.effect == Effect::Destructive {
        return Err(Refused::Destructive(action.clone()));
    }
    let args = followed.args.to_string();
    let caller = caller(&followed.from);
    let first = list
        .run_json_async_with(&caller, action, args.as_bytes())
        .await;
    let out = match first {
        Err(ActionError::ConfirmationRequired { ticket, .. }) => {
            // The person sees the sheet; the ticket leaves the list's
            // pending ones when they answer (or it expires).
            let started = std::time::Instant::now();
            while list
                .pending_confirmations()
                .iter()
                .any(|c| c.ticket == ticket)
                && started.elapsed() < ASK_FOR
            {
                futures_timer::Delay::new(Duration::from_millis(250)).await;
            }
            list.run_json_async_with(&caller.with_ticket(ticket), action, args.as_bytes())
                .await
        }
        other => other,
    };
    out.map(|bytes| serde_json::from_slice(&bytes).unwrap_or(Value::Null))
        .map_err(|e| Refused::Failed(e.to_string()))
}

type Inbox = (
    UnboundedSender<String>,
    Mutex<Option<UnboundedReceiver<String>>>,
);

fn inbox() -> &'static Inbox {
    static INBOX: OnceLock<Inbox> = OnceLock::new();
    INBOX.get_or_init(|| {
        let (tx, rx) = unbounded();
        (tx, Mutex::new(Some(rx)))
    })
}

/// A link the OS handed the app, from any thread: followed by the window
/// once it is up ([`use_links`]). Whether it names a route (the rest are
/// left to the caller: a share link, a tab).
pub fn incoming(raw: impl Into<String>) -> bool {
    let raw = raw.into();
    if resolve(&raw).is_err() {
        return false;
    }
    let _ = inbox().0.unbounded_send(raw);
    true
}

/// How many links may wait for the person at once: past it, a link is
/// dropped (said in the log), so a pile of them cannot stack up sheets.
pub const MAX_WAITING: usize = 4;

/// The links being followed now (each may wait up to two minutes for the
/// person): a duplicate of one is dropped, and at most [`MAX_WAITING`].
#[derive(Debug, Default)]
pub struct Waiting(std::collections::HashSet<String>);

impl Waiting {
    /// Whether `raw` may be followed now: not one already waiting, and
    /// room for it.
    pub fn admit(&mut self, raw: &str) -> bool {
        if self.0.contains(raw) || self.0.len() >= MAX_WAITING {
            return false;
        }
        self.0.insert(raw.to_owned());
        true
    }

    /// `raw` was followed (or refused).
    pub fn done(&mut self, raw: &str) {
        self.0.remove(raw);
    }
}

/// What a log line says of a link: its route, never its text or
/// arguments (they can be a note's words).
#[must_use]
pub fn route_of(raw: &str) -> String {
    resolve(raw).map_or_else(|_| "-".to_owned(), |f| f.route.path)
}

/// The window follows the links the OS hands it, on its Action List.
pub fn use_links(list: &std::sync::Arc<ActionList>) {
    let list = list.clone();
    dioxus::prelude::use_hook(move || {
        let taken = inbox()
            .1
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(mut links) = taken {
            let waiting = std::sync::Arc::new(Mutex::new(Waiting::default()));
            dioxus::core::spawn_forever(async move {
                use futures_util::StreamExt as _;
                while let Some(raw) = links.next().await {
                    let route = route_of(&raw);
                    if !waiting
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .admit(&raw)
                    {
                        tracing::warn!(link.route = %route, "link: dropped (the same one, or too many, waiting)");
                        continue;
                    }
                    // Each on its own: one waiting for the person does
                    // not hold the others up.
                    let list = list.clone();
                    let waiting = waiting.clone();
                    dioxus::core::spawn_forever(async move {
                        match follow(&list, &raw).await {
                            Ok(_) => tracing::info!(link.route = %route, "link: followed"),
                            Err(why) => {
                                tracing::warn!(link.route = %route, link.refused = why.kind(), "link: refused");
                            }
                        }
                        waiting
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .done(&raw);
                    });
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use architect::action::ActionMeta;
    use futures::executor::block_on;

    const fn meta(id: &'static str) -> ActionMeta {
        ActionMeta {
            id,
            trait_name: "T",
            method_name: id,
            display_name: id,
            description: "",
            category: "T",
            group: "T",
            shortcut: "",
            toggleable: false,
            undo: false,
        }
    }

    #[derive(Debug, Clone, facet::Facet)]
    struct TestArgs {
        text: Option<String>,
        fixed: Option<String>,
        n: Option<i64>,
    }

    static OPEN: ActionMeta = meta("LINK_TEST_OPEN");
    static ADD: ActionMeta = meta("LINK_TEST_ADD");
    static WIPE: ActionMeta = meta("LINK_TEST_WIPE");

    fn list() -> ActionList {
        let list = ActionList::new();
        for (m, e) in [
            (&OPEN, Effect::ReadOnly),
            (&ADD, Effect::Mutating),
            (&WIPE, Effect::Destructive),
        ] {
            // It answers with its arguments.
            list.register_with(m, e, |_, args: TestArgs| Ok::<_, String>(args))
                .unwrap();
        }
        declare(
            Route::new("t/open", "LINK_TEST_OPEN").with_args(serde_json::json!({"fixed": "yes"})),
        );
        declare(Route::new("t/add", "LINK_TEST_ADD"));
        declare(Route::new("t/wipe", "LINK_TEST_WIPE"));
        list
    }

    #[test]
    fn a_link_names_a_declared_route_never_an_action() {
        let _ = list();
        let f = resolve("architect://t/open?text=hi&fixed=no").unwrap();
        assert_eq!(f.route.action, "LINK_TEST_OPEN");
        // The route's own arguments win over the link's.
        assert_eq!(f.args, serde_json::json!({ "text": "hi", "fixed": "yes" }));
        let u = resolve("https://architect.fasttrackstudio.app/a/t/open?args=%7B%22n%22%3A2%7D")
            .unwrap();
        assert_eq!(u.args["n"], 2);
        assert_eq!(
            resolve("architect://LINK_TEST_OPEN"),
            Err(Refused::Unknown("LINK_TEST_OPEN".to_owned()))
        );
        assert_eq!(
            resolve("architect://open?url=https://n/s/x/"),
            Err(Refused::NotARoute)
        );
        assert_eq!(
            resolve("https://architect.example/s/abc/"),
            Err(Refused::NotARoute)
        );
        assert!(matches!(
            resolve("architect://t/open?args=3"),
            Err(Refused::BadArgs(_))
        ));
        assert!(resolve("architect://capture?text=milk").is_ok());
        // A web link only from the app's own domains (L4).
        assert_eq!(
            resolve("https://evil.example/a/t/open"),
            Err(Refused::Domain("evil.example".to_owned()))
        );
        let ours = resolve("https://architect.fasttrackstudio.app/a/t/open").unwrap();
        assert_eq!(ours.from, "architect.fasttrackstudio.app");
        assert_eq!(resolve("architect://t/open").unwrap().from, "architect");
        declare_domain("architect.example");
        assert!(resolve("https://architect.example/a/t/open").is_ok());
        // The sheet's caller says where it came from.
        assert_eq!(
            caller("architect.example").principal().cloned(),
            Some(Principal::Service {
                name: "link:architect.example".to_owned()
            })
        );
    }

    #[test]
    fn a_log_line_names_the_route_never_the_link() {
        let _ = list();
        let raw = "architect://t/open?text=my%20secret%20note&args=%7B%22n%22%3A2%7D";
        let route = route_of(raw);
        assert_eq!(route, "t/open");
        assert!(
            !route.contains("secret") && !route.contains("args") && !route.contains("text"),
            "{route}"
        );
        assert_eq!(route_of("https://evil.example/a/t/open?text=x"), "-");
        assert_eq!(Refused::Failed("my secret".to_owned()).kind(), "failed");
    }

    #[test]
    fn a_link_waiting_holds_its_duplicates_and_the_queue_is_capped() {
        let mut w = Waiting::default();
        assert!(w.admit("architect://capture?text=a"));
        // The same link again while it waits: dropped.
        assert!(!w.admit("architect://capture?text=a"));
        for i in 1..MAX_WAITING {
            assert!(w.admit(&format!("architect://capture?text={i}")));
        }
        // Full: the next waits for one to finish.
        assert!(!w.admit("architect://capture?text=more"));
        w.done("architect://capture?text=a");
        assert!(w.admit("architect://capture?text=more"));
    }

    #[test]
    fn a_read_runs_a_write_asks_and_a_destructive_one_never_runs() {
        let list = list();
        list.require_confirmation(Duration::from_secs(60));
        let out = block_on(follow(&list, "architect://t/open?text=hi")).unwrap();
        assert_eq!(out["text"], "hi");
        assert_eq!(
            block_on(follow(&list, "architect://t/wipe")),
            Err(Refused::Destructive("LINK_TEST_WIPE".to_owned()))
        );
        // A write waits for the person; denied, it does not run.
        let pending = std::thread::spawn({
            let list = list.clone();
            move || block_on(follow(&list, "architect://t/add?text=x"))
        });
        let ticket = loop {
            if let Some(c) = list.pending_confirmations().first() {
                break c.ticket.clone();
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(list.deny(&ticket));
        assert!(matches!(pending.join().unwrap(), Err(Refused::Failed(_))));
        // Approved, it runs.
        let pending = std::thread::spawn({
            let list = list.clone();
            move || block_on(follow(&list, "architect://t/add?text=y"))
        });
        let ticket = loop {
            if let Some(c) = list.pending_confirmations().first() {
                break c.ticket.clone();
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(list.approve(&ticket));
        assert_eq!(pending.join().unwrap().unwrap()["text"], "y");
    }
}
