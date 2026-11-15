//! The `Meals` client Mealplan's actions and views hold.
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
use crate::service::{MealplanError, MealsClient};

struct Held {
    handle: Option<Handle>,
    // A browser's client is not `Send` (vox's runtime there is
    // single-threaded): it is kept per thread, in [`LOCAL`].
    #[cfg(not(target_family = "wasm"))]
    client: Option<MealsClient>,
    #[cfg(not(target_family = "wasm"))]
    scope: Option<Arc<architect::resource::Scope>>,
}

static HELD: Mutex<Held> = Mutex::new(Held {
    handle: None,
    #[cfg(not(target_family = "wasm"))]
    client: None,
    #[cfg(not(target_family = "wasm"))]
    scope: None,
});

#[cfg(target_family = "wasm")]
thread_local! {
    static LOCAL: std::cell::RefCell<Option<(MealsClient, Arc<architect::resource::Scope>)>> =
        const { std::cell::RefCell::new(None) };
}

fn held() -> std::sync::MutexGuard<'static, Held> {
    HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The client made last, if any.
fn cached() -> Option<MealsClient> {
    #[cfg(not(target_family = "wasm"))]
    {
        held().client.clone()
    }
    #[cfg(target_family = "wasm")]
    {
        LOCAL.with(|l| l.borrow().as_ref().map(|(c, _)| c.clone()))
    }
}

/// Keeps `client`, served for as long as `scope` lives (`None`: lets
/// it go).
fn keep(made: Option<(MealsClient, Arc<architect::resource::Scope>)>) {
    #[cfg(not(target_family = "wasm"))]
    {
        let mut h = held();
        let (client, scope) = made.unzip();
        h.client = client;
        h.scope = scope;
    }
    #[cfg(target_family = "wasm")]
    {
        LOCAL.with(|l| *l.borrow_mut() = made);
    }
}

/// The host this copy was built against: the client is made over it
/// when first asked for.
pub fn connect(handle: Handle) {
    held().handle = Some(handle);
    keep(None);
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
            .thread_name("mealplan")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// A client of Mealplan over `files`, served in process for as long
/// as `scope` lives.
///
/// # Errors
///
/// No runtime, or the session could not be made.
pub async fn serve(
    files: Files,
    scope: &Arc<architect::resource::Scope>,
) -> Result<MealsClient, MealplanError> {
    let local = architect::serve_local(Backend::new(files), scope);
    #[cfg(not(target_family = "wasm"))]
    {
        runtime()
            .map_err(MealplanError::Failed)?
            .spawn(async move { local.establish::<MealsClient>().await })
            .await
            .map_err(|e| MealplanError::Failed(e.to_string()))?
            .map_err(|e| MealplanError::Failed(e.to_string()))
    }
    #[cfg(target_family = "wasm")]
    {
        local
            .establish::<MealsClient>()
            .await
            .map_err(|e| MealplanError::Failed(e.to_string()))
    }
}

/// The client over the host's vault: made the first time.
///
/// # Errors
///
/// Mealplan is not running, or the service could not be served.
pub async fn client() -> Result<MealsClient, MealplanError> {
    if let Some(c) = cached() {
        return Ok(c);
    }
    let handle = held()
        .handle
        .clone()
        .ok_or_else(|| MealplanError::Failed("Mealplan is not running".into()))?;
    let scope = architect::resource::Scope::new();
    let client = serve(Files::Host(handle), &scope).await?;
    keep(Some((client.clone(), scope)));
    Ok(client)
}
