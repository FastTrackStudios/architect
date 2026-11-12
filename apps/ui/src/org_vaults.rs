//! The signed-in account's org vaults on this device (org = vault, #443),
//! and their sync with the node that hosts each org.
//!
//! - **Listed**: every org of the active account (not the device's own)
//!   has a vault folder in the app's folder, `<app dir>/Org vaults/<slug>`,
//!   kept in `vaults.json` `orgs` ([`adopt`]), so the navigator mounts it
//!   as `@<org>` beside the app's own vault (`crate::vaults::catalog`,
//!   `crate::vault`), on a phone and a desk alike. Its path is kept
//!   relative to the app's folder: iOS moves an app's folder on every
//!   install and update.
//! - **Synced**: when a node is known for the org (the one given on the
//!   org's Devices page, `Accounts::host_of`, or `ARCHITECT_ORG_HOST`), the
//!   app reads the org's notes on the node's lane (`/org/{slug}/vox`, as
//!   the account: `vault_proto::notes::service::NotesClient`) and writes them into
//!   the folder, and writes back a note changed here ([`plan`]): on sign-in,
//!   when the account's orgs change, every [`EVERY`], and on "Sync now".
//!   A note changed on both sides keeps this device's text and the node's
//!   beside it (`<name> (from the node).md`). A note deleted on either side
//!   comes back (the lane has no delete). A viewer's changes are refused
//!   by the node and stay here.
//!
//! What each org's sync last did is [`OrgSyncs`], in the root's context;
//! the phone's drawer shows it (`OrgVaultsPanel`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use architect_accounts::Accounts;
use architect_orgs::Org;
use dioxus::prelude::*;

use crate::vaults::OrgVault;

/// The folder in the app's folder that holds the org vaults.
pub const FOLDER: &str = "Org vaults";
/// In each org vault: every note's text as last agreed with the node
/// (its hash, by path), for [`plan`].
pub const AGREED: &str = ".architect-synced.json";
/// How often an org vault is synced while the app runs.
pub const EVERY: Duration = Duration::from_secs(120);

/// An org's vault folder name: its slug, kept to safe characters.
#[must_use]
pub fn folder_of(slug: &str) -> String {
    let safe: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if safe.trim_matches('-').is_empty() {
        "org".to_owned()
    } else {
        safe
    }
}

/// The orgs that get a vault: the account's, not the device's own.
fn synced_orgs(orgs: &[Org]) -> impl Iterator<Item = &Org> {
    orgs.iter()
        .filter(|o| o.id.0 != architect_accounts::DEVICE_ORG && !o.slug.trim().is_empty())
}

/// Makes a vault folder for each of `orgs` and keeps them in
/// `vaults.json` `orgs` (beside entries written by hand, which stay).
/// Answers the account's org vaults.
///
/// # Errors
///
/// The app has no folder, or a folder or `vaults.json` cannot be written.
pub fn adopt(orgs: &[Org]) -> std::io::Result<Vec<OrgVault>> {
    let app = crate::app_dir().ok_or_else(|| std::io::Error::other("the app has no folder"))?;
    adopt_in(&app, orgs)
}

fn adopt_in(app: &Path, orgs: &[Org]) -> std::io::Result<Vec<OrgVault>> {
    let mut out = Vec::new();
    let mut entries = Vec::new();
    for org in synced_orgs(orgs) {
        let rel = Path::new(FOLDER).join(folder_of(&org.slug));
        let dir = app.join(&rel);
        std::fs::create_dir_all(&dir)?;
        entries.push(serde_json::json!({
            "org": org.id.0,
            "slug": org.slug,
            "name": org.name,
            "colour": org.color,
            "dir": rel.display().to_string(),
            "account": true,
        }));
        out.push(OrgVault {
            org: org.id.0.clone(),
            name: org.name.clone(),
            colour: org.color.clone(),
            icon: None,
            dir,
        });
    }
    crate::vaults::keep_account_orgs(app, entries)?;
    Ok(out)
}

/// A note's text, hashed (FNV-1a, 64 bits): stable across runs, which is
/// all [`AGREED`] needs.
#[must_use]
pub fn hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// One step of a sync, by a note's path in the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The node's text, written here.
    Write(String),
    /// This device's text, written to the node.
    Push(String),
    /// Changed on both sides: this device's text stays, the node's is
    /// written beside it.
    Both(String),
}

