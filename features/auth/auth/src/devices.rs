//! The engine behind [`auth_proto::DeviceService`]: an account's devices
//! and the org they share. See `auth_proto::device` for why the issuer
//! keeps this address book.
//!
//! Every call resolves its token through
//! [`ArchitectAuth::organization_caller`], so a native app holding the
//! OAuth access token from `/oauth2/token` is served the same as a page
//! holding a session token — and every call is scoped to that account.

use auth_proto::{
    AccountOrg, AuthAccountOrg, AuthDevice, AuthFlowError, DeviceDirectory, DeviceRegistration,
    PeerDevice,
};
use chrono::Utc;
use uuid::Uuid;

use crate::{ArchitectAuth, AuthStorage};

const NAME_MAX: usize = 80;
const PLATFORM_MAX: usize = 24;
const ENDPOINT_MAX: usize = 128;
const SLUG_MAX: usize = 64;
const DISPLAY_MAX: usize = 120;

fn invalid(what: &str) -> AuthFlowError {
    AuthFlowError::InvalidInput(what.to_owned())
}

/// A device name a person typed or the OS gave: trimmed, one line,
/// bounded.
fn clean_name(raw: &str) -> Result<String, AuthFlowError> {
    let name: String = raw.trim().chars().filter(|c| !c.is_control()).collect();
    if name.is_empty() || name.chars().count() > NAME_MAX {
        return Err(invalid("a device needs a name of at most 80 characters"));
    }
    Ok(name)
}

fn clean_platform(raw: &str) -> Result<String, AuthFlowError> {
    let p = raw.trim().to_ascii_lowercase();
    let ok = !p.is_empty()
        && p.len() <= PLATFORM_MAX
        && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if ok {
        Ok(p)
    } else {
        Err(invalid("platform is a short word: ios, macos, linux…"))
    }
}

/// An iroh endpoint id: the public key, as text. Only its shape is
/// checked — the peers verify the key when they dial.
fn clean_endpoint(raw: Option<String>) -> Result<Option<String>, AuthFlowError> {
    let Some(e) = raw.map(|e| e.trim().to_owned()).filter(|e| !e.is_empty()) else {
        return Ok(None);
    };
    if e.len() <= ENDPOINT_MAX && e.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(Some(e))
    } else {
        Err(invalid(
            "endpoint_id is an endpoint's id, letters and digits",
        ))
    }
}

/// Longest relay URL a device may register.
const RELAY_URL_MAX: usize = 512;

/// An iroh relay's URL: `https://` (or `http://` for a local relay), no
/// whitespace. Only its shape is checked; the dialler connects to it.
fn clean_relay(raw: Option<String>) -> Result<Option<String>, AuthFlowError> {
    let Some(url) = raw.map(|u| u.trim().to_owned()).filter(|u| !u.is_empty()) else {
        return Ok(None);
    };
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    // A host to dial: the part before any path, query or fragment.
    let host = rest.map_or("", |r| r.split(['/', '?', '#']).next().unwrap_or(""));
    let host_ok = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'));
    if host_ok && url.len() <= RELAY_URL_MAX && !url.bytes().any(|b| b.is_ascii_whitespace()) {
        Ok(Some(url))
    } else {
        Err(invalid("relay_url is an http(s) URL"))
    }
}

fn clean_org(org: &AccountOrg) -> Result<AccountOrg, AuthFlowError> {
    let slug = org.slug.trim().to_ascii_lowercase();
    let slug_ok = !slug.is_empty()
        && slug.len() <= SLUG_MAX
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !slug_ok {
        return Err(invalid(
            "an org slug is lowercase letters, digits and dashes",
        ));
    }
    let display_name = org.display_name.trim().to_owned();
    if display_name.is_empty() || display_name.chars().count() > DISPLAY_MAX {
        return Err(invalid("an org needs a name of at most 120 characters"));
    }
    if org.id.is_nil() {
        return Err(invalid("an org needs its id"));
    }
    Ok(AccountOrg {
        id: org.id,
        slug,
        display_name,
    })
}

impl<S> ArchitectAuth<S>
where
    S: AuthStorage,
{
    /// Record this install as one of the caller's devices (or refresh
    /// it), take its offered org as the account's if the account has
    /// none, and answer with the directory.
    ///
    /// A device id already registered to another account is refused: an
    /// install signed in as somebody else derives a different id, so a
    /// clash is a mistake or a probe, never a handover.
    pub async fn register_device(
        &self,
        token: &str,
        registration: DeviceRegistration,
    ) -> Result<DeviceDirectory, AuthFlowError> {
        let user = self.organization_caller(token).await?;
        if registration.device_id.is_nil() {
            return Err(invalid("a device needs its id"));
        }
        let name = clean_name(&registration.name)?;
        let platform = clean_platform(&registration.platform)?;
        let endpoint_id = clean_endpoint(registration.endpoint_id)?;
        let relay_url = clean_relay(registration.relay_url)?;
        let offer = registration.offer_org.as_ref().map(clean_org).transpose()?;

        if let Some(held) = self.storage.find_device(registration.device_id).await? {
            if held.user_id != user.id {
                return Err(AuthFlowError::PermissionDenied);
            }
        }
        if let Some(org) = offer {
            if self.storage.find_account_org(user.id).await?.is_none() {
                self.storage
                    .claim_account_org(AuthAccountOrg {
                        id: Uuid::new_v4(),
                        user_id: user.id,
                        org_id: org.id,
                        slug: org.slug,
                        display_name: org.display_name,
                        first_device_id: registration.device_id,
                        created_at: Utc::now(),
                    })
                    .await?;
            }
        }
        let now = Utc::now();
        self.storage
            .upsert_device(AuthDevice {
                id: registration.device_id,
                user_id: user.id,
                name,
                platform,
                endpoint_id,
                relay_url,
                created_at: now,
                last_seen_at: now,
            })
            .await?;
        self.directory_for(user.id).await
    }

    /// The caller's shared org and devices.
    pub async fn list_devices(&self, token: &str) -> Result<DeviceDirectory, AuthFlowError> {
        let user = self.organization_caller(token).await?;
        self.directory_for(user.id).await
    }

    /// Take one of the caller's devices off the account.
    pub async fn forget_device(&self, token: &str, device_id: Uuid) -> Result<(), AuthFlowError> {
        let user = self.organization_caller(token).await?;
        if self.storage.delete_device(device_id, user.id).await? {
            Ok(())
        } else {
            // Somebody else's device and no device at all read the same.
            Err(AuthFlowError::PermissionDenied)
        }
    }

    async fn directory_for(&self, user_id: Uuid) -> Result<DeviceDirectory, AuthFlowError> {
        Ok(DeviceDirectory {
            account_org: self
                .storage
                .find_account_org(user_id)
                .await?
                .map(AccountOrg::from),
            devices: self
                .storage
                .list_devices_for_user(user_id)
                .await?
                .into_iter()
                .map(PeerDevice::from)
                .collect(),
        })
    }
}
