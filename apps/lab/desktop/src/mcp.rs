//! The desk lab's list over MCP, when asked: with `ARCHITECT_LAB_MCP_PORT`
//! set, a model reaches it on that loopback port (`ARCHITECT_LAB_MCP_TOKEN`
//! its bearer token, if any) — and meets the confirmation sheet when it
//! deletes (`apps/lab/ui/src/confirm.rs`). Off otherwise: a lab nobody
//! asked to serve hands no one its list.

use std::sync::Arc;

use architect::action::{ActionList, Principal};

/// The variable that turns the endpoint on, and its port.
const PORT_ENV: &str = "ARCHITECT_LAB_MCP_PORT";
/// Its bearer token, when set.
const TOKEN_ENV: &str = "ARCHITECT_LAB_MCP_TOKEN";

/// Serve `list` on a thread of its own for the app's life, when
/// [`PORT_ENV`] is set. Its calls are made as `service:mcp`: a caller
/// from outside, which the list asks a person about.
pub fn serve(list: &Arc<ActionList>) {
    if std::env::var_os(PORT_ENV).is_none() {
        return;
    }
    let list = ActionList::clone(list);
    let started = std::thread::Builder::new()
        .name("lab-mcp".to_owned())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                tracing::error!("the lab's MCP runtime did not start");
                return;
            };
            runtime.block_on(async move {
                let server = architect_mcp::Server::new("architect-lab", "0", list).with_principal(
                    Principal::Service {
                        name: "mcp".to_owned(),
                    },
                );
                match server.serve_loopback(PORT_ENV, TOKEN_ENV).await {
                    Ok(Some(serving)) => {
                        tracing::info!(url = %serving.url, "the lab serves MCP");
                        let _ = serving.task.await;
                    }
                    Ok(None) => {}
                    Err(error) => tracing::error!(%error, "the lab's MCP did not bind"),
                }
            });
        });
    if let Err(error) = started {
        tracing::error!(%error, "the lab's MCP thread did not start");
    }
}
