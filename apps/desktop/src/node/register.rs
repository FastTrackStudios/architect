//! `architect-desktop serve register`: put a machine with no browser of
//! its own on an account, as a storage server.
//!
//! A storage server is the app run with `serve` on a machine that holds
//! the files (`files_dir`, `files_hold`), signed in to the person's
//! account like their other devices. Being on the account is what makes
//! it one of theirs: the account's other devices let it in, sync their
//! documents with it and fetch bytes from it, and it is listed with them
//! (`ACCOUNT_DEVICES`, platform `storage`) and among the holders
//! (`FILES_HOLDERS`, at its `files_site`). Where each file then lives is
//! each node's holding rule (#904, `Config::hold_rule`).
//!
//! Until now getting such a machine a session meant signing in somewhere
//! else and copying the token by hand. [`sign_in`] is the device
//! authorization grant, as the app's own "sign in with a code" is: the
//! machine shows a link and a code, its person approves it from any
//! signed-in browser, and the session is written to the node's
//! `session_file` ([`write_session`], owner-only). Nothing here prints
//! the session.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use super::Config;
use super::org::IssuerLane;

/// What the machine's person is shown: where to approve, and the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// The issuer's page, the code filled in.
    pub url: String,
    /// The code, to check against what the page shows.
    pub code: String,
    /// How long the code lasts.
    pub expires_in: Duration,
}

/// Sign this machine in at `issuer` as the device `name`: `shown` is
/// handed the link and the code once, then this waits until its person
/// approves it. The session token, to be written with [`write_session`].
///
/// # Errors
///
/// The issuer did not answer, its person refused, or the code ran out.
pub async fn sign_in(
    issuer: &str,
    name: &str,
    shown: impl FnOnce(&Prompt),
) -> Result<String, String> {
    let origin = issuer.trim_end_matches('/');
    let lane = IssuerLane::new(origin);
    let auth = lane
        .client::<auth_proto::AuthServiceClient>()
        .await
        .map_err(|e| format!("the issuer {origin}: {e}"))?;
    let started = auth
        .start_device_sign_in(name.to_owned())
        .await
        .map_err(|e| format!("the issuer did not start a sign-in: {e}"))?;
    let secs = |n: i64| Duration::from_secs(u64::try_from(n).unwrap_or(0));
    let prompt = Prompt {
        url: if started.verification_uri_complete.starts_with('/') {
            format!("{origin}{}", started.verification_uri_complete)
        } else {
            started.verification_uri_complete.clone()
        },
        code: started.user_code.clone(),
        expires_in: secs(started.expires_in_seconds),
    };
    shown(&prompt);
    let started_at = Instant::now();
    let mut every = secs(started.interval_seconds).max(Duration::from_secs(1));
    loop {
        tokio::time::sleep(every).await;
        if started_at.elapsed() >= prompt.expires_in {
            return Err("the code ran out before it was approved".to_owned());
        }
        match auth
            .poll_device_sign_in(started.device_code.clone(), Some(name.to_owned()))
            .await
        {
            Ok(bundle) => return Ok(bundle.token),
            Err(e) => {
                // The issuer's answers while it waits, by name: the error
                // is the issuer's own enum inside the wire's.
                let why = format!("{e:?}");
                if why.contains("slow_down") {
                    every = every.saturating_add(Duration::from_secs(1));
                } else if why.contains("PermissionDenied") {
                    return Err("the sign-in was refused".to_owned());
                } else if why.contains("InvalidCredentials") {
                    return Err("the code ran out before it was approved".to_owned());
                } else if !why.contains("VerificationRequired") {
                    return Err(format!("the issuer: {e}"));
                }
            }
        }
    }
}

/// Write the session `token` to `path`, readable by its owner alone,
/// through a temp file and a rename.
///
/// # Errors
///
/// The folder or the file cannot be written.
pub fn write_session(path: &Path, token: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("part");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

fn say(text: &str) {
    let _ = writeln!(std::io::stdout(), "{text}");
}

fn fail(text: &str) -> ExitCode {
    let _ = writeln!(
        std::io::stderr(),
        "architect-desktop serve register: {text}"
    );
    ExitCode::FAILURE
}

/// `serve register [--issuer <url>] [--session-file <path>] [--name <name>]`.
///
/// The issuer, the session file and the name default to `config`'s
/// (`issuer`, `session_file` else `<data dir>/session`, `name`).
#[must_use]
pub fn command(config: &Config, args: &[String]) -> ExitCode {
    let mut issuer = config.issuer.clone();
    let mut file = config.session_file.clone();
    let mut name = config.name.clone();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--issuer", Some(v)) => issuer = Some(v.clone()),
            ("--session-file", Some(v)) => file = Some(PathBuf::from(v)),
            ("--name", Some(v)) => name.clone_from(v),
            ("--help" | "-h", _) => {
                say(
                    "architect-desktop serve [--config <file>] register \\\n    \
                     [--issuer <url>] [--session-file <path>] [--name <name>]\n\n\
                     Sign this machine in to your account with a code (no browser\n\
                     needed here), and write its session where the node reads it.\n\
                     Then run it with `serve` as a storage server: see\n\
                     docs/content/guide/deploy.md, \"A storage server\".",
                );
                return ExitCode::SUCCESS;
            }
            (other, _) => return fail(&format!("unknown or incomplete argument {other}")),
        }
    }
    let Some(issuer) = issuer else {
        return fail("no issuer: pass --issuer <url> or set ARCHITECT_ISSUER");
    };
    let file = file.unwrap_or_else(|| config.data_dir.join("session"));
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return fail(&error.to_string()),
    };
    let signed = runtime.block_on(sign_in(&issuer, &name, |prompt| {
        say(&format!(
            "To put this machine ({name}) on your account, open\n\n  {}\n\n\
             in a browser where you are signed in, and check the code is\n\n  {}\n\n\
             Waiting up to {} minutes for you to approve it…",
            prompt.url,
            prompt.code,
            prompt.expires_in.as_secs().div_ceil(60)
        ));
    }));
    let token = match signed {
        Ok(token) => token,
        Err(why) => return fail(&why),
    };
    if let Err(error) = write_session(&file, &token) {
        return fail(&format!("writing {}: {error}", file.display()));
    }
    say(&format!(
        "\nSigned in. The session is in {} (readable by you alone).\n\n\
         To run this machine as a storage server of the account, set\n\n  \
         ARCHITECT_ISSUER={issuer}\n  \
         ARCHITECT_SESSION_FILE={}\n  \
         ARCHITECT_PLATFORM=storage\n  \
         ARCHITECT_FILES_DIR=<the tree it holds>\n  \
         ARCHITECT_FILES_HOLD=true\n\n\
         and start `architect-desktop serve`. It is then listed with the\n\
         account's devices and among the files' holders.",
        file.display(),
        file.display(),
    ));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_is_written_for_its_owner_alone_and_replaces_the_last() {
        let dir = std::env::temp_dir().join(format!("node-register-{}", std::process::id()));
        let file = dir.join("deep").join("session");
        assert!(write_session(&file, "first").is_ok());
        assert!(write_session(&file, "second").is_ok());
        assert_eq!(
            std::fs::read_to_string(&file).ok().as_deref(),
            Some("second")
        );
        assert!(!file.with_extension("part").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&file).map(|m| m.permissions().mode() & 0o777);
            assert_eq!(mode.ok(), Some(0o600));
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
