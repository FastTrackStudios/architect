//! SeaORM backend for architect-auth entities.

// r[impl auth.storage.repo-first]
// r[impl auth.storage.no-public-sql]
// r[impl auth.storage.backend-parity]
// r[impl auth.storage.transactions]
// r[impl auth.storage.clock]
pub const AUTH_DB_BACKEND: &str = "sea-orm";
pub const AUTH_DB_SUPPORTS_TRANSACTIONS: bool = true;
pub const AUTH_DB_CLOCK_SEMANTICS: &str = "backend-generated-utc";

// r[impl auth.storage.database-matrix]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthDatabaseKind {
    Sqlite,
    Postgres,
    MySql,
}

// r[impl auth.storage.database-matrix]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthDatabaseSupport {
    pub kind: AuthDatabaseKind,
    pub status: &'static str,
    pub ci_required: bool,
    pub notes: &'static str,
}

// r[impl auth.storage.database-matrix]
pub const AUTH_DATABASE_SUPPORT_MATRIX: &[AuthDatabaseSupport] = &[
    AuthDatabaseSupport {
        kind: AuthDatabaseKind::Sqlite,
        status: "supported",
        ci_required: true,
        notes: "CI runs in-memory SQLite migration and native storage coverage.",
    },
    AuthDatabaseSupport {
        kind: AuthDatabaseKind::Postgres,
        status: "planned",
        ci_required: false,
        notes: "Requires enabling sqlx-postgres SeaORM features and a CI service database.",
    },
    AuthDatabaseSupport {
        kind: AuthDatabaseKind::MySql,
        status: "planned",
        ci_required: false,
        notes: "Requires enabling sqlx-mysql SeaORM features and a CI service database.",
    },
];

pub fn auth_db_storage_capabilities() -> (&'static str, bool, &'static str) {
    (
        AUTH_DB_BACKEND,
        AUTH_DB_SUPPORTS_TRANSACTIONS,
        AUTH_DB_CLOCK_SEMANTICS,
    )
}

pub fn auth_database_support_matrix() -> &'static [AuthDatabaseSupport] {
    AUTH_DATABASE_SUPPORT_MATRIX
}

