//! The desk app's list over MCP, when asked: with `ARCHITECT_MCP_PORT`
//! set, a model reaches it on that loopback port (`ARCHITECT_MCP_TOKEN`
//! its bearer token, if any) — and meets the confirmation sheet when it
//! deletes (`apps/ui/src/confirm.rs`). The lab's `ARCHITECT_LAB_MCP_PORT`
//! and `ARCHITECT_LAB_MCP_TOKEN` are still read. Off otherwise: an app
//! nobody asked to serve hands no one its list.

use std::sync::Arc;

use architect::action::{ActionList, Principal};

/// The variables that turn the endpoint on, and its port: the first set.
const PORT_ENVS: [&str; 2] = ["ARCHITECT_MCP_PORT", "ARCHITECT_LAB_MCP_PORT"];
/// Its bearer token, when set, beside the port's variable.
const TOKEN_ENVS: [&str; 2] = ["ARCHITECT_MCP_TOKEN", "ARCHITECT_LAB_MCP_TOKEN"];

/// Serve `list` on a thread of its own for the app's life, when
/// one of [`PORT_ENVS`] is set. Its calls are made as `service:mcp`: a caller
/// from outside, which the list asks a person about.
pub fn serve(list: &Arc<ActionList>) {
    let Some((port_env, token_env)) = PORT_ENVS
        .into_iter()
        .zip(TOKEN_ENVS)
        .find(|(port, _)| std::env::var_os(port).is_some())
    else {
        return;
    };
    let list = ActionList::clone(list);
    let started = std::thread::Builder::new()
        .name("app-mcp".to_owned())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                tracing::error!("the app's MCP runtime did not start");
                return;
            };
            runtime.block_on(async move {
                let server = architect_mcp::Server::new(
                    architect_app::distribution().folder_name(),
                    "0",
                    list,
                )
                .with_principal(Principal::Service {
                    name: "mcp".to_owned(),
                });
                match server.serve_loopback(port_env, token_env).await {
                    Ok(Some(serving)) => {
                        tracing::info!(url = %serving.url, "the app serves MCP");
                        let _ = serving.task.await;
                    }
                    Ok(None) => {}
                    Err(error) => tracing::error!(%error, "the app's MCP did not bind"),
                }
            });
        });
    if let Err(error) = started {
        tracing::error!(%error, "the app's MCP thread did not start");
    }
}
