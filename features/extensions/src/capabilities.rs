//! What an extension asks to reach, as a person reads it and as the host
//! checks it (#178).
//!
//! A manifest's [`Capabilities`] are what it asks for; a grant
//! (`crate::consent`) is what a person said yes to. What it may do is
//! the two together ([`Capabilities::within`]): an update that asks for
//! more than was granted runs with what was granted until the person says
//! yes to the rest ([`Capabilities::grows_from`] is what to ask them).

use serde::Serialize;

use crate::manifest::{Capabilities, Reach};

/// What of the app's state an extension may read
/// ([`Capabilities::reads`]).
pub const READS: [&str; 5] = [ORG, SELECTION, DOCUMENTS, EDITOR, FILES];
/// The current organization.
pub const ORG: &str = "org";
/// What is selected and focused.
pub const SELECTION: &str = "selection";
/// The app's documents: open them, read them (`reads`), and list and
/// watch them; change them (`writes`).
pub const DOCUMENTS: &str = "documents";
/// The images and lyrics files of a note its view draws (#669).
///
/// Art, chart images and synced lyrics, read-only and size-capped
/// (`architect_spaces::view`): from the person's own vault, or for a
/// share link's guest only from what the link shares. Handed in with the
/// note, so it needs [`DOCUMENTS`] too.
pub const FILES: &str = "files";
/// The editor being typed in (`crate::editor`): its text, selection and
/// the block the cursor is in (`reads`); edit it, take keys in it, draw
/// marks and widgets in it, add slash-menu rows (`writes`).
pub const EDITOR: &str = "editor";

/// What of the app's state an extension may change
/// ([`Capabilities::writes`]).
pub const WRITES: [&str; 2] = [DOCUMENTS, EDITOR];

/// What of the app's interface an extension may use
/// ([`Capabilities::ui`]).
pub const UI: [&str; 6] = [NOTIFY, CLIPBOARD, PICKERS, PROMPTS, MENUS, BADGES];
/// Notifications (toasts).
pub const NOTIFY: &str = "notify";
/// The clipboard: read it and write it.
pub const CLIPBOARD: &str = "clipboard";
/// File and folder pickers, and save dialogs: a file a person picks is
/// one it may then read (or, saved to, write) for the run.
pub const PICKERS: &str = "pickers";
/// Prompts: ask for text, a yes or no, one of a list.
pub const PROMPTS: &str = "prompts";
/// A context menu of its own, at a point.
pub const MENUS: &str = "menus";
/// Icons and badges on file-tree rows and tabs.
pub const BADGES: &str = "badges";

/// What the host may do with an extension's languages past highlighting
/// them ([`Capabilities::languages`]).
pub const LANGUAGES: [&str; 1] = [SERVERS];
/// Run a language's server (`[languages.server]`), a program on this
/// computer, while a file of the language is open.
pub const SERVERS: &str = "servers";

/// One thing an extension asks for, in a person's words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Ask {
    /// `actions`, `effect`, `hosts`, `folders`, `reads`, `writes`, `ui`,
    /// `events`, `ports` or `languages`.
    pub kind: &'static str,
    /// What exactly (`TRANSPORT_`, `api.github.com`, `selection`).
    pub what: String,
    /// The sentence a consent prompt shows.
    pub says: String,
}

/// Whether prefix `p` is inside one of `of` (`*` is every action).
fn prefix_in(p: &str, of: &[String]) -> bool {
    of.iter().any(|g| g == "*" || p.starts_with(g.as_str()))
}

/// Whether host `h` is one of `of` (`*` any; `*.example.com` its
/// subdomains).
fn host_in(h: &str, of: &[String]) -> bool {
    of.iter().any(|g| {
        g == "*"
            || g == h
            || g.strip_prefix("*.")
                .is_some_and(|d| h.ends_with(&format!(".{d}")) || h == d)
    })
}

/// Whether network host `host` (on `port`) is one of `of`: by name, by
/// domain (`*.example.com`), as `host:port`, or `*`.
#[must_use]
pub fn reaches_host(of: &[String], host: &str, port: Option<u16>) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    let lower: Vec<String> = of.iter().map(|h| h.to_ascii_lowercase()).collect();
    host_in(&host, &lower)
        || port.is_some_and(|p| lower.iter().any(|g| *g == format!("{host}:{p}")))
}

