//! Icon resolution system.
//!
//! Resolves icon identifiers to file paths. Supports:
//! - Absolute file paths (returned as-is)
//! - XDG icon theme names (looked up in standard icon directories)
//! - Custom icon registry (for plugin icons, tag icons, etc.)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Icon resolver with caching.
#[derive(Debug, Clone, Default)]
pub struct IconResolver {
    /// Cache of icon name -> resolved file path.
    cache: HashMap<String, Option<String>>,
    /// Custom icon overrides: name -> path.
    custom: HashMap<String, String>,
    /// Icon theme directories to search.
    search_dirs: Vec<PathBuf>,
}

impl IconResolver {
    pub fn new() -> Self {
        let mut resolver = Self::default();
        resolver.search_dirs = Self::xdg_icon_dirs();
        resolver
    }

    /// Register a custom icon: name -> absolute path.
    pub fn register(&mut self, name: impl Into<String>, path: impl Into<String>) {
        self.custom.insert(name.into(), path.into());
    }

    /// Resolve an icon identifier to a file path (or None).
    ///
    /// Resolution order:
    /// 1. Empty string -> None
    /// 2. Custom registry lookup
    /// 3. Already an absolute path to an existing file -> return as-is
    /// 4. XDG icon theme lookup (search standard directories)
    /// 5. None (not found)
    pub fn resolve(&mut self, icon: &str) -> Option<String> {
        if icon.is_empty() {
            return None;
        }

        // Check cache
        if let Some(cached) = self.cache.get(icon) {
            return cached.clone();
        }

        let result = self.resolve_uncached(icon);
        self.cache.insert(icon.to_string(), result.clone());
        result
    }

    fn resolve_uncached(&self, icon: &str) -> Option<String> {
        // Custom registry
        if let Some(path) = self.custom.get(icon)
            && Path::new(path).exists()
        {
            return Some(path.clone());
        }

        // Already a file path
        if icon.starts_with('/') && Path::new(icon).exists() {
            return Some(icon.to_string());
        }

        // XDG icon theme lookup
        self.find_in_theme(icon)
    }

    /// Search XDG icon theme directories for an icon by name.
    fn find_in_theme(&self, name: &str) -> Option<String> {
        // Common sizes to search, largest first (better quality)
        let sizes = [
            "256x256", "128x128", "96x96", "64x64", "48x48", "32x32", "scalable",
        ];
        let categories = [
            "apps",
            "applications",
            "devices",
            "mimetypes",
            "actions",
            "categories",
        ];
        let extensions = ["svg", "png", "xpm"];

        for dir in &self.search_dirs {
            // Try hicolor theme first (most common)
            for size in &sizes {
                for category in &categories {
                    for ext in &extensions {
                        let path = dir
                            .join("hicolor")
                            .join(size)
                            .join(category)
                            .join(format!("{name}.{ext}"));
                        if path.exists() {
                            return Some(path.to_string_lossy().to_string());
                        }
                    }
                }
            }

            // Try Adwaita theme
            for size in &sizes {
                for category in &categories {
                    for ext in &extensions {
                        let path = dir
                            .join("Adwaita")
                            .join(size)
                            .join(category)
                            .join(format!("{name}.{ext}"));
                        if path.exists() {
                            return Some(path.to_string_lossy().to_string());
                        }
                    }
                }
            }

            // Try breeze theme (KDE)
            for size in &sizes {
                for category in &categories {
                    for ext in &extensions {
                        let path = dir
                            .join("breeze")
                            .join(size)
                            .join(category)
                            .join(format!("{name}.{ext}"));
                        if path.exists() {
                            return Some(path.to_string_lossy().to_string());
                        }
                    }
                }
            }

            // Try pixmaps (flat directory)
            for ext in &extensions {
                let path = dir.join(format!("{name}.{ext}"));
                if path.exists() {
                    return Some(path.to_string_lossy().to_string());
                }
            }
        }

        // Also check /usr/share/pixmaps
        for ext in &extensions {
            let path = PathBuf::from("/usr/share/pixmaps").join(format!("{name}.{ext}"));
            if path.exists() {
                return Some(path.to_string_lossy().to_string());
            }
        }

        None
    }

    /// Standard XDG icon directories.
    fn xdg_icon_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        // User icons
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(&home).join(".local/share/icons"));
            dirs.push(PathBuf::from(&home).join(".icons"));
        }

        // System icons from XDG_DATA_DIRS
        if let Ok(xdg_data) = std::env::var("XDG_DATA_DIRS") {
            for dir in xdg_data.split(':') {
                dirs.push(PathBuf::from(dir).join("icons"));
            }
        } else {
            dirs.push(PathBuf::from("/usr/share/icons"));
            dirs.push(PathBuf::from("/usr/local/share/icons"));
        }

        // Nix-specific paths
        if let Ok(xdg_data) = std::env::var("XDG_DATA_DIRS") {
            for dir in xdg_data.split(':') {
                let icons = PathBuf::from(dir).join("icons");
                if icons.exists() && !dirs.contains(&icons) {
                    dirs.push(icons);
                }
            }
        }

        dirs
    }

    /// Get cache statistics.
    pub fn cache_stats(&self) -> (usize, usize) {
        let total = self.cache.len();
        let found = self.cache.values().filter(|v| v.is_some()).count();
        (found, total)
    }
}
