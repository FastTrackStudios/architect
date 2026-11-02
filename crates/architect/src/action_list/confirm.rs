//! Confirmation tickets: a person's yes before a caller from outside runs
//! what cannot be taken back.
//!
//! Off until the app turns it on ([`ActionList::require_confirmation`]):
//! only an app with a person in front of it, and a sheet to ask them in,
//! can answer a ticket — a headless server that turned it on would leave
//! every agent waiting. Once on, a call needs a person's approval when
//!
//! - it is made by a caller from outside (an [`ActionCaller`] with a
//!   principal: a model over MCP, a peer, a script run as someone). The
//!   app itself — a key, a menu, the palette, which a person drives — is
//!   never asked;
//! - and the action says so: an [`Effect::Destructive`] one by default,
//!   or whatever [`ActionList::set_confirm`] decides for that call (a
//!   mutating action flagged `confirm`, a command runner that only asks
//!   for the commands that close something).
//!
//! The first call is refused with [`ActionError::ConfirmationRequired`]
//! and a ticket; the person sees it in [`ActionList::pending_confirmations`]
//! (the action, its arguments, who asked) and answers with
//! [`ActionList::approve`] or [`ActionList::deny`]; the caller runs the
//! same call again carrying the ticket ([`ActionCaller::with_ticket`]).
//! A ticket is good for one run of exactly that call — the same caller,
//! action and arguments — before it expires ([`CONFIRM_TTL`] unless set).
//! Every change is a `Changed { topic: "confirmation" }` event
//! ([`CONFIRM_TOPIC`], detail `"<ticket> pending|approved|denied"`), so a
//! sheet or a waiting agent (`watch`) hears it.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use web_time::Instant;

use super::{ActionCaller, ActionError, ActionList, Effect, Entry, admit_args};

/// How long a ticket lasts when the app does not say: long enough for a
/// person to read the sheet, short enough that a yes is about now.
pub const CONFIRM_TTL: Duration = Duration::from_secs(120);

/// The topic of the `Changed` event every ticket's change is.
pub const CONFIRM_TOPIC: &str = "confirmation";

/// Whether one call of an action needs a person's yes: handed the call's
/// arguments (JSON, as the action takes them).
pub type ConfirmCheck = Arc<dyn Fn(&[u8]) -> bool + Send + Sync>;

/// Where a ticket is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfirmState {
    /// Waiting for the person.
    Pending,
    /// The person said yes: the call may run once.
    Approved,
    /// The person said no.
    Denied,
    /// Spent: it ran, or its refusal was told.
    Used,
}

impl ConfirmState {
    const fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Used => "used",
        }
    }
}

/// A call waiting for a person, as the sheet that asks them shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirmation {
    /// What the caller carries back, and what the person answers.
    pub ticket: String,
    /// The action's id (`FILES_DELETE`).
    pub id: String,
    /// Its name, as a menu shows it.
    pub display_name: String,
    pub effect: Effect,
    /// The arguments it would run with, as JSON (`null` for none).
    pub args: String,
    /// Who asked ([`architect_permissions::Principal::describe`]:
    /// `service:claude`, `user:ada`).
    pub caller: String,
    /// How long the ticket has left.
    pub expires_in: Duration,
}

/// The exact call a ticket is good for.
#[derive(Clone, PartialEq, Eq)]
struct Call {
    caller: String,
    id: String,
    args: Vec<u8>,
}

struct Ticket {
    call: Call,
    display_name: String,
    effect: Effect,
    issued: Instant,
    state: ConfirmState,
}

/// The list's tickets, and what it asks about.
#[derive(Default)]
pub(super) struct Gate {
    /// `Some(ttl)` once [`ActionList::require_confirmation`] turned it on.
    ttl: Option<Duration>,
    /// What [`ActionList::set_confirm`] said, by action id.
    checks: BTreeMap<String, ConfirmCheck>,
    tickets: BTreeMap<String, Ticket>,
}

impl Gate {
    /// Forget tickets a while after they expire: kept one more lifetime,
    /// so a late retry hears "expired" or "used", not "no such ticket".
    fn sweep(&mut self, ttl: Duration, now: Instant) {
        self.tickets
            .retain(|_, t| now.saturating_duration_since(t.issued) < ttl.saturating_mul(2));
    }
}

