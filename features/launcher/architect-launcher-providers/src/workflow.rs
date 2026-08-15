//! Workflow provider — serves items from loaded workflow packs.
//!
//! This provider is the bridge between Styx pack files and the query engine.

use architect_launcher_core::pack;
use architect_launcher_core::{ActivationResult, Item, Provider, ProviderConfig};

pub struct WorkflowProvider {
    config: ProviderConfig,
    items: Vec<Item>,
}

impl WorkflowProvider {
    /// Create from pre-extracted items (packs already converted to Items).
    pub fn from_items(items: Vec<Item>) -> Self {
        Self {
            config: ProviderConfig {
                name: "workflows".into(),
                icon: "W".into(),
                prefix: Some('w'),
                ..Default::default()
            },
            items,
        }
    }

    /// Create by scanning the default pack directory.
    pub fn from_default_dir() -> Self {
        let dir = pack::default_pack_dir();
        let packs = pack::scan_packs(&dir);
        let items = packs.iter().flat_map(|p| p.to_items()).collect();
        Self::from_items(items)
    }
}

impl Default for WorkflowProvider {
    fn default() -> Self {
        Self::from_default_dir()
    }
}

impl Provider for WorkflowProvider {
    fn name(&self) -> &str {
        "workflows"
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn config_mut(&mut self) -> &mut ProviderConfig {
        &mut self.config
    }

    fn setup(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(items = self.items.len(), "Workflow provider initialized");
        Ok(())
    }

    fn query(
        &self,
        _query: &str,
        _exact: bool,
    ) -> Result<Vec<Item>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.items.clone())
    }

    fn activate(
        &self,
        item: &Item,
        action_name: &str,
    ) -> Result<ActivationResult, Box<dyn std::error::Error + Send + Sync>> {
        let action = item
            .actions
            .iter()
            .find(|a| a.name == action_name)
            .or_else(|| item.actions.first());

        let Some(action) = action else {
            return Ok(ActivationResult::Close);
        };

        execute_action(&action.exec)?;

        if action.keep_open {
            Ok(ActivationResult::KeepOpen)
        } else {
            Ok(ActivationResult::Close)
        }
    }
}

fn execute_action(exec: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (protocol, payload) = exec.split_once(':').unwrap_or(("sh", exec));

    match protocol {
        "sh" | "" => {
            std::process::Command::new("sh")
                .arg("-c")
                .arg(payload)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
        }
        "script" => {
            let path = std::path::Path::new(payload);
            let interpreter = match path.extension().and_then(|e| e.to_str()) {
                Some("lua") => "lua",
                Some("py") => "python3",
                Some("rb") => "ruby",
                Some("js") => "node",
                _ => "sh",
            };
            std::process::Command::new(interpreter)
                .arg(payload)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
        }
        "reaper" => {
            tracing::info!(action = payload, "Reaper action requested");
        }
        "ipc" => {
            if let Some((addr, msg)) = payload.split_once(':') {
                use std::io::Write;
                if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(addr) {
                    let _ = stream.write_all(msg.as_bytes());
                }
            }
        }
        "internal" => {
            tracing::debug!(action = payload, "Internal action");
        }
        other => {
            tracing::warn!(protocol = other, "Unknown action protocol");
        }
    }

    Ok(())
}