pub use auth_proto::account::{
    ActiveModel as AuthAccountActiveModel, AuthAccount, AuthAccountCreate, AuthAccountList,
    AuthAccountRepo, AuthAccountRepoStorage, AuthAccountUpdate, Column as AuthAccountColumn,
    Entity as AuthAccountEntity, Model as AuthAccountModel, Relation as AuthAccountRelation,
};
pub use auth_proto::api_key::{
    ActiveModel as AuthApiKeyActiveModel, AuthApiKey, AuthApiKeyCreate, AuthApiKeyList,
    AuthApiKeyRepo, AuthApiKeyRepoStorage, AuthApiKeyUpdate, Column as AuthApiKeyColumn,
    Entity as AuthApiKeyEntity, Model as AuthApiKeyModel, Relation as AuthApiKeyRelation,
};
pub use auth_proto::audit_event::{
    ActiveModel as AuthAuditEventRecordActiveModel, AuthAuditEventRecord,
    AuthAuditEventRecordCreate, AuthAuditEventRecordList, AuthAuditEventRecordRepo,
    AuthAuditEventRecordRepoStorage, AuthAuditEventRecordUpdate,
    Column as AuthAuditEventRecordColumn, Entity as AuthAuditEventRecordEntity,
    Model as AuthAuditEventRecordModel, Relation as AuthAuditEventRecordRelation,
};
pub use auth_proto::email_change::{
    ActiveModel as AuthEmailChangeActiveModel, AuthEmailChange, AuthEmailChangeCreate,
    AuthEmailChangeList, AuthEmailChangeRepo, AuthEmailChangeRepoStorage, AuthEmailChangeUpdate,
    Column as AuthEmailChangeColumn, Entity as AuthEmailChangeEntity,
    Model as AuthEmailChangeModel, Relation as AuthEmailChangeRelation,
};
pub use auth_proto::invitation::{
    ActiveModel as AuthInvitationActiveModel, AuthInvitation, AuthInvitationCreate,
    AuthInvitationList, AuthInvitationRepo, AuthInvitationRepoStorage, AuthInvitationUpdate,
    Column as AuthInvitationColumn, Entity as AuthInvitationEntity, InvitationStatus,
    Model as AuthInvitationModel, Relation as AuthInvitationRelation,
};
pub use auth_proto::member::{
    ActiveModel as AuthMemberActiveModel, AuthMember, AuthMemberCreate, AuthMemberList,
    AuthMemberRepo, AuthMemberRepoStorage, AuthMemberUpdate, Column as AuthMemberColumn,
    Entity as AuthMemberEntity, Model as AuthMemberModel, Relation as AuthMemberRelation,
};
pub use auth_proto::organization::{
    ActiveModel as AuthOrganizationActiveModel, AuthOrganization, AuthOrganizationCreate,
    AuthOrganizationList, AuthOrganizationRepo, AuthOrganizationRepoStorage,
    AuthOrganizationUpdate, Column as AuthOrganizationColumn, Entity as AuthOrganizationEntity,
    Model as AuthOrganizationModel, Relation as AuthOrganizationRelation,
};
pub use auth_proto::organization_role::{
    ActiveModel as AuthOrganizationRoleActiveModel, AuthOrganizationRole,
    AuthOrganizationRoleCreate, AuthOrganizationRoleList, AuthOrganizationRoleRepo,
    AuthOrganizationRoleRepoStorage, AuthOrganizationRoleUpdate,
    Column as AuthOrganizationRoleColumn, Entity as AuthOrganizationRoleEntity,
    Model as AuthOrganizationRoleModel, Relation as AuthOrganizationRoleRelation,
};
pub use auth_proto::passkey::{
    ActiveModel as AuthPasskeyActiveModel, AuthPasskey, AuthPasskeyCreate, AuthPasskeyList,
    AuthPasskeyRepo, AuthPasskeyRepoStorage, AuthPasskeyUpdate, Column as AuthPasskeyColumn,
    Entity as AuthPasskeyEntity, Model as AuthPasskeyModel, Relation as AuthPasskeyRelation,
};
pub use auth_proto::session::{
    ActiveModel as AuthSessionActiveModel, AuthSession, AuthSessionCreate, AuthSessionList,
    AuthSessionRepo, AuthSessionRepoStorage, AuthSessionUpdate, Column as AuthSessionColumn,
    Entity as AuthSessionEntity, Model as AuthSessionModel, Relation as AuthSessionRelation,
};
pub use auth_proto::team::{
    ActiveModel as AuthTeamActiveModel, AuthTeam, AuthTeamCreate, AuthTeamList, AuthTeamRepo,
    AuthTeamRepoStorage, AuthTeamUpdate, Column as AuthTeamColumn, Entity as AuthTeamEntity,
    Model as AuthTeamModel, Relation as AuthTeamRelation,
};
pub use auth_proto::team_member::{
    ActiveModel as AuthTeamMemberActiveModel, AuthTeamMember, AuthTeamMemberCreate,
    AuthTeamMemberList, AuthTeamMemberRepo, AuthTeamMemberRepoStorage, AuthTeamMemberUpdate,
    Column as AuthTeamMemberColumn, Entity as AuthTeamMemberEntity, Model as AuthTeamMemberModel,
    Relation as AuthTeamMemberRelation,
};
pub use auth_proto::two_factor::{
    ActiveModel as AuthTwoFactorActiveModel, AuthTwoFactor, AuthTwoFactorCreate, AuthTwoFactorList,
    AuthTwoFactorRepo, AuthTwoFactorRepoStorage, AuthTwoFactorUpdate,
    Column as AuthTwoFactorColumn, Entity as AuthTwoFactorEntity, Model as AuthTwoFactorModel,
    Relation as AuthTwoFactorRelation,
};
pub use auth_proto::user::{
    ActiveModel as AuthUserActiveModel, AuthUser, AuthUserCreate, AuthUserList, AuthUserRepo,
    AuthUserRepoStorage, AuthUserUpdate, Column as AuthUserColumn, Entity as AuthUserEntity,
    Model as AuthUserModel, Relation as AuthUserRelation,
};
pub use auth_proto::verification::{
    ActiveModel as AuthVerificationActiveModel, AuthVerification, AuthVerificationCreate,
    AuthVerificationList, AuthVerificationRepo, AuthVerificationRepoStorage,
    AuthVerificationUpdate, Column as AuthVerificationColumn, Entity as AuthVerificationEntity,
    Model as AuthVerificationModel, Relation as AuthVerificationRelation,
};

pub mod migrations;

pub use migrations::Migrator;

#[cfg(test)]
mod tests {
    use super::{AuthDatabaseKind, auth_database_support_matrix, auth_db_storage_capabilities};

    // r[verify auth.storage.repo-first]
    // r[verify auth.storage.no-public-sql]
    #[test]
    fn public_surface_exports_repositories_not_sql_adapters() {
        assert!(
            std::any::type_name::<super::AuthUserRepoStorage<sea_orm::DatabaseConnection>>()
                .contains("RepoStorage")
        );
        assert!(
            std::any::type_name::<super::AuthSessionRepoStorage<sea_orm::DatabaseConnection>>()
                .contains("RepoStorage")
        );
        assert!(
            std::any::type_name::<super::AuthAccountRepoStorage<sea_orm::DatabaseConnection>>()
                .contains("RepoStorage")
        );
    }

    // r[verify auth.storage.backend-parity]
    // r[verify auth.storage.transactions]
    // r[verify auth.storage.clock]
    #[test]
    fn sea_orm_backend_declares_storage_capabilities() {
        let (backend, transactions, clock) = auth_db_storage_capabilities();

        assert_eq!(backend, "sea-orm");
        assert!(transactions);
        assert_eq!(clock, "backend-generated-utc");
    }

    // r[verify auth.storage.database-matrix]
    #[test]
    fn database_support_matrix_names_ci_backed_database_set() {
        let matrix = auth_database_support_matrix();

        assert_eq!(matrix.len(), 3);
        assert!(matrix.iter().any(|entry| {
            entry.kind == AuthDatabaseKind::Sqlite
                && entry.status == "supported"
                && entry.ci_required
        }));
        assert!(matrix.iter().any(|entry| {
            entry.kind == AuthDatabaseKind::Postgres
                && entry.status == "planned"
                && !entry.ci_required
        }));
        assert!(matrix.iter().any(|entry| {
            entry.kind == AuthDatabaseKind::MySql && entry.status == "planned" && !entry.ci_required
        }));
    }
}