/// What to do so the folder and the node agree, from each side's text
/// hash by path (`here`, `node`) and the hashes last agreed (`agreed`).
#[must_use]
pub fn plan(
    here: &BTreeMap<String, String>,
    node: &BTreeMap<String, String>,
    agreed: &BTreeMap<String, String>,
) -> Vec<Step> {
    let paths: BTreeSet<&String> = here.keys().chain(node.keys()).collect();
    let mut out = Vec::new();
    for path in paths {
        let (h, n, a) = (here.get(path), node.get(path), agreed.get(path));
        match (h, n) {
            (Some(h), Some(n)) if h == n => {}
            // Only the node has it: new there, or deleted here (the lane
            // deletes nothing, so it comes back).
            (None, Some(_)) => out.push(Step::Write(path.clone())),
            // Only here: new here, or deleted on the node (it goes back).
            (Some(_), None) => out.push(Step::Push(path.clone())),
            (Some(h), Some(n)) => {
                if a == Some(h) {
                    out.push(Step::Write(path.clone()));
                } else if a == Some(n) {
                    out.push(Step::Push(path.clone()));
                } else {
                    out.push(Step::Both(path.clone()));
                }
            }
            (None, None) => {}
        }
    }
    out
}

/// Where an org's note from the node goes in its folder: the vault's
/// own path when the node holds one vault, else under the vault's name.
#[must_use]
pub fn local_path(vaults: usize, vault: &str, path: &str) -> String {
    if vaults > 1 {
        format!("{vault}/{path}")
    } else {
        path.to_owned()
    }
}

/// `<name> (from the node).md` beside `path`.
#[must_use]
pub fn beside(path: &str) -> String {
    path.strip_suffix(".md").map_or_else(
        || format!("{path} (from the node)"),
        |stem| format!("{stem} (from the node).md"),
    )
}

/// What an org's last sync did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    /// No node is known for the org: give its address on its Devices page.
    NoNode,
    /// Under way.
    Syncing,
    /// Done: notes in the folder, written here, sent to the node, kept
    /// both, and when.
    Synced {
        notes: usize,
        pulled: usize,
        pushed: usize,
        both: usize,
        at: String,
    },
    /// It failed: why.
    Failed(String),
}

/// Each org's vault and its last sync, by org id: the root's context.
#[derive(Clone, Copy)]
pub struct OrgSyncs {
    pub vaults: Signal<Vec<OrgVault>>,
    pub states: Signal<BTreeMap<String, SyncState>>,
    run: Callback<()>,
}

impl OrgSyncs {
    /// Syncs every org vault now.
    pub fn sync_now(&self) {
        self.run.call(());
    }
}

/// The account's org vaults kept and synced while the app runs; `None`
/// in the sandbox or signed out of every account.
pub fn use_org_vaults(accounts: Option<architect_accounts::ui::AccountsUi>) -> Option<OrgSyncs> {
    let vaults = use_signal(Vec::<OrgVault>::new);
    let states = use_signal(BTreeMap::<String, SyncState>::new);
    let run = use_callback(move |()| {
        let Some(ui) = accounts else { return };
        spawn(sync_all(ui.accounts(), vaults, states));
    });
    let syncs = use_context_provider(|| OrgSyncs {
        vaults,
        states,
        run,
    });
    // Signed in, the orgs fetched or changed: adopt and sync.
    let signed_in = accounts.and_then(|ui| {
        let orgs: Vec<String> = ui.orgs().orgs.into_iter().map(|o| o.id.0).collect();
        ui.accounts().active().map(|a| (a.key(), orgs))
    });
    use_effect(use_reactive!(|signed_in| {
        if signed_in.is_some() {
            run.call(());
        }
    }));
    use_hook(move || {
        spawn(async move {
            loop {
                futures_timer::Delay::new(EVERY).await;
                run.call(());
            }
        });
    });
    accounts.map(|_| syncs)
}

