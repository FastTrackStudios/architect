//! Extension API — what extensions can do.
//!
//! Provides a sandboxed API surface for extensions to interact with
//! the launcher and system. Similar to Raycast's `@raycast/api`.

use std::collections::HashMap;
use std::path::PathBuf;

/// API handle given to extension commands when they execute.
/// Provides access to launcher services in a sandboxed way.
pub struct ExtensionApi {
    ext_name: String,
    storage_dir: PathBuf,
}

impl ExtensionApi {
    pub fn new(ext_name: &str, storage_dir: PathBuf) -> Self {
        Self {
            ext_name: ext_name.to_string(),
            storage_dir,
        }
    }

    pub fn extension_name(&self) -> &str {
        &self.ext_name
    }

    // ── Clipboard ──────────────────────────────────────────

    /// Copy text to the system clipboard.
    pub fn clipboard_copy(&self, text: &str) -> Result<(), String> {
        // Try wl-copy (Wayland), then xclip (X11)
        let result = std::process::Command::new("wl-copy")
            .arg(text)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        match result {
            Ok(status) if status.success() => Ok(()),
            _ => {
                // Fallback to xclip
                let mut child = std::process::Command::new("xclip")
                    .args(["-selection", "clipboard"])
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("Failed to copy: {e}"))?;
                if let Some(stdin) = child.stdin.as_mut() {
                    use std::io::Write;
                    stdin
                        .write_all(text.as_bytes())
                        .map_err(|e| format!("{e}"))?;
                }
                child.wait().map_err(|e| format!("{e}"))?;
                Ok(())
            }
        }
    }

    // ── Notifications ──────────────────────────────────────

    /// Show a toast/notification.
    pub fn show_toast(&self, title: &str, message: &str) {
        let _ = std::process::Command::new("notify-send")
            .arg(title)
            .arg(message)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    // ── Open URLs/Files ────────────────────────────────────

    /// Open a URL or file with the default application.
    pub fn open(&self, target: &str) -> Result<(), String> {
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
            .spawn()
            .map_err(|e| format!("Failed to open: {e}"))?;
        Ok(())
    }

    // ── Per-Extension Storage ──────────────────────────────

    /// Get a value from per-extension local storage.
    pub fn storage_get(&self, key: &str) -> Option<String> {
        let path = self.storage_dir.join("storage.json");
        let data: HashMap<String, String> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        data.get(key).cloned()
    }

    /// Set a value in per-extension local storage.
    pub fn storage_set(&self, key: &str, value: &str) -> Result<(), String> {
        let path = self.storage_dir.join("storage.json");
        std::fs::create_dir_all(&self.storage_dir).map_err(|e| format!("{e}"))?;
        let mut data: HashMap<String, String> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        data.insert(key.to_string(), value.to_string());
        let json = serde_json::to_string_pretty(&data).map_err(|e| format!("{e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("{e}"))?;
        Ok(())
    }

    /// Remove a value from per-extension local storage.
    pub fn storage_remove(&self, key: &str) -> Result<(), String> {
        let path = self.storage_dir.join("storage.json");
        let mut data: HashMap<String, String> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        data.remove(key);
        let json = serde_json::to_string_pretty(&data).map_err(|e| format!("{e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("{e}"))?;
        Ok(())
    }

    /// List all keys in per-extension local storage.
    pub fn storage_keys(&self) -> Vec<String> {
        let path = self.storage_dir.join("storage.json");
        let data: HashMap<String, String> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        data.keys().cloned().collect()
    }

    // ── Shell Execution ────────────────────────────────────

    /// Execute a shell command and return its stdout.
    pub fn exec(&self, command: &str) -> Result<String, String> {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|e| format!("Failed to exec: {e}"))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }
}
