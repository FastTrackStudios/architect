//! A router served in this process, each client on a connection of its
//! own whose serving side lives as long as the `Local` does.

use std::sync::Mutex;

use auth::vox_host::{VoxRouter, local};

pub struct Local {
    router: VoxRouter,
    serving: Mutex<Vec<local::Serving>>,
}

impl Local {
    pub fn serve(router: VoxRouter) -> Self {
        Self {
            router,
            serving: Mutex::new(Vec::new()),
        }
    }

    pub async fn establish<C: vox_core::FromVoxLane>(&self) -> Result<C, String> {
        let (client, serving) = local::establish(&self.router).await?;
        self.serving
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(serving);
        Ok(client)
    }
}
