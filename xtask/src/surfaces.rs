//! `cargo xtask surfaces` — everything on MCP is on the command line too,
//! and the two say the same.
//!
//! The rule (Cody's): an app's Action List is both CLI- and
//! MCP-accessible. Both faces answer from one `architect_control::Surface`
//! (`features/control`), so for a given server they agree by construction;
//! what can still drift is an *app*: one that serves MCP and never mounts
//! the `actions` CLI or its control socket, or the reverse, or one whose
//! parity was never checked against its real list. So, over every
//! workspace member's sources:
//!
//! 1. a crate whose `src/` serves MCP (`serve_stdio`, `serve_stdio_via`,
//!    `serve_loopback`, `McpHost`, `http::router`, `http::serve`) also
//!    serves the CLI (`architect_control`'s `Cli::new` / `Control`), and
//!    the reverse;
//! 2. such a crate has a parity test (`architect_mcp::parity`'s
//!    `Parity::new`, in `src/` or `tests/`), which compares the identical
//!    set of actions and the identical answers over both;
//! 3. MCP's generic tools are the control protocol's methods
//!    (`features/mcp/src/lib.rs` builds them from `Method::TOOLS`), not a
//!    table of its own.
//!
//! An app with no stdio of its own (the iOS lab) may instead say, in its
//! `src/`, `// surfaces: mcp via <member dir>`: its MCP face is that
//! member's bridge onto its socket. That member must itself pass 1 and 2
//! and serve the bridge (`serve_stdio_via`).
//!
//! The faces themselves (`features/mcp`, `features/control`) are not apps
//! and are skipped. Task and studio run the same check in their own repos
//! by having the same parity test.

use std::path::{Path, PathBuf};

/// What serving MCP looks like in a crate's sources.
const MCP_SERVES: &[&str] = &[
    "serve_stdio(",
    "serve_stdio_via(",
    "serve_loopback",
    "McpHost",
    "http::router(",
    "http::serve(",
];

/// What serving the CLI looks like: the `actions` command or the socket.
const CLI_SERVES: &[&str] = &["Cli::new(", "Control::new(", "Control::at("];

/// How an app names the member whose bridge is its MCP face.
const MCP_VIA: &str = "// surfaces: mcp via ";

/// The faces, not apps.
const FACES: &[&str] = &["features/mcp", "features/control"];

/// One crate's sources, as the rules read them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Sources {
    /// Workspace-relative directory.
    pub dir: String,
    /// Everything under `src/`.
    pub src: String,
    /// Everything under `tests/`.
    pub tests: String,
    /// Its manifest up to `[dev-dependencies]`: what it ships with.
    pub deps: String,
}

impl Sources {
    /// Whether it serves MCP: ships with `architect-mcp` and serves it.
    fn serves_mcp(&self) -> bool {
        self.deps.contains("architect-mcp") && MCP_SERVES.iter().any(|p| self.src.contains(p))
    }

    /// Whether it serves the CLI: ships with `architect-control` and
    /// mounts its command or socket.
    fn serves_cli(&self) -> bool {
        self.deps.contains("architect-control") && CLI_SERVES.iter().any(|p| self.src.contains(p))
    }

    /// The member it names as its MCP face (`// surfaces: mcp via <dir>`).
    fn mcp_via(&self) -> Option<&str> {
        let (_, rest) = self.src.split_once(MCP_VIA)?;
        rest.lines().next().map(str::trim).filter(|d| !d.is_empty())
    }

    /// Whether it has a parity test.
    fn has_parity(&self) -> bool {
        self.src.contains("Parity::new(") || self.tests.contains("Parity::new(")
    }
}

/// The rules over `crates`, and MCP's own source: each problem, one line.
#[must_use]
pub fn problems(crates: &[Sources], mcp_lib: &str) -> Vec<String> {
    let mut out = Vec::new();
    for c in crates {
        if FACES.contains(&c.dir.as_str()) {
            continue;
        }
        let mcp = c.serves_mcp();
        let cli = c.serves_cli();
        if cli
            && !mcp
            && let Some(via) = c.mcp_via()
        {
            let bridge = crates.iter().find(|o| o.dir == via);
            match bridge {
                Some(b) if b.serves_mcp() && b.serves_cli() && b.has_parity() && b.src.contains("serve_stdio_via(") => {}
                _ => out.push(format!(
                    "{}: its MCP face `{via}` must be a member serving both faces, a parity test and `serve_stdio_via`",
                    c.dir
                )),
            }
            continue;
        }
        if mcp && !cli {
            out.push(format!(
                "{}: serves MCP but not the CLI — mount `architect_control::cli::Cli` (and its control socket)",
                c.dir
            ));
        }
        if cli && !mcp {
            out.push(format!(
                "{}: serves the CLI but not MCP — serve `architect_mcp` too (`bridge::serve_stdio_via` for a running app)",
                c.dir
            ));
        }
        if (mcp || cli) && !c.has_parity() {
            out.push(format!(
                "{}: no parity test — `architect_mcp::parity::Parity::new(&server).check()` over its list",
                c.dir
            ));
        }
    }
    if !mcp_lib.contains("Method::TOOLS") || mcp_lib.contains(r#""name": "actions""#) {
        out.push(
            "features/mcp/src/lib.rs: MCP's generic tools must be built from \
             `architect_control::protocol::Method::TOOLS`, not a table of their own"
                .to_owned(),
        );
    }
    out
}

