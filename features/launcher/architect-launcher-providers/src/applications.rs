//! Desktop applications provider.
//!
//! Scans XDG .desktop files and indexes them for search.
//! Mirrors Elephant's `desktopapplications` provider.

use std::path::PathBuf;

use architect_launcher_core::{ActivationResult, Item, Provider, ProviderConfig};

pub struct ApplicationsProvider {
    config: ProviderConfig,
    apps: Vec<DesktopEntry>,
    icon_resolver: architect_launcher_core::icons::IconResolver,
}

#[derive(Debug, Clone)]
struct DesktopEntry {
    id: String,
    name: String,
    generic_name: String,
    comment: String,
    exec: String,
    icon: String,
    /// Resolved icon file path (PNG/SVG), or empty if not found.
    icon_path: String,
    keywords: Vec<String>,
    categories: Vec<String>,
    hidden: bool,
}

impl ApplicationsProvider {
    pub fn new() -> Self {
        Self {
            config: ProviderConfig {
                name: "applications".into(),
                icon: "application-x-executable".into(),
                prefix: Some('a'),
                ..Default::default()
            },
            apps: Vec::new(),
            icon_resolver: architect_launcher_core::icons::IconResolver::new(),
        }
    }

    /// Directories to scan for .desktop files.
    fn desktop_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        // XDG data dirs
        if let Ok(xdg_data) = std::env::var("XDG_DATA_DIRS") {
            for dir in xdg_data.split(':') {
                dirs.push(PathBuf::from(dir).join("applications"));
            }
        } else {
            dirs.push(PathBuf::from("/usr/share/applications"));
            dirs.push(PathBuf::from("/usr/local/share/applications"));
        }

        // User applications
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(&home).join(".local/share/applications"));
        }
        if let Ok(xdg_data_home) = std::env::var("XDG_DATA_HOME") {
            dirs.push(PathBuf::from(xdg_data_home).join("applications"));
        }

        dirs
    }

    fn scan_desktop_files(&mut self) {
        self.apps.clear();

        for dir in Self::desktop_dirs() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "desktop")
                    && let Some(app) = parse_desktop_file(&path)
                    && !app.hidden
                {
                    self.apps.push(app);
                }
            }
        }

        // Deduplicate by name (keep first seen, which is usually the more specific one)
        let mut seen = std::collections::HashSet::new();
        self.apps.retain(|app| seen.insert(app.name.clone()));

        // Resolve icon names to file paths
        let mut resolved = 0;
        for app in &mut self.apps {
            if !app.icon.is_empty()
                && let Some(path) = self.icon_resolver.resolve(&app.icon)
            {
                app.icon_path = path;
                resolved += 1;
            }
        }

        tracing::info!(
            count = self.apps.len(),
            icons = resolved,
            "Scanned desktop applications"
        );
    }
}

impl Default for ApplicationsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for ApplicationsProvider {
    fn name(&self) -> &str {
        "applications"
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn config_mut(&mut self) -> &mut ProviderConfig {
        &mut self.config
    }

    fn setup(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.scan_desktop_files();
        Ok(())
    }

    fn query(
        &self,
        _query: &str,
        _exact: bool,
    ) -> Result<Vec<Item>, Box<dyn std::error::Error + Send + Sync>> {
        // Return all apps — the engine handles scoring/filtering.
        let items = self
            .apps
            .iter()
            .map(|app| {
                let mut search_fields: Vec<String> = vec![app.name.clone()];
                if !app.generic_name.is_empty() {
                    search_fields.push(app.generic_name.clone());
                }
                if !app.comment.is_empty() {
                    search_fields.push(app.comment.clone());
                }
                if !app.keywords.is_empty() {
                    search_fields.push(app.keywords.join(" "));
                }
                if !app.categories.is_empty() {
                    search_fields.push(app.categories.join(" "));
                }

                // Convert .desktop categories to hierarchical tags
                let tags: Vec<&str> = vec!["desktop/applications"];
                let category_tags: Vec<String> = app
                    .categories
                    .iter()
                    .map(|c| format!("desktop/applications/{}", c.to_lowercase()))
                    .collect();
                let tag_refs: Vec<&str> = category_tags.iter().map(|s| s.as_str()).collect();

                let mut all_tags = tags.clone();
                all_tags.extend(tag_refs.iter());

                // Use resolved icon path if available, otherwise the icon name
                let icon = if !app.icon_path.is_empty() {
                    &app.icon_path
                } else {
                    &app.icon
                };

                Item::new(&app.id, &app.name, "applications")
                    .with_sub(&app.comment)
                    .with_icon(icon)
                    .with_search_fields(search_fields)
                    .with_tags(&all_tags)
                    .with_metadata(serde_json::json!({ "exec": app.exec }))
            })
            .collect();

        Ok(items)
    }

    fn activate(
        &self,
        item: &Item,
        _action: &str,
    ) -> Result<ActivationResult, Box<dyn std::error::Error + Send + Sync>> {
        let exec = item.metadata["exec"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        if exec.is_empty() {
            return Err("No exec command for this application".into());
        }

        // Strip field codes (%f, %F, %u, %U, etc.) from Exec line
        let cmd: String = exec
            .split_whitespace()
            .filter(|s| !s.starts_with('%'))
            .collect::<Vec<_>>()
            .join(" ");

        // Spawn detached
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        Ok(ActivationResult::Close)
    }
}

/// Parse a freedesktop .desktop file into a DesktopEntry.
fn parse_desktop_file(path: &PathBuf) -> Option<DesktopEntry> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut in_desktop_entry = false;
    let mut name = String::new();
    let mut generic_name = String::new();
    let mut comment = String::new();
    let mut exec = String::new();
    let mut icon = String::new();
    let mut keywords = Vec::new();
    let mut categories = Vec::new();
    let mut hidden = false;
    let mut no_display = false;
    let mut entry_type = String::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "Name" => name = value.to_string(),
                "GenericName" => generic_name = value.to_string(),
                "Comment" => comment = value.to_string(),
                "Exec" => exec = value.to_string(),
                "Icon" => icon = value.to_string(),
                "Type" => entry_type = value.to_string(),
                "Hidden" => hidden = value == "true",
                "NoDisplay" => no_display = value == "true",
                "Keywords" => {
                    keywords = value
                        .split(';')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                "Categories" => {
                    categories = value
                        .split(';')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                _ => {}
            }
        }
    }

    if entry_type != "Application" || name.is_empty() {
        return None;
    }

    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    Some(DesktopEntry {
        id,
        name,
        generic_name,
        comment,
        exec,
        icon,
        icon_path: String::new(), // Resolved later in scan_desktop_files
        keywords,
        categories,
        hidden: hidden || no_display,
    })
}
