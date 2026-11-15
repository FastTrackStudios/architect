//! The `Recipes` client the cookbook's actions and views hold.
//!
//! The service is served in process (`architect::serve_local`) over the
//! vault the host gives: on a desk and a phone on a small runtime of its
//! own, in a browser on the page's thread. Made once per [`connect`],
//! again after the extension is built anew (a reload, a development
//! copy).

use std::sync::{Arc, Mutex, PoisonError};

use architect_extensions::linked::Handle;

use crate::backend::Backend;
use crate::files::Files;
use crate::service::{CookbookError, RecipesClient};

struct Held {
    handle: Option<Handle>,
    client: Option<RecipesClient>,
    scope: Option<Arc<architect::resource::Scope>>,
}

static HELD: Mutex<Held> = Mutex::new(Held {
    handle: None,
    client: None,
    scope: None,
});

fn held() -> std::sync::MutexGuard<'static, Held> {
    HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The host this copy was built against: the client is made over it
/// when first asked for.
pub fn connect(handle: Handle) {
    let mut h = held();
    h.handle = Some(handle);
    h.client = None;
    h.scope = None;
}

/// The host's handle, for what is not the service's (opening a view, a
/// notification).
#[must_use]
pub fn handle() -> Option<Handle> {
    held().handle.clone()
}

/// The runtime the in-process service runs on, natively.
#[cfg(not(target_family = "wasm"))]
fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RT: std::sync::OnceLock<Result<tokio::runtime::Runtime, String>> =
        std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("cookbook")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// A client of the cookbook over `files`, served in process for as long
/// as `scope` lives.
///
/// # Errors
///
/// No runtime, or the session could not be made.
pub async fn serve(
    files: Files,
    scope: &Arc<architect::resource::Scope>,
) -> Result<RecipesClient, CookbookError> {
    let local = architect::serve_local(Backend::new(files), scope);
    #[cfg(not(target_family = "wasm"))]
    {
        runtime()
            .map_err(CookbookError::Failed)?
            .spawn(async move { local.establish::<RecipesClient>().await })
            .await
            .map_err(|e| CookbookError::Failed(e.to_string()))?
            .map_err(|e| CookbookError::Failed(e.to_string()))
    }
    #[cfg(target_family = "wasm")]
    {
        local
            .establish::<RecipesClient>()
            .await
            .map_err(|e| CookbookError::Failed(e.to_string()))
    }
}

/// The client over the host's vault: made the first time.
///
/// # Errors
///
/// The cookbook is not running, or the service could not be served.
pub async fn client() -> Result<RecipesClient, CookbookError> {
    let handle = {
        let h = held();
        if let Some(c) = &h.client {
            return Ok(c.clone());
        }
        h.handle
            .clone()
            .ok_or_else(|| CookbookError::Failed("the cookbook is not running".into()))?
    };
    let scope = architect::resource::Scope::new();
    let client = serve(Files::Host(handle), &scope).await?;
    {
        let mut h = held();
        h.client = Some(client.clone());
        h.scope = Some(scope);
    }
    Ok(client)
}
