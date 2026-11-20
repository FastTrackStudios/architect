//! A project's files the org's node holds, for the extensions in this
//! page (`file_list`, `file_url`; `architect_extensions::held`): the
//! signed-in member's own, through the node's `architect_files::held` face
//! with the bearer the org lane holds ([`super::org::OrgVaults::held_list`]).
//! The node decides what the person may read; the host decides which
//! extension may ask (`reads = ["files"]`). A share link's guest and the
//! demo get none: their host has no source, and answers `unsupported`.

use std::cell::RefCell;

use architect_extensions::held::{HeldFilesSource, HeldFuture};
use architect_extensions::hostcall::HostError;
use serde_json::{Value, json};

use super::org::{HeldRefusal, OrgVaults};

thread_local! {
    /// The signed-in org's lane: a browser has one thread, and the lane
    /// is not `Send`, so the host's source finds it here.
    static ORG: RefCell<Option<OrgVaults>> = const { RefCell::new(None) };
}

/// The host's source of a project's files: the signed-in org's node.
pub struct WebHeldFiles;

impl WebHeldFiles {
    /// The source over `org`'s lane, for the page's one host.
    #[must_use]
    pub fn over(org: OrgVaults) -> Self {
        ORG.with(|o| *o.borrow_mut() = Some(org));
        Self
    }
}

fn org() -> Result<OrgVaults, HostError> {
    ORG.with(|o| o.borrow().clone())
        .ok_or_else(architect_extensions::held::unsupported)
}

fn host_error(e: HeldRefusal) -> HostError {
    match e {
        HeldRefusal::Refused(fault) => match architect::http::HttpError::status(&fault) {
            401 | 403 => HostError::Refused {
                capability: "files".to_owned(),
                why: fault.to_string(),
            },
            404 => HostError::NotFound {
                what: fault.to_string(),
            },
            _ => HostError::Failed {
                message: fault.to_string(),
            },
        },
        HeldRefusal::Unreachable => HostError::Failed {
            message: "the org's node did not answer".to_owned(),
        },
    }
}

impl HeldFilesSource for WebHeldFiles {
    fn list(&self, place: String, folder: Option<String>) -> HeldFuture {
        Box::pin(async move {
            let listing = org()?.held_list(place, folder).await.map_err(host_error)?;
            let files: Vec<Value> = listing
                .files
                .into_iter()
                .map(|f| json!({"path": f.path, "size": f.size, "sha256": f.sha256, "url": f.url}))
                .collect();
            Ok(json!({
                "place": listing.place,
                "folder": listing.folder,
                "files": files,
                "expires_us": listing.expires_us,
            }))
        })
    }

    fn url(&self, place: String, path: String) -> HeldFuture {
        Box::pin(async move {
            let ticket = org()?.held_url(place, path).await.map_err(host_error)?;
            Ok(json!({"url": ticket.url, "expires_us": ticket.expires_us}))
        })
    }
}
