//! Session-token → [`Principal`] resolution for the permissions gate.
//!
//! The VALIDATING upgrade of `AuthServerMiddleware` (which only parses the
//! bearer string out of metadata): this resolver runs the real
//! `current_session` flow — token hash lookup, active + expiry checks — and
//! produces a [`Principal::User`] for the gate
//! (`architect::permissions_gate`). Invalid, expired, or absent tokens
//! resolve to [`Principal::Anonymous`]; the engines decide what anonymous
//! may do.
//!
//! Validation hits the session store, so results are cached per token for a
//! short TTL — a permissions check must not cost a DB round-trip per RPC.
//! The cache is deliberately small-dumb (token → principal, expiry stamp):
//! sign-out revocation propagates within `ttl`, which the gate's callers
//! accept (default 30 s; pass `Duration::ZERO` to disable).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use architect_permissions::{BoxIdentityFuture, IdentityResolver, Principal};

use crate::commands::CurrentSession;
use crate::storage::AuthStorage;
use crate::ArchitectAuth;

/// Resolve bearer tokens against an org's [`ArchitectAuth`] engine.
pub struct SessionIdentityResolver<S> {
    auth: ArchitectAuth<S>,
    ttl: Duration,
    cache: Mutex<HashMap<String, (Principal, Instant)>>,
}

impl<S> SessionIdentityResolver<S> {
    pub fn new(auth: ArchitectAuth<S>) -> Self {
        Self {
            auth,
            ttl: Duration::from_secs(30),
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    fn cached(&self, token: &str) -> Option<Principal> {
        if self.ttl.is_zero() {
            return None;
        }
        let mut cache = self.cache.lock().expect("identity cache poisoned");
        match cache.get(token) {
            Some((principal, at)) if at.elapsed() < self.ttl => Some(principal.clone()),
            Some(_) => {
                cache.remove(token);
                None
            }
            None => None,
        }
    }

    fn store(&self, token: &str, principal: &Principal) {
        if self.ttl.is_zero() {
            return;
        }
        let mut cache = self.cache.lock().expect("identity cache poisoned");
        // Bounded: drop everything once it grows silly (sessions per org are
        // small; this is belt-and-braces against token spray).
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert(token.to_string(), (principal.clone(), Instant::now()));
    }
}

impl<S> IdentityResolver for SessionIdentityResolver<S>
where
    S: AuthStorage,
{
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        Box::pin(async move {
            let Some(token) = bearer_token.filter(|t| !t.is_empty()) else {
                return Principal::Anonymous;
            };
            if let Some(hit) = self.cached(token) {
                return hit;
            }
            let principal = match self
                .auth
                .current_session(CurrentSession {
                    token: token.to_string(),
                })
                .await
            {
                Ok(bundle) => Principal::User {
                    user_id: bundle.user.id.to_string(),
                },
                Err(_) => Principal::Anonymous,
            };
            self.store(token, &principal);
            principal
        })
    }
}
