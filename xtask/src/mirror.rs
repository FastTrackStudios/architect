//! `cargo xtask auth-mirror` — pull a sanitised copy of a running
//! server's database into a local one.
//!
//! The sanitising happens on the *server*, in `auth_db::snapshot`, so
//! this command never holds a credential it then has to be trusted to
//! throw away. What arrives over the wire is already free of them.
//!
//! Which makes this file's job small and worth being strict about:
//! authenticate as an administrator, write the file, and load it into
//! something local. The credential is read from the environment rather
//! than an argument, because an argument lands in shell history and in
//! the process table where any other user on the machine can read it.

use std::path::Path;
use std::process::Command;

/// The environment variable holding the administrator's session token.
pub const TOKEN_VAR: &str = "AUTH_ADMIN_TOKEN";

/// Fetch a snapshot of `from` with `token`: an authenticated GET of
/// `/admin/snapshot`, nothing else. The token goes to curl on its stdin
/// (`--header @-`), never on its command line, where any user of the
/// machine could read it in the process table. `timeout` bounds the
/// whole call (`devenv up` never waits on production for long).
///
/// # Errors
///
/// curl failed, the server refused, or what came back is not a snapshot.
pub fn fetch(
    from: &str,
    token: &str,
    redact_emails: bool,
    timeout: Option<std::time::Duration>,
) -> Result<Vec<u8>, String> {
    use std::io::Write as _;
    use std::process::Stdio;

    let base = from.trim_end_matches('/');
    let url = if redact_emails {
        format!("{base}/admin/snapshot?redact_emails=true")
    } else {
        format!("{base}/admin/snapshot")
    };

    // curl rather than a HTTP client dependency: xtask is repo tooling
    // that should compile in seconds, and this is one GET.
    let mut curl = Command::new("curl");
    curl.arg("--silent")
        .arg("--show-error")
        .arg("--fail-with-body")
        .arg("--location")
        .arg("--proto")
        .arg("=https,http")
        // The token goes in a header, never the URL (a URL reaches every
        // proxy log on the path), and to curl on stdin.
        .arg("--header")
        .arg("@-");
    if let Some(timeout) = timeout {
        curl.arg("--connect-timeout")
            .arg("5")
            .arg("--max-time")
            .arg(timeout.as_secs().max(1).to_string());
    }
    let mut child = curl
        .arg(&url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("running curl: {err}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("authorization: Bearer {}\n", token.trim()).as_bytes())
            .map_err(|err| format!("handing curl the header: {err}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|err| format!("running curl: {err}"))?;

    if !output.status.success() {
        let body = String::from_utf8_lossy(&output.stdout);
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "fetching {url}: {}{}{}",
            output.status,
            if body.trim().is_empty() {
                String::new()
            } else {
                format!("\n  {}", body.trim().chars().take(200).collect::<String>())
            },
            if err.trim().is_empty() {
                String::new()
            } else {
                format!("\n  {}", err.trim())
            },
        ));
    }

    // A 403 answered as a JSON body would otherwise be written to disk
    // as though it were a snapshot.
    let text = String::from_utf8_lossy(&output.stdout);
    if !text.contains("\"users\"") || !text.contains("\"organizations\"") {
        return Err(format!(
            "{url} did not answer with a snapshot. The first 200 bytes were:\n  {}",
            text.chars().take(200).collect::<String>()
        ));
    }
    Ok(output.stdout)
}

/// Fetch a snapshot and write it to `out`.
pub fn run(from: &str, out: &Path, redact_emails: bool) -> Result<(), String> {
    let token = std::env::var(TOKEN_VAR).map_err(|_| {
        format!(
            "set {TOKEN_VAR} to an administrator's session token.\n\
             \n\
             Get one by signing in to {from} as an administrator and copying the token from\n\
             the `{cookie}` cookie, or:\n\
             \n\
             \x20 curl -sS {from}/auth/sign-in/email \\\n\
             \x20   -H 'content-type: application/json' \\\n\
             \x20   -d '{{\"input\":{{\"email\":\"you@example.com\",\"password\":\"…\"}}}}' | jq -r .token\n\
             \n\
             It is read from the environment rather than taken as an argument so it does not\n\
             land in shell history or the process table.",
            cookie = "architect_session",
        )
    })?;

    let snapshot = fetch(from, &token, redact_emails, None)?;

    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("creating {}: {err}", parent.display()))?;
    }
    std::fs::write(out, &snapshot).map_err(|err| format!("writing {}: {err}", out.display()))?;

    println!("wrote {} ({} bytes)", out.display(), snapshot.len());
    println!(
        "\nLoad it into a local server with:\n\
         \x20 AUTH_DATABASE_URL=sqlite://./auth-local.db?mode=rwc \\\n\
         \x20 AUTH_IMPORT_SNAPSHOT={} \\\n\
         \x20 cargo run -p auth-server",
        out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::TOKEN_VAR;

    #[test]
    fn the_token_variable_is_the_one_the_help_text_names() {
        // The message a person reads when they have not set it must
        // name the variable that is actually read.
        assert_eq!(TOKEN_VAR, "AUTH_ADMIN_TOKEN");
    }
}
