//! Wire contract for architect-auth.

pub use architect;

// r[impl auth.core.entities.single-source]
pub mod account;
pub mod agent_link;
pub mod api_key;
pub mod audit_event;
pub mod device;
pub mod email_change;
pub mod invitation;
pub mod invite_link;
pub mod member;
pub mod organization;
pub mod organization_role;
pub mod organizations;
pub mod passkey;
pub mod passkey_ceremony;
pub mod service;
pub mod session;
pub mod team;
pub mod team_member;
pub mod two_factor;
pub mod user;
pub mod verification;

// r[verify auth.core.entities.single-source]
pub use account::{
    AuthAccount, AuthAccountCreate, AuthAccountList, AuthAccountUpdate,
};
pub use agent_link::{
    AuthAgentLink, AuthAgentLinkCreate, AuthAgentLinkList, AuthAgentLinkUpdate,
};
pub use api_key::{AuthApiKey, AuthApiKeyCreate, AuthApiKeyList, AuthApiKeyUpdate};
pub use audit_event::{
    AuthAuditEventRecord, AuthAuditEventRecordCreate, AuthAuditEventRecordList,
    AuthAuditEventRecordUpdate,
};
pub use invitation::{
    AuthInvitation, AuthInvitationCreate, AuthInvitationList, AuthInvitationUpdate, InvitationStatus,
};
pub use invite_link::{
    AuthInviteLink, AuthInviteLinkCreate, AuthInviteLinkList, AuthInviteLinkUpdate,
};
pub use member::{AuthMember, AuthMemberCreate, AuthMemberList, AuthMemberUpdate};
pub use organization::{
    AuthOrganization, AuthOrganizationCreate, AuthOrganizationList, AuthOrganizationUpdate,
};
pub use organization_role::{
    AuthOrganizationRole, AuthOrganizationRoleCreate, AuthOrganizationRoleList,
    AuthOrganizationRoleUpdate,
};
pub use passkey::{
    AuthPasskey, AuthPasskeyCreate, AuthPasskeyList, AuthPasskeyUpdate,
};
pub use passkey_ceremony::{
    AuthPasskeyCeremony, AuthPasskeyCeremonyCreate, AuthPasskeyCeremonyList,
    AuthPasskeyCeremonyUpdate, PasskeyCeremonyKind,
};
pub use session::{
    AuthSession, AuthSessionCreate, AuthSessionList, AuthSessionUpdate,
};
pub use team::{AuthTeam, AuthTeamCreate, AuthTeamList, AuthTeamUpdate};
pub use team_member::{
    AuthTeamMember, AuthTeamMemberCreate, AuthTeamMemberList, AuthTeamMemberUpdate,
};
pub use two_factor::{
    AuthTwoFactor, AuthTwoFactorCreate, AuthTwoFactorList, AuthTwoFactorUpdate,
};
pub use user::{AuthUser, AuthUserCreate, AuthUserList, AuthUserUpdate};
pub use verification::{
    AuthVerification, AuthVerificationCreate, AuthVerificationList, AuthVerificationUpdate,
};

// r[impl auth.core.errors-stable]
// r[verify auth.core.errors-stable]
#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[repr(u8)]
#[derive(Eq, thiserror::Error)]
pub enum AuthFlowError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("session expired")]
    SessionExpired,
    #[error("verification required")]
    VerificationRequired,
    #[error("two-factor required")]
    TwoFactorRequired,
    #[error("permission denied")]
    PermissionDenied,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Eq)]
pub struct SignInEmailPassword {
    pub email: String,
    pub password: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

/// A device with no browser of its own being signed in (RFC 8628 —
/// a CLI, a TV): what it shows its person, and what it polls with.
#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Eq)]
pub struct DeviceSignIn {
    /// What the device polls with. A secret: never shown.
    pub device_code: String,
    /// What its person confirms from a signed-in browser — a phone.
    pub user_code: String,
    /// Where they confirm it, relative to the auth server
    /// (`/auth/device`).
    pub verification_uri: String,
    /// The same with the code filled in: the link to open.
    pub verification_uri_complete: String,
    pub expires_in_seconds: i64,
    /// How long to wait between polls.
    pub interval_seconds: i64,
}

/// Wire shape of `ArchitectAuth::create_email_password_user` — the
/// sign-up command, minus nothing: same fields, RPC-serializable.
#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Eq)]
pub struct SignUpEmailPassword {
    pub email: String,
    pub password: String,
    pub name: Option<String>,
    pub username: Option<String>,
    pub image: Option<String>,
    pub metadata_json: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Eq)]
pub struct AuthSessionBundle {
    pub user: AuthUser,
    pub session: AuthSession,
    pub token: String,
}

// The session RPC surface. The prelude glob carries the trait plus the
// vox-gated client/dispatcher/descriptor and the `AuthServiceService` /
// `auth_service_layer` / `auth_service_serve` mount verbs; the explicit
// re-exports preserve the names downstream code mounted with before the
// trait moved into `service` (`AuthServiceDispatcher::new(..)` +
// `auth_service_service_descriptor()`).
pub use service::AUTHORIZATION_METADATA_KEY;
pub use service::OrgMember;
pub use service::prelude::*;
#[cfg(feature = "vox")]
pub use service::{AuthServiceDispatcher, auth_service_service_descriptor};

// The device address book — a third trait, same shape.
#[cfg(feature = "vox")]
pub use device::ensure_registered;
pub use device::prelude::*;
pub use device::{
    AccountOrg, AuthAccountOrg, AuthDevice, AuthDeviceCreate, DeviceDirectory,
    DeviceRegistration, PeerDevice, Registered,
};

// The organization surface — same shape, second trait.
pub use organizations::prelude::*;
pub use organizations::{
    Invite, IssuedInvitation, LinkedAgent, NewOrganization, OrganizationBundle, OrganizationMember,
};

/// Which page of a list: its index (from 0) and how many rows it holds.
#[derive(Debug, Clone, PartialEq, Eq, Default, facet::Facet)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
pub struct Page {
    /// Zero-indexed page number.
    pub index: u32,
    /// Rows per page.
    pub size: u32,
}

/// A call that never reached the engine is an internal failure from the
/// caller's point of view — which is what lets `AuthServiceClient` and
/// `AuthServiceHttpClient` implement `AuthService` itself.
impl From<architect::TransportError> for AuthFlowError {
    fn from(e: architect::TransportError) -> Self {
        Self::Internal(e.to_string())
    }
}

// r[impl auth.transport.error-mapping]
// r[impl auth.errors.taxonomy]
/// How an `AuthFlowError` appears on the HTTP face — the status and the
/// stable code the engine's taxonomy (`auth::transport::AUTH_ERROR_TAXONOMY`)
/// has always promised. Declared here because the type is declared here;
/// the taxonomy test in `auth` pins the two against each other.
impl architect::http::HttpError for AuthFlowError {
    fn status(&self) -> u16 {
        match self {
            Self::InvalidCredentials | Self::SessionExpired => 401,
            Self::VerificationRequired | Self::TwoFactorRequired | Self::PermissionDenied => 403,
            Self::InvalidInput(_) => 400,
            Self::Internal(_) => 500,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::InvalidCredentials => "invalid_credentials",
            Self::SessionExpired => "session_expired",
            Self::VerificationRequired => "verification_required",
            Self::TwoFactorRequired => "two_factor_required",
            Self::PermissionDenied => "permission_denied",
            Self::InvalidInput(_) => "invalid_input",
            Self::Internal(_) => "internal",
        }
    }
}
