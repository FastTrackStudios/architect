//! A stub language server for tests (feature `lsp-stub`): it speaks
//! `initialize`, publishes one diagnostic for each document it is sent
//! (a warning over the first two bytes, `stub: a warning`), and answers
//! one hover (`stub hover`), one completion (`stub_completion`), a
//! format (a comment line put first), a rename (the first two bytes) and
//! a definition (the start of the document). So a test runs the host's
//! supervision and a `CodeEditor`'s wiring with no real server installed.
//!
//! [`serve`] runs it on stdin and stdout; a test binary runs itself as
//! the server (`harness = false`, a `--stub-lsp` argument). With
//! `--die-once <file>` it exits after its first document while `<file>`
//! does not exist, creating it: one crash, for restart tests.

use std::io::{BufRead, BufReader, Write};

use serde_json::{Value, json};

/// The diagnostic's message.
pub const DIAGNOSTIC: &str = "stub: a warning";
/// The hover's text.
pub const HOVER: &str = "stub hover";
/// The completion's label.
pub const COMPLETION: &str = "stub_completion";
/// The line formatting puts first.
pub const FORMATTED: &str = "// formatted\n";

fn read(input: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(n) = line.strip_prefix("Content-Length:") {
            length = n.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn write(out: &mut impl Write, message: &Value) {
    let body = message.to_string();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

const fn range(start: u32, end: u32) -> (u32, u32) {
    (start, end)
}

fn lsp_range((start, end): (u32, u32)) -> Value {
    json!({"start": {"line": 0, "character": start}, "end": {"line": 0, "character": end}})
}

/// Runs the stub on stdin and stdout until `exit` or end of input.
/// `args` are the process's arguments after `--stub-lsp`.
pub fn serve(args: &[String]) {
    let die_once = args
        .iter()
        .position(|a| a == "--die-once")
        .and_then(|i| args.get(i.saturating_add(1)))
        .map(std::path::PathBuf::from);
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    while let Some(message) = read(&mut input) {
        let method = message["method"].as_str().unwrap_or_default();
        let id = message.get("id").cloned();
        let params = &message["params"];
        let reply = |out: &mut std::io::StdoutLock<'_>, result: Value| {
            if let Some(id) = &id {
                write(out, &json!({"jsonrpc": "2.0", "id": id, "result": result}));
            }
        };
        match method {
            "initialize" => reply(
                &mut out,
                json!({"capabilities": {
                    "textDocumentSync": 2,
                    "hoverProvider": true,
                    "completionProvider": {},
                    "documentFormattingProvider": true,
                    "renameProvider": true,
                    "definitionProvider": true,
                }, "serverInfo": {"name": "stub-lsp"}}),
            ),
            "textDocument/didOpen" | "textDocument/didChange" => {
                let doc = &params["textDocument"];
                let version = doc["version"].clone();
                write(
                    &mut out,
                    &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
                        "uri": doc["uri"],
                        "version": version,
                        "diagnostics": [{
                            "range": lsp_range(range(0, 2)),
                            "severity": 2,
                            "source": "stub",
                            "message": DIAGNOSTIC,
                        }],
                    }}),
                );
                if method == "textDocument/didOpen"
                    && let Some(marker) = &die_once
                    && !marker.exists()
                {
                    let _ = std::fs::write(marker, "died");
                    std::process::exit(3);
                }
            }
            "textDocument/hover" => reply(
                &mut out,
                json!({"contents": {"kind": "markdown", "value": HOVER}}),
            ),
            "textDocument/completion" => reply(
                &mut out,
                json!([{"label": COMPLETION, "detail": "from the stub", "insertText": COMPLETION}]),
            ),
            "textDocument/formatting" => reply(
                &mut out,
                json!([{"range": lsp_range(range(0, 0)), "newText": FORMATTED}]),
            ),
            "textDocument/rename" => {
                let uri = params["textDocument"]["uri"].clone();
                let name = params["newName"].clone();
                let mut changes = serde_json::Map::new();
                changes.insert(
                    uri.as_str().unwrap_or_default().to_owned(),
                    json!([{"range": lsp_range(range(0, 2)), "newText": name}]),
                );
                reply(&mut out, json!({"changes": changes}));
            }
            "textDocument/definition" => reply(
                &mut out,
                json!({"uri": params["textDocument"]["uri"], "range": lsp_range(range(0, 0))}),
            ),
            "shutdown" => reply(&mut out, Value::Null),
            "exit" => return,
            _ => {
                if id.is_some() {
                    reply(&mut out, Value::Null);
                }
            }
        }
    }
}
