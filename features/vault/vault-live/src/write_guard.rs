//! Who may write a vault file — a question the host answers.
//!
//! The backend knows files, not people. Some shelves are governed by
//! rules the vault cannot see: a wiki whose Editors have been declared
//! takes direct writes from its Editors only, and everyone else's change
//! goes through an Edit Request (`wiki.edit.editor`). The web editor
//! saves a wiki page through [`VaultSync`](vault_proto::VaultSync), not
//! through the wiki lane, so that rule has to hold here too — or the
//! editor would be a way around it.
//!
//! So the host hands the backend a [`WriteGuard`], and the backend asks
//! it before every write a caller can make: `put_file`, `delete_file`,
//! `set_folder`, and `open_collab` (joining a file's live session is how
//! the collaborative editor writes).
//!
//! # Knowing who asked
//!
//! The permissions gate records the caller in a task-local, and the
//! backend runs its sync methods on tokio's blocking pool, where that
//! task-local is gone. [`CallerDispatcher`] reads the caller on the
//! request task, before the hop, and carries it onto the blocking thread
//! for the length of the call; [`current_caller`] reads it back.
//!
//! A call with no caller is the server acting on its own behalf — the
//! collab write-behind flushing a document, a seed, a migration — and a
//! guard lets it through, the same rule the wiki lane applies: the lane
//! governs *people*, and the server is not one.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;

use architect::dispatch::{BoxedAny, DispatchError, Dispatcher};

/// The host's answer to "may this caller write this file?".
pub trait WriteGuard: Send + Sync + 'static {
    /// `Ok` to allow; `Err` with the reason to refuse. `principal` is the
    /// account the gate resolved, or `None` for an in-process call.
    fn check(&self, vault_id: &str, path: &str, principal: Option<&str>) -> Result<(), String>;
}

