//! An org's enrolled devices, from the app: `DeviceEnrolment` on the
//! node that hosts the org (`architect-desktop serve` with `host_org`),
//! at `/org/{slug}/vox`, signed in as the active account.
//!
//! The org is the account's current one; the host is the one given last
//! for that org ([`crate::Account::org_host`]), else the node
//! `ARCHITECT_ORG_HOST` names ([`ORG_HOST_ENV`]: `devenv up`'s local node,
//! #541). The issuer keeps no enrolments: they live on the hosting node, in
//! its device table.

use architect::connect::{Endpoint, Pool};
use architect_devices::{DeviceEnrolmentClient, Enrolment, Peer};
use architect_orgs::{Org, Role};

use crate::controller::Accounts;

/// The variable naming the node every org of the account is hosted on
/// until one is given for it (`devenv up` points the app at its local
/// node with it).
pub const ORG_HOST_ENV: &str = "ARCHITECT_ORG_HOST";

/// The node [`ORG_HOST_ENV`] names, if any.
#[must_use]
pub fn org_host_from_env() -> Option<String> {
    std::env::var(ORG_HOST_ENV)
        .ok()
        .and_then(|v| clean_host(&v))
}

/// The host kept for an org, else the default one.
fn host_or_default(kept: Option<&str>, default: Option<String>) -> Option<String> {
    kept.and_then(clean_host).or(default)
}

fn clean_host(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('/');
    (!host.is_empty()).then(|| host.to_owned())
}

/// The `/org/{slug}/vox` lane of the node at `host` (`https://x` →
/// `wss://x/org/{slug}/vox`).
pub fn lane(host: &str, slug: &str) -> String {
    let host = host.trim().trim_end_matches('/');
    let ws = host
        .strip_prefix("https://")
        .map(|rest| format!("wss://{rest}"))
        .or_else(|| {
            host.strip_prefix("http://")
                .map(|rest| format!("ws://{rest}"))
        })
        .unwrap_or_else(|| host.to_owned());
    format!("{ws}/org/{slug}/vox")
}

impl Accounts {
    /// The account's current org and the node hosting it: `host` when
    /// given (and kept for the org), else the one kept.
    pub(crate) fn org_and_host(&self, host: Option<&str>) -> Result<(Org, String), String> {
        let account = self
            .active()
            .ok_or_else(|| "no account is signed in".to_owned())?;
        let org = self
            .orgs()
            .current()
            .cloned()
            .filter(|o| o.id.0 != crate::DEVICE_ORG)
            .ok_or_else(|| "switch to one of the account's orgs first (ORG_SWITCH)".to_owned())?;
        let host = match host.map(str::trim).filter(|h| !h.is_empty()) {
            Some(host) => {
                self.store()
                    .set_org_host(&org.id.0, host)
                    .map_err(|e| e.to_string())?;
                host.trim_end_matches('/').to_owned()
            }
            None => host_or_default(account.org_host(&org.id.0), org_host_from_env()).ok_or_else(
                || format!("no node hosts {}'s devices yet: give its address", org.name),
            )?,
        };
        Ok((org, host))
    }

    /// The node that hosts the current org's enrolments, if one was given.
    #[must_use]
    pub fn org_host(&self) -> Option<String> {
        let org = self.orgs().current()?.id.0.clone();
        host_or_default(self.active()?.org_host(&org), org_host_from_env())
    }

    async fn enrolment(&self, host: Option<&str>) -> Result<(Org, DeviceEnrolmentClient), String> {
        let (org, host) = self.org_and_host(host)?;
        let token = self
            .store()
            .active_session()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no account is signed in".to_owned())?
            .token;
        let endpoint = Endpoint::url(lane(&host, &org.slug)).with_bearer(Some(token));
        let client = Pool::new()
            .client(&endpoint)
            .await
            .map_err(|e| format!("{host}: {e}"))?;
        Ok((org, client))
    }

    /// The current org's enrolled devices: every one for an owner or
    /// admin, the person's own otherwise. Kept for the devices page.
    ///
    /// # Errors
    ///
    /// Signed out, in no org, no host; the host's refusal.
    pub async fn org_devices(&self, host: Option<&str>) -> Result<Vec<Peer>, String> {
        let answer = async {
            let (_, client) = self.enrolment(host).await?;
            client.enrolments().await.map_err(|e| e.to_string())
        }
        .await;
        self.state().org_devices = Some(answer.clone());
        self.changed();
        answer
    }

    /// The current org's devices as last fetched.
    #[must_use]
    pub fn org_devices_seen(&self) -> Option<Result<Vec<Peer>, String>> {
        self.state().org_devices.clone()
    }

    /// Enrol a device in the current org with `role` (at most the
    /// person's own there).
    ///
    /// # Errors
    ///
    /// Signed out, in no org, no host; the host's refusal (a role above
    /// the person's, a device somebody else enrolled).
    pub async fn org_enrol(
        &self,
        host: Option<&str>,
        endpoint: &str,
        name: &str,
        platform: &str,
        role: Role,
    ) -> Result<Peer, String> {
        let (_, client) = self.enrolment(host).await?;
        let peer = client
            .enrol(Enrolment {
                endpoint: endpoint.to_owned(),
                name: name.to_owned(),
                platform: platform.to_owned(),
                role,
            })
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.org_devices(None).await;
        Ok(peer)
    }

    /// Withdraw device `endpoint`'s enrolment in the current org: its
    /// next call there is refused.
    ///
    /// # Errors
    ///
    /// Signed out, in no org, no host; the host's refusal (not the
    /// person's to revoke, not enrolled).
    pub async fn org_revoke(&self, host: Option<&str>, endpoint: &str) -> Result<(), String> {
        let (_, client) = self.enrolment(host).await?;
        client
            .revoke(endpoint.to_owned())
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.org_devices(None).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_lane_is_on_the_hosts_origin() {
        assert_eq!(
            super::lane("https://node.example/", "acme"),
            "wss://node.example/org/acme/vox"
        );
        assert_eq!(
            super::lane("http://127.0.0.1:9", "acme"),
            "ws://127.0.0.1:9/org/acme/vox"
        );
    }

    #[test]
    fn a_host_kept_for_the_org_beats_the_default() {
        use super::host_or_default;
        let default = || Some("http://127.0.0.1:7412".to_owned());
        assert_eq!(
            host_or_default(Some("https://node.example/"), default()).as_deref(),
            Some("https://node.example")
        );
        assert_eq!(
            host_or_default(None, default()).as_deref(),
            Some("http://127.0.0.1:7412")
        );
        assert_eq!(host_or_default(Some("  "), None), None);
    }
}