/// The account's orgs adopted, mounted in the navigator, and each synced.
#[expect(
    clippy::future_not_send,
    reason = "spawned on the window's task, where its signals live"
)]
async fn sync_all(
    accounts: Accounts,
    mut vaults: Signal<Vec<OrgVault>>,
    mut states: Signal<BTreeMap<String, SyncState>>,
) {
    if accounts.active().is_none() {
        return;
    }
    let orgs = accounts.orgs().orgs;
    let adopted = match adopt(&orgs) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "app.org_vaults.adopt_failed");
            return;
        }
    };
    if *vaults.peek() != adopted {
        vaults.set(adopted.clone());
        crate::vault::mount_org_vaults(&adopted);
    }
    let Some(bearer) = accounts.lane_bearer() else {
        return;
    };
    for org in synced_orgs(&orgs) {
        let Some(vault) = adopted.iter().find(|v| v.org == org.id.0) else {
            continue;
        };
        let Some(host) = accounts.host_of(&org.id.0) else {
            states.write().insert(org.id.0.clone(), SyncState::NoNode);
            continue;
        };
        if states.peek().get(&org.id.0) == Some(&SyncState::Syncing) {
            continue;
        }
        states.write().insert(org.id.0.clone(), SyncState::Syncing);
        // The lane is tokio's; the window's executor is not.
        let (h, slug, b, dir) = (
            host.clone(),
            org.slug.clone(),
            bearer.clone(),
            vault.dir.clone(),
        );
        let done = match net() {
            Ok(rt) => rt
                .spawn(async move { sync_one(&h, &slug, &b, &dir).await })
                .await
                .unwrap_or_else(|e| Err(format!("the sync stopped: {e}"))),
            Err(e) => Err(format!("no runtime for the sync: {e}")),
        };
        states.write().insert(
            org.id.0.clone(),
            done.unwrap_or_else(|e| {
                tracing::warn!(org = %org.id.0, error = %e, "app.org_vaults.sync_failed");
                SyncState::Failed(e)
            }),
        );
    }
}

/// The runtime the syncs run on (two threads, `org-vaults`): the node's
/// lane is tokio's, and the window's executor is not.
fn net() -> Result<&'static tokio::runtime::Runtime, String> {
    static RT: std::sync::OnceLock<Result<tokio::runtime::Runtime, String>> =
        std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("org-vaults")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// One org's vault synced with its node's lane at `host`.
async fn sync_one(host: &str, slug: &str, bearer: &str, dir: &Path) -> Result<SyncState, String> {
    use vault_proto::notes::service::NotesClient;
    let endpoint = architect::connect::Endpoint::url(architect_accounts::lane(host, slug))
        .with_bearer(Some(bearer.to_owned()));
    let client: NotesClient = architect::connect::Pool::new()
        .client(&endpoint)
        .await
        .map_err(|e| format!("{host}: {e}"))?;
    let listed = client.list().await.map_err(|e| e.to_string())?;
    let vaults: BTreeSet<&str> = listed.iter().map(|n| n.vault.as_str()).collect();
    let count = vaults.len();
    // Each note's text on the node, by its path here.
    let mut node_text = BTreeMap::new();
    let mut origin = BTreeMap::new();
    for n in &listed {
        let text = client
            .read(n.vault.clone(), n.path.clone())
            .await
            .map_err(|e| e.to_string())?;
        let here = local_path(count, &n.vault, &n.path);
        origin.insert(here.clone(), (n.vault.clone(), n.path.clone()));
        node_text.insert(here, text);
    }
    let here_text = read_notes(dir);
    let agreed = read_agreed(dir);
    let hashes = |m: &BTreeMap<String, String>| -> BTreeMap<String, String> {
        m.iter().map(|(p, t)| (p.clone(), hash(t))).collect()
    };
    let steps = plan(&hashes(&here_text), &hashes(&node_text), &agreed);
    let first_vault = vaults.iter().next().map(|v| (*v).to_owned());
    let (mut pulled, mut pushed, mut both) = (0_usize, 0_usize, 0_usize);
    let mut now_agreed = agreed;
    for step in steps {
        match step {
            Step::Write(path) => {
                let Some(text) = node_text.get(&path) else {
                    continue;
                };
                write_note(dir, &path, text)?;
                now_agreed.insert(path, hash(text));
                pulled = pulled.saturating_add(1);
            }
            Step::Push(path) => {
                let Some(text) = here_text.get(&path) else {
                    continue;
                };
                let target = origin.get(&path).cloned().or_else(|| {
                    if count > 1 {
                        let (v, rest) = path.split_once('/')?;
                        Some((v.to_owned(), rest.to_owned()))
                    } else {
                        first_vault.clone().map(|v| (v, path.clone()))
                    }
                });
                let Some((vault, at)) = target else { continue };
                match client.write(vault, at, text.clone()).await {
                    Ok(()) => {
                        now_agreed.insert(path, hash(text));
                        pushed = pushed.saturating_add(1);
                    }
                    Err(e) => tracing::info!(path, error = %e, "app.org_vaults.push_refused"),
                }
            }
            Step::Both(path) => {
                let Some(text) = node_text.get(&path) else {
                    continue;
                };
                write_note(dir, &beside(&path), text)?;
                both = both.saturating_add(1);
            }
        }
    }
    for (path, text) in &node_text {
        if here_text.get(path) == Some(text) {
            now_agreed.insert(path.clone(), hash(text));
        }
    }
    write_agreed(dir, &now_agreed)?;
    Ok(SyncState::Synced {
        notes: node_text.len(),
        pulled,
        pushed,
        both,
        at: time_now(),
    })
}

