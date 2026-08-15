//! Extension provider — serves items from loaded extensions.

use architect_launcher_core::extension::ExtensionRegistry;
use architect_launcher_core::{ActivationResult, Item, Provider, ProviderConfig};

pub struct ExtensionProvider {
    config: ProviderConfig,
    items: Vec<Item>,
}

impl ExtensionProvider {
    /// Create from an already-populated registry.
    pub fn from_registry(registry: &ExtensionRegistry) -> Self {
        Self {
            config: ProviderConfig {
                name: "extensions".into(),
                icon: "E".into(),
                prefix: Some('e'),
                ..Default::default()
            },
            items: registry.all_items(),
        }
    }
}

impl Provider for ExtensionProvider {
    fn name(&self) -> &str {
        "extensions"
    }
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn config_mut(&mut self) -> &mut ProviderConfig {
        &mut self.config
    }

    fn setup(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(items = self.items.len(), "Extension provider initialized");
        Ok(())
    }

    fn query(
        &self,
        _q: &str,
        _exact: bool,
    ) -> Result<Vec<Item>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.items.clone())
    }

    fn activate(
        &self,
        item: &Item,
        action: &str,
    ) -> Result<ActivationResult, Box<dyn std::error::Error + Send + Sync>> {
        let exec = item
            .actions
            .iter()
            .find(|a| a.name == action)
            .map(|a| a.exec.as_str())
            .unwrap_or("");

        if exec.is_empty() {
            return Ok(ActivationResult::Close);
        }

        let parts: Vec<&str> = exec.splitn(3, ':').collect();

        match parts.first().copied() {
            Some("ext") => {
                // Extension command: ext:{ext_name}:{command_name}
                tracing::info!(exec = exec, "Extension command invoked");
                // In the future, this dispatches to the extension's command handler.
                // For now, log it.
            }
            Some("sh") => {
                let cmd = parts.get(1).unwrap_or(&"");
                std::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;
            }
            Some("open") => {
                let target = parts.get(1).unwrap_or(&"");
                let cmd = if cfg!(target_os = "macos") {
                    "open"
                } else {
                    "xdg-open"
                };
                std::process::Command::new(cmd)
                    .arg(target)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;
            }
            Some("copy") => {
                let text = parts.get(1).unwrap_or(&"");
                let _ = std::process::Command::new("wl-copy").arg(text).spawn();
            }
            _ => {
                // Treat as shell command
                std::process::Command::new("sh")
                    .arg("-c")
                    .arg(exec)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;
            }
        }

        let keep_open = item
            .actions
            .iter()
            .find(|a| a.name == action)
            .map(|a| a.keep_open)
            .unwrap_or(false);

        if keep_open {
            Ok(ActivationResult::KeepOpen)
        } else {
            Ok(ActivationResult::Close)
        }
    }
}