thread_local! {
    static CALLER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The account whose call is running on this thread, as
/// [`CallerDispatcher`] carried it here. `None` off a dispatched call.
#[must_use]
pub fn current_caller() -> Option<String> {
    CALLER.with(|c| c.borrow().clone())
}

/// The account the permissions gate resolved for the call on this task.
#[must_use]
pub fn gate_principal() -> Option<String> {
    account_of(architect::permissions_gate::caller().as_ref()).map(str::to_owned)
}

/// The account `who` is, for a [`WriteGuard`]: a signed-in user's id;
/// `None` for everyone else (a node, the server itself), whom the guard
/// does not govern.
#[must_use]
pub fn account_of(who: Option<&architect_permissions::Principal>) -> Option<&str> {
    match who {
        Some(architect_permissions::Principal::User { user_id }) => Some(user_id),
        _ => None,
    }
}

/// Clears the carried caller when the call ends, however it ends.
struct CallerScope(Option<String>);

impl CallerScope {
    fn enter(who: Option<String>) -> Self {
        Self(CALLER.with(|c| c.replace(who)))
    }
}

impl Drop for CallerScope {
    fn drop(&mut self) {
        let previous = self.0.take();
        CALLER.with(|c| *c.borrow_mut() = previous);
    }
}

/// `TokioBlockingDispatcher`, carrying the gate's caller across the hop.
#[derive(Default)]
pub struct CallerDispatcher;

impl Dispatcher for CallerDispatcher {
    fn dispatch(
        &self,
        f: Box<dyn FnOnce() -> BoxedAny + Send + 'static>,
    ) -> Pin<Box<dyn Future<Output = Result<BoxedAny, DispatchError>> + Send + 'static>> {
        // Read on the request task, where the gate set it.
        let who = gate_principal();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let _scope = CallerScope::enter(who);
                f()
            })
            .await
            .map_err(|join_err| {
                if join_err.is_panic() {
                    DispatchError::Panicked(format!("{join_err:?}"))
                } else {
                    DispatchError::Cancelled
                }
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_carries_the_caller_and_restores_what_was_there() {
        assert_eq!(current_caller(), None);
        {
            let _outer = CallerScope::enter(Some("alice".into()));
            assert_eq!(current_caller().as_deref(), Some("alice"));
            {
                let _inner = CallerScope::enter(Some("sam".into()));
                assert_eq!(current_caller().as_deref(), Some("sam"));
            }
            assert_eq!(current_caller().as_deref(), Some("alice"));
        }
        assert_eq!(current_caller(), None);
    }
}

/// Who may write what extensions keep in a vault (#710): a share link's
/// extensions (`_architect/shares/<code>.extensions.toml` and the
/// packages in `_architect/shares/<code>.extensions/`) only the share's
/// owner; a person's installs (`.architect/people/<id>/`) only that
/// person. Every other path, and a call with no account (the server
/// itself), is left to the rest of the rules.
pub struct ExtensionPaths {
    /// The account that owns share `code` (an audience's code: its
    /// share's) in vault `vault`; `None` for no such share.
    share_owner: Box<dyn Fn(&str, &str) -> Option<String> + Send + Sync>,
}

impl ExtensionPaths {
    #[must_use]
    pub fn new(share_owner: impl Fn(&str, &str) -> Option<String> + Send + Sync + 'static) -> Self {
        Self {
            share_owner: Box::new(share_owner),
        }
    }
}

/// The share code a path under `_architect/shares/` keeps extensions for.
fn share_extensions_code(path: &str) -> Option<&str> {
    let name = path.strip_prefix("_architect/shares/")?.split('/').next()?;
    name.strip_suffix(".extensions.toml")
        .or_else(|| name.strip_suffix(".extensions"))
}

impl WriteGuard for ExtensionPaths {
    fn check(&self, vault_id: &str, path: &str, principal: Option<&str>) -> Result<(), String> {
        let Some(who) = principal else {
            return Ok(());
        };
        if let Some(rest) = path.strip_prefix(".architect/people/") {
            let person = rest.split('/').next().unwrap_or_default();
            return if person == who {
                Ok(())
            } else {
                Err(format!("{path}: only {person} writes their installs"))
            };
        }
        if let Some(code) = share_extensions_code(path) {
            return match (self.share_owner)(vault_id, code) {
                Some(owner) if owner == who => Ok(()),
                Some(_) => Err(format!(
                    "{path}: only the share's owner chooses what its link runs"
                )),
                None => Err(format!("{path}: there is no share {code}")),
            };
        }
        Ok(())
    }
}

#[cfg(test)]
mod extension_paths_tests {
    use super::{ExtensionPaths, WriteGuard};

    #[test]
    fn only_a_shares_owner_writes_what_its_link_runs() {
        let g = ExtensionPaths::new(|_, code| (code == "abc").then(|| "alice".to_owned()));
        for path in [
            "_architect/shares/abc.extensions.toml",
            "_architect/shares/abc.extensions/x-1.0.0.archext",
        ] {
            assert!(g.check("v", path, Some("alice")).is_ok());
            assert!(
                g.check("v", path, Some("mallory"))
                    .unwrap_err()
                    .contains("owner")
            );
            assert!(g.check("v", path, None).is_ok(), "the server itself");
        }
        assert!(
            g.check("v", "_architect/shares/zzz.extensions.toml", Some("alice"))
                .is_err()
        );
        assert!(
            g.check("v", "_architect/shares/abc.md", Some("mallory"))
                .is_ok(),
            "not this rule's"
        );
        assert!(g.check("v", "notes/a.md", Some("mallory")).is_ok());
    }

    #[test]
    fn only_a_person_writes_their_installs() {
        let g = ExtensionPaths::new(|_, _| None);
        let path = ".architect/people/alice/extensions.toml";
        assert!(g.check("v", path, Some("alice")).is_ok());
        assert!(
            g.check("v", path, Some("mallory"))
                .unwrap_err()
                .contains("only alice")
        );
        assert!(
            g.check(
                "v",
                ".architect/people/alice/extensions.toml.minisig",
                Some("mallory")
            )
            .is_err()
        );
        assert!(
            g.check("v", ".architect/extensions.toml", Some("mallory"))
                .is_ok(),
            "not this rule's"
        );
    }
}