/// Whether `ports` entry `entry` (`midi:<name>`, `osc:<host>:<port>`,
/// `osc:listen:<port>`) is well formed: the reason it is not.
///
/// # Errors
///
/// What is wrong with it.
pub fn check_port_entry(entry: &str) -> Result<(), String> {
    match entry.split_once(':') {
        Some(("midi", name)) if !name.trim().is_empty() => Ok(()),
        Some(("osc", "*")) => Ok(()),
        Some(("osc", rest)) => {
            let (host, port) = rest
                .rsplit_once(':')
                .ok_or_else(|| "an OSC endpoint is <host>:<port>".to_owned())?;
            let port_ok = port == "*" || port.parse::<u16>().is_ok_and(|p| p > 0);
            if host.is_empty() || !port_ok {
                return Err("an OSC endpoint is <host>:<port> (or *), osc:listen:<port>".to_owned());
            }
            Ok(())
        }
        _ => Err("midi:<port name> (a trailing * for any it begins), osc:<host>:<port>, osc:listen:<port> or osc:*".to_owned()),
    }
}

/// Whether one of `of` (`capabilities.ports`) lets it open `kind` port
/// `name` (a MIDI port's name, an OSC endpoint's `host:port`) and, an OSC
/// one, hear on local port `listen`.
#[must_use]
pub fn opens_port(of: &[String], kind: &str, name: &str, listen: Option<u16>) -> bool {
    let entries = || {
        of.iter()
            .filter_map(move |e| e.strip_prefix(kind).and_then(|r| r.strip_prefix(':')))
    };
    let named = match kind {
        "midi" => entries().any(|g| {
            g == "*" || g == name || g.strip_suffix('*').is_some_and(|p| name.starts_with(p))
        }),
        "osc" => {
            let name = name.to_ascii_lowercase();
            let host = name.rsplit_once(':').map_or(name.as_str(), |(h, _)| h);
            entries().any(|g| {
                let g = g.to_ascii_lowercase();
                !g.starts_with("listen:")
                    && (g == "*" || g == name || g.strip_suffix(":*").is_some_and(|h| h == host))
            })
        }
        _ => false,
    };
    let hears = match (kind, listen) {
        (_, None) => true,
        ("osc", Some(p)) => entries().any(|g| {
            g == "*" || g == "listen:*" || g.strip_prefix("listen:") == Some(p.to_string().as_str())
        }),
        _ => false,
    };
    named && hears
}

fn port_words(p: &str) -> String {
    match p.split_once(':') {
        Some(("midi", "*")) => "Use any of your MIDI ports".to_owned(),
        Some(("midi", name)) => name.strip_suffix('*').map_or_else(
            || format!("Use your MIDI port {name}"),
            |prefix| format!("Use your MIDI ports named {prefix}…"),
        ),
        Some(("osc", "*")) => "Send and receive OSC anywhere on the network".to_owned(),
        Some(("osc", rest)) => match rest.strip_prefix("listen:") {
            Some("*") => "Listen for OSC on any port of this computer".to_owned(),
            Some(port) => format!("Listen for OSC on port {port} of this computer"),
            None => format!("Send and receive OSC at {rest}"),
        },
        _ => format!("Use {p}"),
    }
}

/// Whether folder `f` is one of `of` or inside one.
fn folder_in(f: &str, of: &[String]) -> bool {
    let f = f.trim_end_matches('/');
    of.iter().any(|g| {
        let g = g.trim_end_matches('/');
        f == g || f.starts_with(&format!("{g}/"))
    })
}

const fn reach_words(r: Reach) -> &'static str {
    match r {
        Reach::ReadOnly => "only read",
        Reach::Mutating => "change things",
        Reach::Destructive => "delete or replace things",
    }
}

fn ui_words(u: &str) -> String {
    match u {
        NOTIFY => "Show you notifications".to_owned(),
        CLIPBOARD => "Read and change what you copied".to_owned(),
        PICKERS => "Ask you to pick files and folders, and where to save".to_owned(),
        PROMPTS => "Ask you questions".to_owned(),
        MENUS => "Show menus of its own".to_owned(),
        BADGES => "Mark files and tabs with icons and badges".to_owned(),
        other => format!("Use {other}"),
    }
}

fn read_words(r: &str) -> String {
    match r {
        ORG => "See which organization you are working in".to_owned(),
        SELECTION => "See what you have selected and focused".to_owned(),
        DOCUMENTS => "Open, read, list and watch your documents".to_owned(),
        EDITOR => "See the text and selection of the editor you are typing in".to_owned(),
        FILES => "See the images and lyrics files of a note it shows".to_owned(),
        other => format!("Read {other}"),
    }
}

fn write_words(w: &str) -> String {
    match w {
        DOCUMENTS => "Create, edit, rename, move and delete your documents".to_owned(),
        EDITOR => "Edit the text you are typing, take keys in the editor, and mark it up with highlights and widgets".to_owned(),
        other => format!("Change {other}"),
    }
}