impl ActionList {
    /// Ask a person before a caller from outside runs what needs it — an
    /// [`Effect::Destructive`] action, or a call [`Self::set_confirm`]
    /// says so of — each ticket lasting `ttl` ([`CONFIRM_TTL`] is the
    /// usual). Off until called: only an app with a person to ask, and a
    /// sheet to ask them in, turns it on.
    pub fn require_confirmation(&self, ttl: Duration) {
        self.gate().ttl = Some(ttl);
    }

    /// Whether [`Self::require_confirmation`] is on.
    #[must_use]
    pub fn requires_confirmation(&self) -> bool {
        self.gate().ttl.is_some()
    }

    /// Decide per call whether `id` needs a person's yes, instead of its
    /// effect: `check` is handed the call's arguments. A mutating action
    /// that should ask answers `true`; a command runner asks only for the
    /// commands that close something. Kept until set again, also across
    /// a re-registration of the id.
    pub fn set_confirm<F>(&self, id: &str, check: F)
    where
        F: Fn(&[u8]) -> bool + Send + Sync + 'static,
    {
        self.gate().checks.insert(id.to_owned(), Arc::new(check));
    }

    /// The calls waiting for a person now, oldest first.
    #[must_use]
    pub fn pending_confirmations(&self) -> Vec<Confirmation> {
        let now = Instant::now();
        let gate = self.gate();
        let Some(ttl) = gate.ttl else {
            return Vec::new();
        };
        let mut out: Vec<(Instant, Confirmation)> = gate
            .tickets
            .iter()
            .filter(|(_, t)| t.state == ConfirmState::Pending)
            .filter_map(|(ticket, t)| {
                let left = ttl.checked_sub(now.saturating_duration_since(t.issued))?;
                (!left.is_zero()).then(|| {
                    (
                        t.issued,
                        Confirmation {
                            ticket: ticket.clone(),
                            id: t.call.id.clone(),
                            display_name: t.display_name.clone(),
                            effect: t.effect,
                            args: String::from_utf8_lossy(&t.call.args).into_owned(),
                            caller: t.call.caller.clone(),
                            expires_in: left,
                        },
                    )
                })
            })
            .collect();
        drop(gate);
        out.sort_by_key(|(issued, _)| *issued);
        out.into_iter().map(|(_, c)| c).collect()
    }

    /// The person said yes to `ticket`: its call may run once, until the
    /// ticket expires. False when it is not waiting (unknown, expired,
    /// already answered).
    ///
    /// The app's own door: never put this on the list, or a caller could
    /// approve its own ticket.
    #[must_use = "false: the ticket was not waiting"]
    pub fn approve(&self, ticket: &str) -> bool {
        self.decide(ticket, ConfirmState::Approved)
    }

    /// The person said no to `ticket`: its retry is refused. False as
    /// [`Self::approve`].
    #[must_use = "false: the ticket was not waiting"]
    pub fn deny(&self, ticket: &str) -> bool {
        self.decide(ticket, ConfirmState::Denied)
    }

    fn decide(&self, ticket: &str, to: ConfirmState) -> bool {
        let now = Instant::now();
        let mut gate = self.gate();
        let ttl = gate.ttl.unwrap_or(CONFIRM_TTL);
        let decided = gate.tickets.get_mut(ticket).is_some_and(|t| {
            let waiting =
                t.state == ConfirmState::Pending && now.saturating_duration_since(t.issued) < ttl;
            if waiting {
                t.state = to;
            }
            waiting
        });
        drop(gate);
        if decided {
            self.changed(CONFIRM_TOPIC, &format!("{ticket} {}", to.name()));
        }
        decided
    }

