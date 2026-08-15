//! Test helpers for downstream architect-auth integration tests.

use chrono::{Duration, Utc};

use crate::{
    ApiKeyBundle, ArchitectAuth, AuthCookieConfig, AuthFlowError, AuthSessionBundle, AuthStorage,
    CreateApiKey, CreateEmailPasswordUser, CreateInvitation, CreateOrganization,
    CreateOrganizationRole, CreateTeam, EmailOtpVerification, InvitationToken, OrganizationBundle,
    SendEmailOtp, SignInEmailPassword, VerifyEmailOtp,
};
use auth_proto::{AuthOrganizationRole, AuthTeam};

pub const TEST_SECRET: &str = "architect-auth-test-secret-32-bytes!!";
pub const TEST_PASSWORD: &str = "correct horse battery staple";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestUserInput {
    pub email: String,
    pub password: String,
    pub name: Option<String>,
}

impl TestUserInput {
    pub fn new(email: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            password: TEST_PASSWORD.into(),
            name: Some("Test User".into()),
        }
    }
}

#[derive(Clone)]
pub struct AuthTestHarness<S> {
    pub auth: ArchitectAuth<S>,
}

impl<S> AuthTestHarness<S>
where
    S: AuthStorage,
{
    pub fn new(auth: ArchitectAuth<S>) -> Self {
        Self { auth }
    }

    // r[impl auth.test-utils.users]
    // r[impl auth.test-utils.sessions]
    pub async fn create_user_session(
        &self,
        input: TestUserInput,
    ) -> Result<AuthSessionBundle, AuthFlowError> {
        self.auth
            .create_email_password_user(CreateEmailPasswordUser {
                email: input.email,
                password: input.password,
                name: input.name,
                username: None,
                image: None,
                metadata_json: None,
                ip_address: Some("127.0.0.1".into()),
                user_agent: Some("architect-auth-test-utils".into()),
            })
            .await
    }

    // r[impl auth.test-utils.sessions]
    pub async fn sign_in(
        &self,
        email: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<AuthSessionBundle, AuthFlowError> {
        self.auth
            .sign_in_email_password(SignInEmailPassword {
                email: email.into(),
                password: password.into(),
                ip_address: Some("127.0.0.1".into()),
                user_agent: Some("architect-auth-test-utils".into()),
            })
            .await
    }

    // r[impl auth.test-utils.organizations]
    pub async fn create_organization(
        &self,
        session_token: impl Into<String>,
        slug: impl Into<String>,
    ) -> Result<OrganizationBundle, AuthFlowError> {
        let slug = slug.into();
        self.auth
            .create_organization(CreateOrganization {
                session_token: session_token.into(),
                name: slug.to_ascii_uppercase(),
                slug,
                logo: None,
                metadata_json: None,
            })
            .await
    }

    // r[impl auth.test-utils.organizations]
    pub async fn create_role(
        &self,
        session_token: impl Into<String>,
        organization_id: uuid::Uuid,
        role: impl Into<String>,
        permissions_json: impl Into<String>,
    ) -> Result<AuthOrganizationRole, AuthFlowError> {
        self.auth
            .create_organization_role(CreateOrganizationRole {
                session_token: session_token.into(),
                organization_id,
                role: role.into(),
                permissions_json: permissions_json.into(),
            })
            .await
    }

    // r[impl auth.test-utils.teams]
    pub async fn create_team(
        &self,
        session_token: impl Into<String>,
        organization_id: uuid::Uuid,
        name: impl Into<String>,
    ) -> Result<AuthTeam, AuthFlowError> {
        self.auth
            .create_team(CreateTeam {
                session_token: session_token.into(),
                organization_id,
                name: name.into(),
            })
            .await
    }

    // r[impl auth.test-utils.api-keys]
    pub async fn create_api_key(
        &self,
        session_token: impl Into<String>,
        permissions_json: Option<String>,
    ) -> Result<ApiKeyBundle, AuthFlowError> {
        self.auth
            .create_api_key(CreateApiKey {
                session_token: session_token.into(),
                name: Some("test key".into()),
                expires_at: None,
                permissions_json,
                rate_limit_time_window: None,
                rate_limit_max: None,
                metadata_json: None,
            })
            .await
    }

    // r[impl auth.test-utils.otp]
    pub async fn send_email_otp(
        &self,
        email: impl Into<String>,
    ) -> Result<crate::VerificationToken, AuthFlowError> {
        self.auth
            .send_email_otp(SendEmailOtp {
                email: email.into(),
            })
            .await
    }

    // r[impl auth.test-utils.otp]
    pub async fn verify_email_otp(
        &self,
        email: impl Into<String>,
        otp: impl Into<String>,
        create_session: bool,
    ) -> Result<EmailOtpVerification, AuthFlowError> {
        self.auth
            .verify_email_otp(VerifyEmailOtp {
                email: email.into(),
                otp: otp.into(),
                create_session,
                ip_address: Some("127.0.0.1".into()),
                user_agent: Some("architect-auth-test-utils".into()),
            })
            .await
    }

    // r[impl auth.test-utils.organizations]
    pub async fn create_invitation(
        &self,
        session_token: impl Into<String>,
        organization_id: uuid::Uuid,
        email: impl Into<String>,
        role: impl Into<String>,
    ) -> Result<InvitationToken, AuthFlowError> {
        self.auth
            .create_invitation(CreateInvitation {
                session_token: session_token.into(),
                organization_id,
                email: email.into(),
                role: role.into(),
                expires_at: Utc::now() + Duration::hours(1),
            })
            .await
    }
}

// r[impl auth.test-utils.cookies]
pub fn session_cookie_header(token: &str, cookie: &AuthCookieConfig) -> String {
    format!("{}={token}", cookie.name)
}

// r[impl auth.test-utils.cookies]
pub fn bearer_header(token: &str) -> String {
    format!("Bearer {token}")
}

// r[impl auth.test-utils.fixtures]
pub fn test_auth_builder<S>(storage: S) -> crate::ArchitectAuthBuilder<S> {
    ArchitectAuth::builder()
        .secret(TEST_SECRET)
        .storage(storage)
}

#[cfg(feature = "backend-db")]
// r[impl auth.test-utils.fixtures]
pub async fn sqlite_harness() -> Result<
    AuthTestHarness<crate::backend_db::AuthSeaOrmStorage>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;

    let db = Database::connect("sqlite::memory:").await?;
    crate::backend_db::Migrator::up(&db, None).await?;
    let storage = crate::backend_db::AuthSeaOrmStorage::new(db);
    let auth = test_auth_builder(storage).build()?;
    Ok(AuthTestHarness::new(auth))
}