/// Run the check from the repo root.
///
/// # Errors
///
/// Every problem found, or a manifest that could not be read.
pub fn run_cli(root: &Path) -> Result<(), String> {
    let crates = members(root)?
        .into_iter()
        .map(|dir| Sources {
            src: read_rs(&root.join(&dir).join("src")),
            tests: read_rs(&root.join(&dir).join("tests")),
            deps: shipped_deps(&root.join(&dir).join("Cargo.toml")),
            dir,
        })
        .collect::<Vec<_>>();
    let mcp_lib = std::fs::read_to_string(root.join("features/mcp/src/lib.rs"))
        .map_err(|e| format!("features/mcp/src/lib.rs: {e}"))?;
    let found = problems(&crates, &mcp_lib);
    let apps: Vec<&str> = crates
        .iter()
        .filter(|c| !FACES.contains(&c.dir.as_str()))
        .filter(|c| c.serves_mcp())
        .map(|c| c.dir.as_str())
        .collect();
    if found.is_empty() {
        println!(
            "surfaces: {} app(s) serve MCP and the CLI, each with a parity test: {}",
            apps.len(),
            apps.join(", ")
        );
        Ok(())
    } else {
        Err(format!(
            "{} surface problem(s):\n- {}",
            found.len(),
            found.join("\n- ")
        ))
    }
}

/// The workspace's members, from the root manifest.
fn members(root: &Path) -> Result<Vec<String>, String> {
    let text =
        std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?;
    let manifest: toml::Value = toml::from_str(&text).map_err(|e| format!("Cargo.toml: {e}"))?;
    Ok(manifest
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(toml::Value::as_array)
        .map(|m| {
            m.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

/// A manifest's text up to its `[dev-dependencies]`.
fn shipped_deps(manifest: &Path) -> String {
    let text = std::fs::read_to_string(manifest).unwrap_or_default();
    text.split("[dev-dependencies]")
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Every `.rs` file under `dir`, concatenated.
fn read_rs(dir: &Path) -> String {
    let mut out = String::new();
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "rs")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                out.push_str(&text);
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Sources, problems};

    fn app(src: &str, tests: &str) -> Sources {
        Sources {
            dir: "apps/x".into(),
            src: src.into(),
            tests: tests.into(),
            deps: "architect-mcp architect-control".into(),
        }
    }

    const MCP_LIB: &str = "Method::TOOLS.iter()";

    #[test]
    fn both_faces_and_a_parity_test_pass() {
        let ok = app(
            "architect_mcp::serve_stdio(&s); architect_control::cli::Cli::new(\"x\")",
            "Parity::new(&server).check()",
        );
        assert_eq!(problems(&[ok], MCP_LIB), Vec::<String>::new());
    }

    #[test]
    fn one_face_alone_fails() {
        let mcp_only = app("architect_mcp::serve_stdio(&s)", "Parity::new(&s)");
        let found = problems(&[mcp_only], MCP_LIB);
        assert!(found[0].contains("not the CLI"), "{found:?}");
        let cli_only = app(
            "architect_control::server::Control::new(\"x\", s)",
            "Parity::new(&s)",
        );
        let found = problems(&[cli_only], MCP_LIB);
        assert!(found[0].contains("not MCP"), "{found:?}");
    }

    #[test]
    fn no_parity_test_fails() {
        let untested = app("serve_stdio(&s); architect_control Cli::new(\"x\")", "");
        let found = problems(&[untested], MCP_LIB);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("no parity test"), "{found:?}");
    }

    #[test]
    fn an_app_whose_mcp_is_another_members_bridge_passes() {
        let desk = Sources {
            dir: "apps/desk".into(),
            ..app(
                "serve_stdio_via(&p); serve_stdio(&s); Control::new(\"d\")",
                "Parity::new(&s)",
            )
        };
        let phone = Sources {
            dir: "apps/phone".into(),
            ..app("Control::new(\"p\")\n// surfaces: mcp via apps/desk\n", "")
        };
        assert!(problems(&[desk, phone], MCP_LIB).is_empty());
    }

    #[test]
    fn a_via_that_is_not_a_bridge_fails() {
        let other = Sources {
            dir: "apps/other".into(),
            ..app("", "")
        };
        let phone = Sources {
            dir: "apps/phone".into(),
            ..app("Control::new(\"p\")\n// surfaces: mcp via apps/other\n", "")
        };
        assert_eq!(problems(&[other, phone], MCP_LIB).len(), 1);
        let missing = Sources {
            dir: "apps/phone".into(),
            ..app("Control::new(\"p\")\n// surfaces: mcp via apps/none\n", "")
        };
        assert_eq!(problems(&[missing], MCP_LIB).len(), 1);
    }

    #[test]
    fn a_hand_written_tool_table_fails() {
        let found = problems(&[], r#"json!({"name": "actions"})"#);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn the_faces_themselves_are_skipped() {
        let face = Sources {
            dir: "features/mcp".into(),
            src: "serve_stdio(".into(),
            tests: String::new(),
            deps: "architect-mcp".into(),
        };
        assert!(problems(&[face], MCP_LIB).is_empty());
        // A crate that does not ship with architect-mcp serves no MCP,
        // whatever it calls `http::serve(`.
        let other = Sources {
            dir: "crates/architect".into(),
            src: "http::serve(".into(),
            tests: String::new(),
            deps: String::new(),
        };
        assert!(problems(&[other], MCP_LIB).is_empty());
    }
}
