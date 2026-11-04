//! The manager as an extension: linked into the app as the built-in
//! `manager`, its actions (`MANAGER_*`) and its view (`manager`, the
//! page) its own, overridable by a development copy of its id.
//!
//! ```ignore
//! let manager = Manager::new().with(ExtensionKind::new(browse)).with(ThemeKind::new(store, list));
//! let extensions = extensions.with_linked(&ManagerExtension::new(manager.clone()))?;
//! ```

use serde_json::{Value, json};

use super::Manager;
use crate::linked::{Builder, Extension};

/// Its manifest.
pub const MANIFEST: &str = include_str!("manager.toml");

/// The manager, as a `linked::Extension`.
#[derive(Clone, Debug)]
pub struct ManagerExtension {
    manager: Manager,
}

impl ManagerExtension {
    #[must_use]
    pub const fn new(manager: Manager) -> Self {
        Self { manager }
    }
}

fn s<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

impl Extension for ManagerExtension {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        let m = self.manager.clone();
        ext.action("MANAGER_KINDS", move |_| {
            let out = m.summary();
            async move { Ok(out) }
        });
        let m = self.manager.clone();
        ext.action("MANAGER_LIST", move |args| {
            let items = m.list(s(&args, "kind"), s(&args, "query").unwrap_or_default());
            async move { Ok(json!({ "items": items })) }
        });
        let m = self.manager.clone();
        ext.action("MANAGER_GET", move |args| {
            let (kind, id) = (
                s(&args, "kind").unwrap_or_default(),
                s(&args, "id").unwrap_or_default(),
            );
            let out = m
                .get(kind, id)
                .map(|d| serde_json::to_value(d).unwrap_or(Value::Null))
                .ok_or_else(|| format!("no {kind} {id} (MANAGER_LIST lists them)"));
            async move { out }
        });
        let m = self.manager.clone();
        ext.action("MANAGER_RUN", move |args| {
            let (kind, id, op) = (
                s(&args, "kind").unwrap_or_default(),
                s(&args, "id").unwrap_or_default(),
                s(&args, "op").unwrap_or_default(),
            );
            let out = m.run(kind, id, op, args.get("args").unwrap_or(&Value::Null));
            async move { out }
        });
        #[cfg(feature = "page")]
        ext.view("manager", crate::page::ManagerView);
    }
}