#[cfg(feature = "axum")]
pub mod axum {
    use ::axum::http::{HeaderMap, HeaderValue, header};

    use crate::AuthCookieConfig;

    // r[impl auth.test-utils.axum]
    pub fn session_headers(token: &str, cookie: &AuthCookieConfig) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&super::session_cookie_header(token, cookie))
                .expect("test cookie header is valid"),
        );
        headers
    }

    // r[impl auth.test-utils.axum]
    pub fn bearer_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&super::bearer_header(token))
                .expect("test authorization header is valid"),
        );
        headers
    }
}

#[cfg(feature = "vox")]
pub mod vox {
    // r[impl auth.test-utils.vox]
    pub fn bearer_middleware(
        token: impl Into<String>,
    ) -> crate::transport::vox::AuthClientMiddleware {
        crate::transport::vox::AuthClientMiddleware::bearer(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // r[verify auth.test-utils.cookies]
    fn cookie_and_bearer_headers_are_stable() {
        let cookie = AuthCookieConfig::default();

        assert_eq!(
            session_cookie_header("session-token", &cookie),
            "architect-auth.session=session-token"
        );
        assert_eq!(bearer_header("session-token"), "Bearer session-token");
    }

    #[cfg(feature = "backend-db")]
    #[tokio::test]
    // r[verify auth.test-utils.fixtures]
    // r[verify auth.test-utils.users]
    // r[verify auth.test-utils.sessions]
    // r[verify auth.test-utils.organizations]
    // r[verify auth.test-utils.teams]
    // r[verify auth.test-utils.api-keys]
    // r[verify auth.test-utils.otp]
    async fn sqlite_harness_creates_common_auth_fixtures() {
        let harness = sqlite_harness().await.expect("sqlite auth harness");
        let session = harness
            .create_user_session(TestUserInput::new("user@example.com"))
            .await
            .expect("create user session");
        let signed_in = harness
            .sign_in("user@example.com", TEST_PASSWORD)
            .await
            .expect("sign in");
        let organization = harness
            .create_organization(&signed_in.token, "acme")
            .await
            .expect("create organization");
        let role = harness
            .create_role(
                &signed_in.token,
                organization.organization.id,
                "billing",
                r#"{"team":["create"]}"#,
            )
            .await
            .expect("create role");
        let team = harness
            .create_team(&signed_in.token, organization.organization.id, "Platform")
            .await
            .expect("create team");
        let api_key = harness
            .create_api_key(&signed_in.token, Some(r#"{"team":["create"]}"#.into()))
            .await
            .expect("create api key");
        let otp = harness
            .send_email_otp("otp@example.com")
            .await
            .expect("send otp");
        let otp_result = harness
            .verify_email_otp("otp@example.com", otp.token, true)
            .await
            .expect("verify otp");
        let invitation = harness
            .create_invitation(
                &signed_in.token,
                organization.organization.id,
                "invitee@example.com",
                "member",
            )
            .await
            .expect("create invitation");

        assert_eq!(session.user.email.as_deref(), Some("user@example.com"));
        assert_eq!(organization.organization.slug, "acme");
        assert_eq!(role.role, "billing");
        assert_eq!(team.name, "Platform");
        assert!(api_key.key.starts_with("ak_"));
        assert!(otp_result.session.is_some());
        assert_eq!(invitation.invitation.email, "invitee@example.com");
    }

    #[cfg(feature = "axum")]
    #[test]
    // r[verify auth.test-utils.axum]
    fn axum_helpers_build_expected_headers() {
        let cookie = AuthCookieConfig::default();

        assert!(
            axum::session_headers("session-token", &cookie)
                .contains_key(::axum::http::header::COOKIE)
        );
        assert!(
            axum::bearer_headers("session-token").contains_key(::axum::http::header::AUTHORIZATION)
        );
    }

    #[cfg(feature = "vox")]
    #[test]
    // r[verify auth.test-utils.vox]
    fn vox_helper_builds_bearer_middleware() {
        let _middleware = vox::bearer_middleware("session-token");
    }
}