/// Every markdown note in `dir`, by its path there (`/`-separated); hidden
/// files and folders left out.
fn read_notes(dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<String, String>) {
        let Ok(entries) = std::fs::read_dir(at) else {
            return;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = e.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("md"))
                && let (Ok(rel), Ok(text)) =
                    (path.strip_prefix(root), std::fs::read_to_string(&path))
            {
                let rel: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                out.insert(rel.join("/"), text);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Writes `text` at `path` in `dir`, refusing a path that leaves it.
fn write_note(dir: &Path, path: &str, text: &str) -> Result<(), String> {
    if path
        .split('/')
        .any(|c| c.is_empty() || c == "." || c == "..")
    {
        return Err(format!("the node named a path outside the vault: {path}"));
    }
    let at = dir.join(path);
    if let Some(parent) = at.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&at, text).map_err(|e| e.to_string())
}

fn read_agreed(dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(dir.join(AGREED))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_agreed(dir: &Path, agreed: &BTreeMap<String, String>) -> Result<(), String> {
    let text = serde_json::to_string(agreed).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(AGREED), text).map_err(|e| e.to_string())
}

fn time_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let day = secs % 86_400;
    format!("{:02}:{:02} UTC", day / 3600, (day % 3600) / 60)
}

/// The org vaults on a phone's left side: each org of the account, its
/// colour and name, what its last sync did, and "Sync now".
#[component]
pub fn OrgVaultsPanel() -> Element {
    let Some(syncs) = try_use_context::<OrgSyncs>() else {
        return rsx! {};
    };
    let t = architect_shell::use_shell().theme;
    let vaults = (syncs.vaults)();
    let states = (syncs.states)();
    rsx! {
        div { "data-org-vaults": "true",
            style: "display: flex; flex-direction: column; gap: 2px; padding: 4px 10px 10px; font-family: {t.font};",
            div { style: "display: flex; align-items: center; justify-content: space-between; padding: 6px 4px;",
                span { style: "font-size: 12px; font-weight: 650; letter-spacing: 0.04em; text-transform: uppercase; color: {t.ink_3};",
                    "Org vaults"
                }
                if !vaults.is_empty() {
                    button { class: "lp", "data-org-vaults-sync": "true",
                        style: "font-size: 13px; padding: 4px 10px; border-radius: 8px; border: 1px solid {t.rule}; background: transparent; color: {t.ink_2};",
                        onclick: move |_| syncs.sync_now(),
                        "Sync now"
                    }
                }
            }
            if vaults.is_empty() {
                div { style: "font-size: 13px; color: {t.ink_3}; padding: 4px;",
                    "Sign in to see your orgs' vaults here."
                }
            }
            for v in vaults {
                {
                    let state = states.get(&v.org).cloned();
                    let said = match state {
                        None => "Not synced yet".to_owned(),
                        Some(SyncState::NoNode) => "No node yet: give its address in Devices".to_owned(),
                        Some(SyncState::Syncing) => "Syncing…".to_owned(),
                        Some(SyncState::Synced { notes, pulled, pushed, both, at }) => {
                            let mut s = format!("{notes} notes · synced {at}");
                            if pulled > 0 || pushed > 0 || both > 0 {
                                let _ = write!(s, " · {pulled} in, {pushed} out");
                            }
                            if both > 0 {
                                let _ = write!(s, ", {both} kept both");
                            }
                            s
                        }
                        Some(SyncState::Failed(why)) => format!("Sync failed: {why}"),
                    };
                    rsx! {
                        div { key: "{v.org}", "data-org-vault": "{v.org}",
                            style: "display: flex; align-items: center; gap: 10px; padding: 8px 4px; border-radius: 8px;",
                            span { style: "width: 10px; height: 10px; border-radius: 3px; flex-shrink: 0; background: {v.colour};" }
                            div { style: "flex: 1 1 0; min-width: 0; display: flex; flex-direction: column;",
                                span { style: "font-size: 15px; color: {t.ink};", "{v.name}" }
                                span { style: "font-size: 12px; color: {t.ink_3}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{said}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn a_sync_brings_in_the_nodes_notes_and_sends_this_devices() {
        // First sync: nothing agreed. The node's note comes in; a note made
        // here goes out; one equal on both sides is left.
        let here = map(&[("Mine.md", "a"), ("Same.md", "s")]);
        let node = map(&[("Theirs.md", "b"), ("Same.md", "s")]);
        assert_eq!(
            plan(&here, &node, &BTreeMap::new()),
            [
                Step::Push("Mine.md".into()),
                Step::Write("Theirs.md".into())
            ]
        );
    }

    #[test]
    fn the_side_that_changed_wins_and_both_changed_keeps_both() {
        let agreed = map(&[("A.md", "1"), ("B.md", "1"), ("C.md", "1")]);
        // A changed on the node, B here, C on both.
        let here = map(&[("A.md", "1"), ("B.md", "2"), ("C.md", "3")]);
        let node = map(&[("A.md", "2"), ("B.md", "1"), ("C.md", "4")]);
        assert_eq!(
            plan(&here, &node, &agreed),
            [
                Step::Write("A.md".into()),
                Step::Push("B.md".into()),
                Step::Both("C.md".into())
            ]
        );
        assert_eq!(beside("Notes/C.md"), "Notes/C (from the node).md");
    }

    #[test]
    fn a_node_with_several_vaults_keeps_each_in_a_folder() {
        assert_eq!(local_path(1, "acme", "Songs/A.md"), "Songs/A.md");
        assert_eq!(local_path(2, "acme", "Songs/A.md"), "acme/Songs/A.md");
        assert_eq!(hash("x"), hash("x"));
        assert_ne!(hash("x"), hash("y"));
    }

    #[test]
    fn the_accounts_orgs_get_a_vault_each_kept_relative_to_the_app() {
        let app = tempfile::tempdir().unwrap();
        let orgs = vec![
            architect_accounts::device_org(),
            Org::new("o1", "Acme", architect_orgs::Role::Owner).color("#1d4ed8"),
        ];
        let mut orgs = orgs;
        orgs[1].slug = "acme".to_owned();
        let adopted = adopt_in(app.path(), &orgs).unwrap();
        assert_eq!(adopted.len(), 1, "not the device's own org");
        assert_eq!(adopted[0].dir, app.path().join("Org vaults/acme"));
        assert!(adopted[0].dir.is_dir());
        let kept: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(app.path().join("vaults.json")).unwrap())
                .unwrap();
        assert_eq!(
            kept["orgs"][0]["dir"], "Org vaults/acme",
            "relative: iOS moves the app's folder"
        );
    }

    #[test]
    fn a_path_from_the_node_never_leaves_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_note(dir.path(), "../out.md", "x").is_err());
        assert!(write_note(dir.path(), "a//b.md", "x").is_err());
        write_note(dir.path(), "Songs/A.md", "x").unwrap();
        assert_eq!(read_notes(dir.path()), map(&[("Songs/A.md", "x")]));
    }
}