impl Capabilities {
    /// It asks for nothing past its own actions, settings and data: no
    /// consent to ask.
    #[must_use]
    pub const fn asks_nothing(&self) -> bool {
        self.actions.is_empty()
            && self.hosts.is_empty()
            && self.folders.is_empty()
            && self.reads.is_empty()
            && self.writes.is_empty()
            && self.ui.is_empty()
            && self.events.is_empty()
            && self.ports.is_empty()
            && self.languages.is_empty()
    }

    /// Everything it asks for, one line a thing, in the order a prompt
    /// shows them.
    #[must_use]
    pub fn asks(&self) -> Vec<Ask> {
        self.grows_from(&Self::default())
    }

    /// What it asks for past `granted`: empty when `granted` covers it.
    #[must_use]
    pub fn grows_from(&self, granted: &Self) -> Vec<Ask> {
        let mut out = Vec::new();
        for p in self
            .actions
            .iter()
            .filter(|p| !prefix_in(p, &granted.actions))
        {
            out.push(Ask {
                kind: "actions",
                what: p.clone(),
                says: if p == "*" {
                    "Run any of the app's actions".to_owned()
                } else {
                    format!("Run the app's {p}… actions")
                },
            });
        }
        let runs_any = !self.actions.is_empty();
        if runs_any && (self.effect > granted.effect || granted.actions.is_empty()) {
            out.push(Ask {
                kind: "effect",
                what: self.effect.name().to_owned(),
                says: format!("Through them, {}", reach_words(self.effect)),
            });
        }
        for r in self.reads.iter().filter(|r| !granted.reads.contains(r)) {
            out.push(Ask {
                kind: "reads",
                what: r.clone(),
                says: read_words(r),
            });
        }
        for w in self.writes.iter().filter(|w| !granted.writes.contains(w)) {
            out.push(Ask {
                kind: "writes",
                what: w.clone(),
                says: write_words(w),
            });
        }
        for u in self.ui.iter().filter(|u| !granted.ui.contains(u)) {
            out.push(Ask {
                kind: "ui",
                what: u.clone(),
                says: ui_words(u),
            });
        }
        for e in self.events.iter().filter(|e| !granted.events.contains(e)) {
            out.push(Ask {
                kind: "events",
                what: e.clone(),
                says: if e == "*" {
                    "Hear every event the app raises".to_owned()
                } else {
                    format!("Hear the app's {e} events")
                },
            });
        }
        for p in self.ports.iter().filter(|p| !granted.ports.contains(p)) {
            out.push(Ask {
                kind: "ports",
                what: p.clone(),
                says: port_words(p),
            });
        }
        for l in self
            .languages
            .iter()
            .filter(|l| !granted.languages.contains(l))
        {
            out.push(Ask {
                kind: "languages",
                what: l.clone(),
                says: if l == SERVERS {
                    "Run its languages' servers (programs on this computer) while you edit their files"
                        .to_owned()
                } else {
                    format!("Use its languages' {l}")
                },
            });
        }
        for f in self
            .folders
            .iter()
            .filter(|f| !folder_in(f, &granted.folders))
        {
            out.push(Ask {
                kind: "folders",
                what: f.clone(),
                says: format!("Read and write files in {f}"),
            });
        }
        for h in self.hosts.iter().filter(|h| !host_in(h, &granted.hosts)) {
            out.push(Ask {
                kind: "hosts",
                what: h.clone(),
                says: if h == "*" {
                    "Reach any server on the network".to_owned()
                } else {
                    format!("Reach {h} on the network")
                },
            });
        }
        out
    }