    fn gate(&self) -> std::sync::MutexGuard<'_, Gate> {
        self.inner
            .confirm
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// May `caller` run this call of `entry` now? `Ok` when it need not
    /// ask (the app itself, confirmation off, an action that does not
    /// need it) or its ticket is approved for exactly this call — which
    /// spends the ticket. Otherwise a new ticket, the same one still
    /// waiting, or why the ticket it carries is no good. Arguments that
    /// are not JSON are refused first: a person is never asked about a
    /// call that cannot run.
    pub(super) fn confirmed(
        &self,
        caller: &ActionCaller,
        entry: &Entry,
        args: &[u8],
    ) -> Result<(), ActionError> {
        let Some(who) = caller.principal() else {
            return Ok(());
        };
        let id = entry.meta.id;
        let (ttl, check) = {
            let gate = self.gate();
            let Some(ttl) = gate.ttl else {
                return Ok(());
            };
            (ttl, gate.checks.get(id).cloned())
        };
        // Asked with no lock held: a check may read the list.
        let needs = check.map_or(entry.effect == Effect::Destructive, |check| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(args)))
                // A check that panics asks: the safe side.
                .unwrap_or(true)
        });
        if !needs {
            return Ok(());
        }
        admit_args(entry, args)?;
        let call = Call {
            caller: who.describe(),
            id: id.to_owned(),
            args: args.to_vec(),
        };
        let ticket = caller.ticket().map(str::to_owned);
        let now = Instant::now();
        let found = {
            let mut gate = self.gate();
            gate.sweep(ttl, now);
            gate.admit(call, ticket.as_deref(), entry, ttl, now)
        };
        match found {
            Admit::Run => Ok(()),
            Admit::Issued(ticket) => {
                self.changed(CONFIRM_TOPIC, &format!("{ticket} pending"));
                Err(ActionError::ConfirmationRequired {
                    id: id.to_owned(),
                    ticket,
                    expires_in: ttl,
                })
            }
            Admit::Waiting(ticket, left) => Err(ActionError::ConfirmationRequired {
                id: id.to_owned(),
                ticket,
                expires_in: left,
            }),
            Admit::Refused(reason) => Err(ActionError::Denied {
                id: id.to_owned(),
                reason,
            }),
        }
    }
}

/// What [`Gate::admit`] made of a call.
enum Admit {
    /// Its ticket was approved, and is now spent.
    Run,
    /// It carried none: this new one waits for a person.
    Issued(String),
    /// Its ticket still waits, this long.
    Waiting(String, Duration),
    /// Its ticket is no good, for this reason.
    Refused(String),
}

