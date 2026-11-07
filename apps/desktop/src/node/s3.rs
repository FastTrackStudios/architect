//! The node's roots over S3 (`architect_files::sync::s3`), on
//! [`Config::s3_bind`](super::Config::s3_bind).
//!
//! A bucket per org (its folder under `files_dir`), keys
//! `Projects/<project>/…` and `Assets/<kind>/…`. Access keys are kept in
//! `<data_dir>/files/s3-keys.json` and issued on the node with
//! `architect-desktop serve s3-key issue`. A device's key holds what its
//! grant in the node's `Admission` holds now (a revoked device's key
//! opens nothing); a person's, the role it was issued with, until it is
//! revoked.

use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use architect_devices::{Admission, Grant};
use architect_files::sync::Engine;
use architect_files::sync::s3::{KeyHolder, S3Face, S3Key, S3Keys};
use architect_orgs::Role;

use super::Config;

/// The keys of the node whose data dir is `data`.
pub(super) fn keys(data: &Path) -> S3Keys {
    S3Keys::in_dir(&data.join("files"))
}

/// What a key's holder holds on this node now.
pub(super) fn role_now(admission: &Admission, key: &S3Key) -> Option<Role> {
    match &key.holder {
        KeyHolder::User(_) => Some(key.role.clone()),
        KeyHolder::Device(endpoint) => admission
            .get(endpoint)?
            .grants
            .iter()
            .filter_map(|g| match g {
                Grant::Account => Some(Role::Owner),
                Grant::Org { role, .. } => Some(role.clone()),
                Grant::Paired { .. } => None,
            })
            .min(),
    }
}

/// Serve the S3 face on `bind`.
pub(super) async fn serve(
    engine: Engine,
    data: &Path,
    admission: Admission,
    bind: &str,
) -> std::io::Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    let face = S3Face::new(engine, keys(data), move |key: &S3Key| {
        role_now(&admission, key)
    });
    Ok((addr, tokio::spawn(face.serve(listener))))
}

/// `architect-desktop serve s3-key <issue|list|revoke> …`.
pub(super) fn command(config: &Config, args: &[String]) -> ExitCode {
    let keys = keys(&config.data_dir);
    let mut out = std::io::stdout();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i.saturating_add(1)))
            .cloned()
    };
    match args.first().map(String::as_str) {
        Some("issue") => {
            let (Some(org), Some(holder)) = (
                flag("--org"),
                flag("--for").as_deref().and_then(KeyHolder::parse),
            ) else {
                let _ = writeln!(
                    std::io::stderr(),
                    "s3-key issue --org <org> --for <user:<id>|device:<endpoint>> [--role viewer|member|admin|owner] [--label <text>]"
                );
                return ExitCode::from(64);
            };
            let role = Role::parse(&flag("--role").unwrap_or_else(|| "member".to_owned()));
            let label = flag("--label").unwrap_or_default();
            match keys.issue(&org, holder, role, &label) {
                Ok(key) => {
                    // The one time the secret is shown.
                    let _ = writeln!(
                        out,
                        "access_key_id = {}\nsecret_access_key = {}\nbucket = {}\nrole = {}",
                        key.access_key,
                        key.secret,
                        key.org,
                        key.role.label()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    let _ = writeln!(std::io::stderr(), "s3-key issue: {e}");
                    ExitCode::from(73)
                }
            }
        }
        Some("list") => {
            for k in keys.list() {
                let _ = writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}",
                    k.access_key,
                    k.org,
                    k.holder.label(),
                    k.role.label(),
                    k.label
                );
            }
            ExitCode::SUCCESS
        }
        Some("revoke") => match args.get(1).map(|ak| keys.revoke(ak)) {
            Some(Ok(true)) => ExitCode::SUCCESS,
            Some(Ok(false)) => {
                let _ = writeln!(std::io::stderr(), "s3-key revoke: no such key");
                ExitCode::from(66)
            }
            Some(Err(e)) => {
                let _ = writeln!(std::io::stderr(), "s3-key revoke: {e}");
                ExitCode::from(73)
            }
            None => ExitCode::from(64),
        },
        _ => {
            let _ = writeln!(
                std::io::stderr(),
                "architect-desktop serve [--config <file>] s3-key <issue|list|revoke>"
            );
            ExitCode::from(64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_key_holds_what_its_grant_holds_now() {
        let admission = Admission::new();
        let key = S3Key {
            access_key: "AK".into(),
            secret: String::new(),
            org: "fts".into(),
            holder: KeyHolder::Device("ep".into()),
            role: Role::Owner,
            label: String::new(),
            created_us: 0,
        };
        assert_eq!(role_now(&admission, &key), None);
        let user = S3Key {
            holder: KeyHolder::User("u".into()),
            role: Role::Viewer,
            ..key
        };
        assert_eq!(role_now(&admission, &user), Some(Role::Viewer));
    }
}