    /// What it may do when `granted` is what a person said yes to: each
    /// part the narrower of the two.
    #[must_use]
    pub fn within(&self, granted: &Self) -> Self {
        let mut actions: Vec<String> = self
            .actions
            .iter()
            .filter(|p| prefix_in(p, &granted.actions))
            .cloned()
            .collect();
        for g in &granted.actions {
            if g != "*" && prefix_in(g, &self.actions) && !prefix_in(g, &actions) {
                actions.push(g.clone());
            }
        }
        let mut hosts: Vec<String> = self
            .hosts
            .iter()
            .filter(|h| host_in(h, &granted.hosts))
            .cloned()
            .collect();
        for g in &granted.hosts {
            if g != "*" && host_in(g, &self.hosts) && !host_in(g, &hosts) {
                hosts.push(g.clone());
            }
        }
        let mut folders: Vec<String> = self
            .folders
            .iter()
            .filter(|f| folder_in(f, &granted.folders))
            .cloned()
            .collect();
        for g in &granted.folders {
            if folder_in(g, &self.folders) && !folder_in(g, &folders) {
                folders.push(g.clone());
            }
        }
        Self {
            actions,
            effect: self.effect.min(granted.effect),
            hosts,
            folders,
            // WebRTC only as declared and granted.
            webrtc: self
                .webrtc
                .iter()
                .filter(|w| granted.webrtc.contains(w))
                .cloned()
                .collect(),
            reads: self
                .reads
                .iter()
                .filter(|r| granted.reads.contains(r))
                .cloned()
                .collect(),
            writes: self
                .writes
                .iter()
                .filter(|w| granted.writes.contains(w))
                .cloned()
                .collect(),
            ui: self
                .ui
                .iter()
                .filter(|u| granted.ui.contains(u))
                .cloned()
                .collect(),
            events: self
                .events
                .iter()
                .filter(|e| granted.events.contains(e))
                .cloned()
                .collect(),
            ports: self
                .ports
                .iter()
                .filter(|p| granted.ports.contains(p))
                .cloned()
                .collect(),
            languages: self
                .languages
                .iter()
                .filter(|l| granted.languages.contains(l))
                .cloned()
                .collect(),
        }
    }

    /// Whether it may run action `id` of `effect` (past its own
    /// namespace, `own`): the capability it lacks and why, when it may
    /// not.
    ///
    /// # Errors
    ///
    /// `actions`: not a namespace it may run; `effect`: more than it may
    /// change.
    pub fn may_run(
        &self,
        own: &str,
        id: &str,
        effect: Reach,
    ) -> Result<(), (&'static str, String)> {
        // The rule itself is `architect_favourites::may_run`, which a
        // browser's favourites use too: one rule, nothing to drift.
        match architect_favourites::may_run(own, &self.actions, self.effect, id, effect) {
            Ok(()) => Ok(()),
            Err(architect_favourites::Refused::Namespace) => Err((
                "actions",
                format!("{id} is not in a namespace this extension may run"),
            )),
            Err(architect_favourites::Refused::Effect) => Err((
                "effect",
                format!(
                    "{id} is {}: this extension may run {} actions at most",
                    effect.name(),
                    self.effect.name()
                ),
            )),
        }
    }

    /// Whether it may read `what` ([`READS`]).
    #[must_use]
    pub fn may_read(&self, what: &str) -> bool {
        self.reads.iter().any(|r| r == what)
    }

    /// Whether it may change `what` ([`WRITES`]).
    #[must_use]
    pub fn may_write(&self, what: &str) -> bool {
        self.writes.iter().any(|w| w == what)
    }

    /// Whether it may use `what` of the app's interface ([`UI`]).
    #[must_use]
    pub fn may_use(&self, what: &str) -> bool {
        self.ui.iter().any(|u| u == what)
    }

    /// Whether it may hear app-defined event `name` (`app:<name>` hooks;
    /// `capabilities.events`).
    #[must_use]
    pub fn may_hear(&self, name: &str) -> bool {
        self.events.iter().any(|p| crate::events::covers(p, name))
    }

    /// Whether it may reach network host `host` (`capabilities.hosts`):
    /// an entry naming it (`api.example.com`), its domain
    /// (`*.example.com`), it and `port` (`127.0.0.1:8080`), or `*`.
    #[must_use]
    pub fn may_reach_host(&self, host: &str, port: Option<u16>) -> bool {
        reaches_host(&self.hosts, host, port)
    }

    /// Whether it may open `kind` (`midi`, `osc`) port `name` and, an OSC
    /// endpoint, hear on local port `listen` (`capabilities.ports`).
    #[must_use]
    pub fn may_open_port(&self, kind: &str, name: &str, listen: Option<u16>) -> bool {
        opens_port(&self.ports, kind, name, listen)
    }

    /// Whether it may reach `folder` (an absolute path, `~` already
    /// expanded in both).
    #[must_use]
    pub fn may_reach_folder(&self, path: &str) -> bool {
        folder_in(path, &self.folders)
    }
}

#[cfg(test)]
mod tests {
    use crate::manifest::{Capabilities, Reach};

    fn caps(actions: &[&str], effect: Reach, reads: &[&str]) -> Capabilities {
        Capabilities {
            actions: actions.iter().map(|s| (*s).to_owned()).collect(),
            effect,
            reads: reads.iter().map(|s| (*s).to_owned()).collect(),
            ..Capabilities::default()
        }
    }