impl Gate {
    fn admit(
        &mut self,
        call: Call,
        carried: Option<&str>,
        entry: &Entry,
        ttl: Duration,
        now: Instant,
    ) -> Admit {
        let Some(carried) = carried else {
            let ticket = uuid::Uuid::new_v4().simple().to_string();
            self.tickets.insert(
                ticket.clone(),
                Ticket {
                    call,
                    display_name: entry.meta.display_name.to_owned(),
                    effect: entry.effect,
                    issued: now,
                    state: ConfirmState::Pending,
                },
            );
            return Admit::Issued(ticket);
        };
        let again = "run it again without one to ask for a new one";
        let Some(held) = self.tickets.get_mut(carried) else {
            return Admit::Refused(format!("no ticket {carried}: {again}"));
        };
        if held.call != call {
            return Admit::Refused(format!(
                "ticket {carried} was issued for another call (caller, action or arguments \
                 differ): {again}"
            ));
        }
        let Some(left) = ttl
            .checked_sub(now.saturating_duration_since(held.issued))
            .filter(|left| !left.is_zero())
        else {
            return Admit::Refused(format!("ticket {carried} expired: {again}"));
        };
        match held.state {
            ConfirmState::Pending => Admit::Waiting(carried.to_owned(), left),
            ConfirmState::Approved => {
                held.state = ConfirmState::Used;
                Admit::Run
            }
            ConfirmState::Denied => {
                held.state = ConfirmState::Used;
                Admit::Refused("a person declined it".to_owned())
            }
            ConfirmState::Used => {
                Admit::Refused(format!("ticket {carried} was already used: {again}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ActionCaller, ActionError, ActionList, Effect, EventKind};
    use crate::action::ActionMeta;
    use architect_permissions::Principal;
    use facet::Facet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    static DELETE: ActionMeta = ActionMeta {
        id: "FILES_DELETE",
        trait_name: "Test",
        method_name: "delete",
        display_name: "Delete a file",
        description: "a test action",
        category: "Test",
        group: "Test",
        shortcut: "",
        toggleable: false,
        undo: false,
    };
    static RENAME: ActionMeta = ActionMeta {
        id: "FILES_RENAME",
        display_name: "Rename a file",
        method_name: "rename",
        ..DELETE
    };

    #[derive(Facet)]
    struct Path {
        path: String,
    }

    /// A list asking for confirmation, with a destructive `FILES_DELETE`
    /// and a mutating `FILES_RENAME`, each counting its runs.
    fn list(ttl: Duration) -> (ActionList, Arc<AtomicUsize>) {
        let list = ActionList::new();
        let runs = Arc::new(AtomicUsize::new(0));
        for (meta, effect) in [(&DELETE, Effect::Destructive), (&RENAME, Effect::Mutating)] {
            let ran = runs.clone();
            list.register(meta, effect, move |p: Path| {
                ran.fetch_add(1, Ordering::SeqCst);
                Ok(p.path)
            })
            .unwrap();
        }
        list.require_confirmation(ttl);
        (list, runs)
    }

    fn agent() -> ActionCaller {
        ActionCaller::new(Principal::Service {
            name: "agent".to_owned(),
        })
    }

    fn ticket_of(out: Result<Vec<u8>, ActionError>) -> String {
        match out {
            Err(ActionError::ConfirmationRequired { ticket, .. }) => ticket,
            other => panic!("expected a ticket, got {other:?}"),
        }
    }

    const A: &[u8] = br#"{"path":"a.txt"}"#;

    #[test]
    fn an_agent_runs_a_destructive_action_once_a_person_approves() {
        let (list, runs) = list(Duration::from_secs(60));
        let ticket = ticket_of(list.run_json_with(&agent(), "FILES_DELETE", A));
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        let pending = list.pending_confirmations();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].ticket, ticket);
        assert_eq!(pending[0].display_name, "Delete a file");
        assert_eq!(pending[0].args, r#"{"path":"a.txt"}"#);
        assert_eq!(pending[0].caller, "service:agent");
        // Still waiting: the same ticket again, and nothing runs.
        let again = list.run_json_with(&agent().with_ticket(&ticket), "FILES_DELETE", A);
        assert_eq!(ticket_of(again), ticket);
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        assert!(list.approve(&ticket));
        assert!(list.pending_confirmations().is_empty());
        let out = list.run_json_with(&agent().with_ticket(&ticket), "FILES_DELETE", A);
        assert_eq!(out.unwrap(), br#""a.txt""#);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        // Single use.
        let reused = list.run_json_with(&agent().with_ticket(&ticket), "FILES_DELETE", A);
        assert!(
            matches!(reused, Err(ActionError::Denied { reason, .. }) if reason.contains("already used"))
        );
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_ticket_is_bound_to_its_exact_call() {
        let (list, runs) = list(Duration::from_secs(60));
        let ticket = ticket_of(list.run_json_with(&agent(), "FILES_DELETE", A));
        assert!(list.approve(&ticket));
        let other_args = list.run_json_with(
            &agent().with_ticket(&ticket),
            "FILES_DELETE",
            br#"{"path":"b.txt"}"#,
        );
        let other_caller = list.run_json_with(
            &ActionCaller::new(Principal::Service {
                name: "other".to_owned(),
            })
            .with_ticket(&ticket),
            "FILES_DELETE",
            A,
        );
        for refused in [other_args, other_caller] {
            assert!(
                matches!(&refused, Err(ActionError::Denied { reason, .. }) if reason.contains("another call")),
                "{refused:?}"
            );
        }
        let made_up = list.run_json_with(&agent().with_ticket("nope"), "FILES_DELETE", A);
        assert!(matches!(made_up, Err(ActionError::Denied { .. })));
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        // The right call still runs with it.
        list.run_json_with(&agent().with_ticket(&ticket), "FILES_DELETE", A)
            .unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_denied_or_expired_ticket_is_refused() {
        let (list, runs) = list(Duration::from_millis(80));
        let denied = ticket_of(list.run_json_with(&agent(), "FILES_DELETE", A));
        assert!(list.deny(&denied));
        assert!(!list.approve(&denied), "a ticket is answered once");
        let out = list.run_json_with(&agent().with_ticket(&denied), "FILES_DELETE", A);
        assert!(
            matches!(out, Err(ActionError::Denied { reason, .. }) if reason.contains("declined"))
        );
        let late = ticket_of(list.run_json_with(&agent(), "FILES_DELETE", A));
        assert!(list.approve(&late));
        std::thread::sleep(Duration::from_millis(120));
        let out = list.run_json_with(&agent().with_ticket(&late), "FILES_DELETE", A);
        assert!(
            matches!(out, Err(ActionError::Denied { reason, .. }) if reason.contains("expired"))
        );
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        // Expired, a pending ticket can no longer be approved.
        let stale = ticket_of(list.run_json_with(&agent(), "FILES_DELETE", A));
        std::thread::sleep(Duration::from_millis(120));
        assert!(list.pending_confirmations().is_empty());
        assert!(!list.approve(&stale));
    }

    #[test]
    fn the_app_itself_and_what_need_not_ask_run_at_once() {
        let (list, runs) = list(Duration::from_secs(60));
        // A key, a menu: the person is the one running it.
        list.run_json("FILES_DELETE", A).unwrap();
        list.run_json_with(&ActionCaller::app(), "FILES_DELETE", A)
            .unwrap();
        // Mutating: an agent runs it without asking.
        list.run_json_with(&agent(), "FILES_RENAME", A).unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        assert!(list.pending_confirmations().is_empty());
        // Off, nothing asks.
        let (off, off_runs) = list_off();
        off.run_json_with(&agent(), "FILES_DELETE", A).unwrap();
        assert_eq!(off_runs.load(Ordering::SeqCst), 1);
    }

    fn list_off() -> (ActionList, Arc<AtomicUsize>) {
        let list = ActionList::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let ran = runs.clone();
        list.register(&DELETE, Effect::Destructive, move |_: Path| {
            ran.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert!(!list.requires_confirmation());
        (list, runs)
    }

    #[test]
    fn set_confirm_decides_per_call() {
        let (list, runs) = list(Duration::from_secs(60));
        // A mutating action that asks for one path only; a destructive
        // one that never asks.
        list.set_confirm("FILES_RENAME", |args| {
            std::str::from_utf8(args).is_ok_and(|a| a.contains("secret"))
        });
        list.set_confirm("FILES_DELETE", |_| false);
        list.run_json_with(&agent(), "FILES_RENAME", A).unwrap();
        list.run_json_with(&agent(), "FILES_DELETE", A).unwrap();
        let asked = list.run_json_with(&agent(), "FILES_RENAME", br#"{"path":"secret"}"#);
        ticket_of(asked);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        // Malformed arguments are refused before anyone is asked.
        let bad = list.run_json_with(&agent(), "FILES_RENAME", b"{\"path\":\"secret\"");
        assert!(matches!(bad, Err(ActionError::BadArgs { .. })));
        assert_eq!(list.pending_confirmations().len(), 1);
    }

    #[test]
    fn every_change_of_a_ticket_is_an_event() {
        let (list, _) = list(Duration::from_secs(60));
        let from = list.latest();
        let ticket = ticket_of(list.run_json_with(&agent(), "FILES_DELETE", A));
        assert!(list.approve(&ticket));
        let (events, _) = list.events_since(from, 16, Duration::ZERO);
        let told: Vec<String> = events
            .into_iter()
            .filter_map(|e| match e.kind {
                EventKind::Changed { topic, detail } if topic == super::CONFIRM_TOPIC => {
                    Some(detail)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            told,
            [format!("{ticket} pending"), format!("{ticket} approved")]
        );
    }

    #[test]
    fn an_async_action_asks_too() {
        let list = ActionList::new();
        list.register_async(
            &DELETE,
            Effect::Destructive,
            |p: Path| async move { Ok(p.path) },
        )
        .unwrap();
        list.require_confirmation(Duration::from_secs(60));
        let ticket = ticket_of(futures::executor::block_on(list.run_json_async_with(
            &agent(),
            "FILES_DELETE",
            A,
        )));
        assert!(list.approve(&ticket));
        let out = futures::executor::block_on(list.run_json_async_with(
            &agent().with_ticket(&ticket),
            "FILES_DELETE",
            A,
        ));
        assert_eq!(out.unwrap(), br#""a.txt""#);
    }
}
