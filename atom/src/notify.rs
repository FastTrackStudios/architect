//! [`Notifications`] — an app-wide queue for failures (and other notices)
//! that outlive the screen that caused them.
//!
//! The gap it closes: an optimistic delete/update usually **navigates away
//! immediately**. If the server then rejects the call, the store rolls the
//! row back — but the page that owned the error signal has unmounted, so
//! the message had nowhere to go. With a `Notifications` context provided
//! at the app root, [`Mutation::run`](crate::Mutation::run) reports
//! rollback failures here automatically; the shell renders the queue once
//! (a toast tray / banner strip) and every page benefits.
//!
//! Queue semantics the tray can rely on:
//!
//! - **TTL**: every notice carries `ttl_ms` (level-dependent default;
//!   `None` = sticky). Expiry is the *renderer's* job — this crate has no
//!   timer source that works on every target, so the tray arms one task
//!   per `(id, count)` and calls [`Notifications::dismiss_if`].
//! - **Dedupe**: pushing a message identical to one already queued (same
//!   level) bumps that notice's `count` and re-arms its TTL instead of
//!   stacking a duplicate — a flapping connection produces ONE toast with
//!   a ×N badge, not a wall.
//! - **Cap**: the queue keeps at most [`MAX_NOTICES`]; the oldest drop
//!   first.
//!
//! ```ignore
//! // app root:
//! provide_notifications();
//! // shell chrome:
//! let notices = use_notifications();
//! for n in notices.list() { /* render + n.id for dismiss */ }
//! ```

use dioxus::prelude::*;

/// Most notices kept at once — oldest drop first past this.
pub const MAX_NOTICES: usize = 6;

/// Severity of a notice. Errors come from failed mutations; `Info` /
/// `Success` are for app use (e.g. "copied to clipboard", "Reconnected").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoticeLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl NoticeLevel {
    /// Default time-on-screen for this severity, in ms.
    #[must_use]
    pub fn default_ttl_ms(self) -> u32 {
        match self {
            Self::Success => 3_000,
            Self::Info => 4_000,
            Self::Warning => 6_000,
            Self::Error => 8_000,
        }
    }
}

/// One queued notice.
#[derive(Clone, PartialEq, Debug)]
pub struct Notice {
    /// Stable id for dismissal.
    pub id: u64,
    pub level: NoticeLevel,
    pub message: String,
    /// How long the tray should keep this on screen. `None` = sticky
    /// (manual dismiss only).
    pub ttl_ms: Option<u32>,
    /// How many times this exact notice has been pushed while queued —
    /// the tray shows a ×N badge past 1, and each bump re-arms the TTL.
    pub count: u32,
}

/// A `Copy` handle to the app-wide notice queue.
pub struct Notifications {
    items: Signal<Vec<Notice>>,
    seq: Signal<u64>,
}

impl Clone for Notifications {
    fn clone(&self) -> Self {
        *self
    }
}
impl Copy for Notifications {}

impl Notifications {
    /// Queue an error notice. Returns the notice id (for programmatic
    /// dismissal).
    pub fn error(&self, message: impl Into<String>) -> u64 {
        self.push(NoticeLevel::Error, message)
    }

    /// Queue an info notice.
    pub fn info(&self, message: impl Into<String>) -> u64 {
        self.push(NoticeLevel::Info, message)
    }

    /// Queue a success notice.
    pub fn success(&self, message: impl Into<String>) -> u64 {
        self.push(NoticeLevel::Success, message)
    }

    /// Queue a warning notice.
    pub fn warning(&self, message: impl Into<String>) -> u64 {
        self.push(NoticeLevel::Warning, message)
    }

    fn push(&self, level: NoticeLevel, message: impl Into<String>) -> u64 {
        self.push_with(level, message, Some(level.default_ttl_ms()))
    }

    /// Queue a notice with an explicit TTL (`None` = sticky until
    /// dismissed). The plain level methods use per-level defaults.
    pub fn push_with(
        &self,
        level: NoticeLevel,
        message: impl Into<String>,
        ttl_ms: Option<u32>,
    ) -> u64 {
        let message = message.into();
        let mut items = self.items;
        // Dedupe: an identical queued message bumps instead of stacking.
        {
            let mut list = items.write();
            if let Some(existing) = list
                .iter_mut()
                .find(|n| n.level == level && n.message == message)
            {
                existing.count += 1;
                existing.ttl_ms = ttl_ms;
                return existing.id;
            }
        }
        let mut seq = self.seq;
        let id = *seq.peek() + 1;
        seq.set(id);
        let mut list = items.write();
        list.push(Notice {
            id,
            level,
            message,
            ttl_ms,
            count: 1,
        });
        let overflow = list.len().saturating_sub(MAX_NOTICES);
        if overflow > 0 {
            list.drain(..overflow);
        }
        id
    }

    /// Remove one notice by id.
    pub fn dismiss(&self, id: u64) {
        let mut items = self.items;
        items.write().retain(|n| n.id != id);
    }

    /// Remove `id` only if its `count` still equals `count` — the tray's
    /// TTL tasks use this so a notice that was bumped (deduped push)
    /// after the timer was armed survives until its NEW timer fires.
    pub fn dismiss_if(&self, id: u64, count: u32) {
        let mut items = self.items;
        items
            .write()
            .retain(|n| n.id != id || n.count != count);
    }

    /// Remove everything.
    pub fn clear(&self) {
        let mut items = self.items;
        items.write().clear();
    }

    /// Snapshot of the queue (reactive read — re-renders on change).
    pub fn list(&self) -> Vec<Notice> {
        self.items.read().clone()
    }

    /// Non-reactive snapshot (for timers / event handlers).
    pub fn list_now(&self) -> Vec<Notice> {
        self.items.peek().clone()
    }
}

/// Provide the queue at the app root. Mutations created below this point
/// report their rollback failures into it automatically.
pub fn provide_notifications() -> Notifications {
    let items = use_signal(Vec::new);
    let seq = use_signal(|| 0u64);
    use_context_provider(|| Notifications { items, seq })
}

/// Pull the queue anywhere under the provider.
pub fn use_notifications() -> Notifications {
    use_context::<Notifications>()
}

/// Like [`use_notifications`] but tolerant of a missing provider —
/// [`crate::use_mutation`] (and the derive-emitted CRDT mutations) use
/// this so writes work (minus auto-report) in apps that haven't opted in.
pub fn try_use_notifications() -> Option<Notifications> {
    use_hook(try_consume_context::<Notifications>)
}