    #[test]
    fn an_update_that_asks_for_more_says_only_the_more() {
        let was = caps(&["TRANSPORT_"], Reach::ReadOnly, &["selection"]);
        let now = caps(
            &["TRANSPORT_", "WORKSPACE_"],
            Reach::Mutating,
            &["selection", "org"],
        );
        let grown: Vec<(&str, String)> = now
            .grows_from(&was)
            .into_iter()
            .map(|a| (a.kind, a.what))
            .collect();
        assert_eq!(
            grown,
            vec![
                ("actions", "WORKSPACE_".to_owned()),
                ("effect", "mutating".to_owned()),
                ("reads", "org".to_owned()),
            ]
        );
        assert!(was.grows_from(&now).is_empty(), "less is covered");
        assert!(Capabilities::default().asks_nothing());
        assert!(!was.asks_nothing());
    }

    #[test]
    fn what_it_may_do_is_the_narrower_of_ask_and_grant() {
        let asks = caps(&["*"], Reach::Destructive, &["org", "selection"]);
        let granted = caps(&["TRANSPORT_"], Reach::Mutating, &["selection"]);
        let may = asks.within(&granted);
        assert_eq!(may.actions, vec!["TRANSPORT_".to_owned()]);
        assert_eq!(may.effect, Reach::Mutating);
        assert_eq!(may.reads, vec!["selection".to_owned()]);
        assert!(
            may.may_run("FTS_M_", "FTS_M_GO", Reach::Destructive)
                .is_ok(),
            "its own"
        );
        assert!(
            may.may_run("FTS_M_", "TRANSPORT_PLAY", Reach::Mutating)
                .is_ok()
        );
        assert!(
            may.may_run("FTS_M_", "WORKSPACE_CLOSE", Reach::ReadOnly)
                .is_err()
        );
        assert!(
            may.may_run("FTS_M_", "TRANSPORT_WIPE", Reach::Destructive)
                .is_err()
        );
        assert!(
            asks.within(&Capabilities::default()).asks_nothing(),
            "nothing granted"
        );
    }

    #[test]
    fn the_interface_it_uses_is_asked_for_and_granted_one_by_one() {
        let asks = Capabilities {
            ui: vec!["notify".to_owned(), "clipboard".to_owned()],
            ..Capabilities::default()
        };
        assert!(!asks.asks_nothing());
        let said: Vec<(&str, String)> = asks.asks().into_iter().map(|a| (a.kind, a.what)).collect();
        assert_eq!(
            said,
            vec![("ui", "notify".to_owned()), ("ui", "clipboard".to_owned())]
        );
        let granted = Capabilities {
            ui: vec!["notify".to_owned()],
            ..Capabilities::default()
        };
        let may = asks.within(&granted);
        assert!(may.may_use("notify"));
        assert!(!may.may_use("clipboard"), "not granted");
        assert_eq!(asks.grows_from(&granted).len(), 1);
    }

    /// A favourite's action rule in a browser
    /// (`architect_favourites::Declarer::may_target`) is `may_run`'s, a
    /// destructive action never: both are `architect_favourites::may_run`,
    /// and this holds the two effect scales and the mapping between them.
    #[test]
    fn a_favourites_action_rule_is_may_runs() {
        use architect_favourites::{Declarer, Effect};
        let effect = |r: Reach| match r {
            Reach::ReadOnly => Effect::ReadOnly,
            Reach::Mutating => Effect::Mutating,
            Reach::Destructive => Effect::Destructive,
        };
        let ids = [
            "ACME_CHARTS_OPEN",
            "ACME_CHARTSX_OPEN",
            "PLAYER_TOGGLE",
            "VAULT_DELETE",
            "APP_SETTINGS",
        ];
        let reaches = [Reach::ReadOnly, Reach::Mutating, Reach::Destructive];
        for named in [&[][..], &["PLAYER_"][..], &["*"][..]] {
            for most in reaches {
                let c = caps(named, most, &[]);
                let d = Declarer {
                    extension: "acme.charts".into(),
                    actions: c.actions.clone(),
                    effect: effect(most),
                };
                assert_eq!(
                    crate::manifest::action_prefix("acme.charts"),
                    architect_favourites::action_prefix("acme.charts")
                );
                for id in ids {
                    for e in reaches {
                        let may_run = c
                            .may_run(&crate::manifest::action_prefix(&d.extension), id, e)
                            .is_ok();
                        assert_eq!(
                            d.may_target(id, effect(e)),
                            may_run && e != Reach::Destructive,
                            "{id} {e:?} with {named:?} up to {most:?}"
                        );
                    }
                }
            }
        }
    }
}
