//! An account's devices, and the one org they share.
//!
//! A person signs in on a phone and a laptop with the same account and
//! expects their work on both. Each device runs its own node and the
//! nodes sync with each other directly (over iroh), but two nodes have to
//! know each other to do that: which org they both hold, and each other's
//! endpoint id for it. The issuer already knows the one thing that makes
//! the introduction safe — that both are signed in as the same person —
//! so it keeps the address book.
//!
//! - [`AuthDevice`]: one row per install, owned by the account that
//!   registered it, carrying the device's endpoint id for the account's
//!   org.
//! - [`AuthAccountOrg`]: the org an account's devices share. The first
//!   device to register offers its own; later devices are told to join
//!   it. It is never replaced, so a second device signing in cannot take
//!   the first one's work away.
//!
//! The issuer stores names and endpoint ids, never content: what the
//! devices sync never passes through here.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// One install, signed in as `user_id`.
#[architect::entity(table_name = "auth_devices", repo)]
#[derive(Eq)]
pub struct AuthDevice {
    /// The install's own id, chosen once by the device and kept with its
    /// data — stable across sign-ins, so re-registering updates the row.
    #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[architect(filterable, sortable)]
    pub user_id: Uuid,
    /// What a person calls it ("Cody's iPhone").
    pub name: String,
    /// `ios`, `macos`, `watchos`, `linux`, `windows`, …
    pub platform: String,
    /// The device's iroh endpoint id for the account's org, once it holds
    /// a node of it.
    pub endpoint_id: Option<String>,
    /// The iroh relay the device is reachable through (`https://…`), so a
    /// peer dials it without a discovery round.
    pub relay_url: Option<String>,
    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
    #[architect(exclude(create, update), on_create = Utc::now(), on_update = Utc::now())]
    pub last_seen_at: DateTime<Utc>,
}

#[cfg(feature = "server")]
pub use __auth_device_storage::{ActiveModel, Column, Entity, Model, Relation};

/// The org an account's devices share, set by the first to offer one.
pub mod account_org {
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    #[architect::entity(table_name = "auth_account_orgs", repo)]
    #[derive(Eq)]
    pub struct AuthAccountOrg {
        #[architect(primary_key, auto_increment = false, on_create = Uuid::new_v4())]
        pub id: Uuid,
        /// One per account (a unique index).
        #[architect(filterable)]
        pub user_id: Uuid,
        /// The org's own id, as its nodes know it.
        pub org_id: Uuid,
        pub slug: String,
        pub display_name: String,
        /// The device that offered it.
        pub first_device_id: Uuid,
        #[architect(exclude(create, update), on_create = Utc::now())]
        pub created_at: DateTime<Utc>,
    }

    #[cfg(feature = "server")]
    pub use __auth_account_org_storage::{ActiveModel, Column, Entity, Model, Relation};
}

pub use account_org::AuthAccountOrg;

/// An org, as a device names it.
#[architect::wire]
#[derive(Eq)]
pub struct AccountOrg {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
}

/// What a device says about itself when it registers.
#[architect::wire]
#[derive(Eq)]
pub struct DeviceRegistration {
    /// The install's own id ([`AuthDevice::id`]).
    pub device_id: Uuid,
    pub name: String,
    pub platform: String,
    /// Its endpoint id for the account's org, if it holds a node of it.
    pub endpoint_id: Option<String>,
    /// The iroh relay it is reachable through ([`AuthDevice::relay_url`]).
    pub relay_url: Option<String>,
    /// The org this device would share, if the account has none yet —
    /// its own personal org. Ignored once the account has one.
    pub offer_org: Option<AccountOrg>,
}

/// One of the account's devices, as another of them sees it.
#[architect::wire]
#[derive(Eq)]
pub struct PeerDevice {
    pub device_id: Uuid,
    pub name: String,
    pub platform: String,
    pub endpoint_id: Option<String>,
    pub relay_url: Option<String>,
    pub last_seen_at: DateTime<Utc>,
}

/// Everything a device needs to find the others: the shared org and
/// every device signed in as this account (the asking one included).
#[architect::wire]
#[derive(Eq)]
pub struct DeviceDirectory {
    pub account_org: Option<AccountOrg>,
    pub devices: Vec<PeerDevice>,
}

impl From<AuthDevice> for PeerDevice {
    fn from(d: AuthDevice) -> Self {
        Self {
            device_id: d.id,
            name: d.name,
            platform: d.platform,
            endpoint_id: d.endpoint_id,
            relay_url: d.relay_url,
            last_seen_at: d.last_seen_at,
        }
    }
}

impl From<AuthAccountOrg> for AccountOrg {
    fn from(o: AuthAccountOrg) -> Self {
        Self {
            id: o.org_id,
            slug: o.slug,
            display_name: o.display_name,
        }
    }
}

/// The account's device address book. Every call is scoped to the
/// account the token names; a device can see and remove only its own
/// account's devices.
#[architect::service(path = "auth")]
pub trait DeviceService {
    /// Say this install is signed in here, with its endpoint id; offer an
    /// org to share if the account has none. Returns the directory.
    #[http(path = "devices/register")]
    async fn register_device(
        &self,
        token: String,
        registration: DeviceRegistration,
    ) -> Result<DeviceDirectory, crate::AuthFlowError>;

    /// The shared org and every device of this account.
    #[http(path = "devices/list")]
    async fn list_devices(&self, token: String) -> Result<DeviceDirectory, crate::AuthFlowError>;

    /// Take a device off the account — a lost phone, an old laptop. Its
    /// peers stop being told about it.
    #[http(path = "devices/forget")]
    async fn forget_device(
        &self,
        token: String,
        device_id: Uuid,
    ) -> Result<(), crate::AuthFlowError>;
}
