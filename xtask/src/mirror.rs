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
             \x20   -d '{{\"email\":\"you@example.com\",\"password\":\"…\"}}' | jq -r .token\n\
             \n\
             It is read from the environment rather than taken as an argument so it does not\n\
             land in shell history or the process table.",
            cookie = "architect_session",
        )
    })?;

    let base = from.trim_end_matches('/');
    let url = if redact_emails {
        format!("{base}/admin/snapshot?redact_emails=true")
    } else {
        format!("{base}/admin/snapshot")
    };

    // curl rather than a HTTP client dependency: xtask is repo tooling
    // that should compile in seconds, and this is one GET.
    let output = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--fail-with-body")
        .arg("--location")
        // The token goes in a header, never the URL: a URL reaches every
        // proxy log on the path.
        .arg("--header")
        .arg(format!("authorization: Bearer {token}"))
        .arg(&url)
        .output()
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
                format!("\n  {}", body.trim())
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

    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("creating {}: {err}", parent.display()))?;
    }
    std::fs::write(out, &output.stdout)
        .map_err(|err| format!("writing {}: {err}", out.display()))?;

    println!("wrote {} ({} bytes)", out.display(), output.stdout.len());
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
