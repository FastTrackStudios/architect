//! Serving a node's endpoint so a revoked device loses its connections
//! (feature `iroh`, native).

use architect::iroh_link::{self, PeerUntil};

use crate::admission::Admission;

/// The closer for `admission`: what `iroh_link::serve_peers_until` and
/// `EngineHost::iroh_until` take, so a connection closes once its
/// device is let in no more ([`Admission::ends`]).
pub fn closer(
    admission: Admission,
) -> impl Fn(iroh::EndpointId) -> Option<PeerUntil> + Send + Sync + 'static {
    move |remote| {
        let admission = admission.clone();
        let endpoint = remote.to_string();
        let until: PeerUntil = Box::pin(async move { admission.ends(&endpoint).await });
        Some(until)
    }
}

/// Serve `endpoint`: each connection the handler `make` builds from the
/// endpoint id it proved (`iroh_link::serve_peers`), closed when that
/// device is revoked from `admission`.
pub async fn serve<H, F>(endpoint: &iroh::Endpoint, admission: Admission, make: F)
where
    F: Fn(iroh::EndpointId) -> H + Send + Sync + 'static,
    H: architect::vox::Handler<architect::vox::DriverReplySink> + Clone + Send + Sync + 'static,
{
    iroh_link::serve_peers_until(endpoint, make, closer(admission)).await;
}
