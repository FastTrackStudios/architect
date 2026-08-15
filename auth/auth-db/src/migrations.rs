//! SeaORM migrator for architect-auth tables.

use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260513_000001_create_auth_tables::Migration),
            Box::new(m20260809_000001_create_email_history::Migration),
        ]
    }
}

mod m20260513_000001_create_auth_tables {
    use sea_orm_migration::prelude::*;

    #[derive(DeriveMigrationName)]
    pub struct Migration;

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            create_tables(manager).await?;
            create_indexes(manager).await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            drop_table(manager, AuthPasskeys::Table).await?;
            drop_table(manager, AuthTeamMembers::Table).await?;
            drop_table(manager, AuthTeams::Table).await?;
            drop_table(manager, AuthOrganizationRoles::Table).await?;
            drop_table(manager, AuthAuditEvents::Table).await?;
            drop_table(manager, AuthApiKeys::Table).await?;
            drop_table(manager, AuthTwoFactors::Table).await?;
            drop_table(manager, AuthVerifications::Table).await?;
            drop_table(manager, AuthInvitations::Table).await?;
            drop_table(manager, AuthMembers::Table).await?;
            drop_table(manager, AuthOrganizations::Table).await?;
            drop_table(manager, AuthAccounts::Table).await?;
            drop_table(manager, AuthSessions::Table).await?;
            drop_table(manager, AuthUsers::Table).await?;

            Ok(())
        }
    }

    async fn drop_table<T>(manager: &SchemaManager<'_>, table: T) -> Result<(), DbErr>
    where
        T: IntoIden + 'static,
    {
        manager
            .drop_table(Table::drop().table(table).if_exists().to_owned())
            .await
    }

    // r[impl auth.storage.migrations-create-tables]
    async fn create_tables(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AuthUsers::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthUsers::Id))
                    .col(ColumnDef::new(AuthUsers::Email).string())
                    .col(ColumnDef::new(AuthUsers::Name).string())
                    .col(
                        ColumnDef::new(AuthUsers::EmailVerified)
                            .boolean()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AuthUsers::Image).string())
                    .col(ColumnDef::new(AuthUsers::Username).string())
                    .col(ColumnDef::new(AuthUsers::DisplayUsername).string())
                    .col(
                        ColumnDef::new(AuthUsers::TwoFactorEnabled)
                            .boolean()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AuthUsers::Role).string())
                    .col(ColumnDef::new(AuthUsers::Banned).boolean().not_null())
                    .col(ColumnDef::new(AuthUsers::BanReason).text())
                    .col(ColumnDef::new(AuthUsers::BanExpires).timestamp_with_time_zone())
                    .col(ColumnDef::new(AuthUsers::MetadataJson).text().not_null())
                    .col(ts(AuthUsers::CreatedAt))
                    .col(ts(AuthUsers::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthSessions::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthSessions::Id))
                    .col(ColumnDef::new(AuthSessions::UserId).uuid().not_null())
                    .col(ColumnDef::new(AuthSessions::TokenHash).string().not_null())
                    .col(ts(AuthSessions::ExpiresAt))
                    .col(ColumnDef::new(AuthSessions::IpAddress).string())
                    .col(ColumnDef::new(AuthSessions::UserAgent).string())
                    .col(ColumnDef::new(AuthSessions::ImpersonatedBy).uuid())
                    .col(ColumnDef::new(AuthSessions::ActiveOrganizationId).uuid())
                    .col(ColumnDef::new(AuthSessions::Active).boolean().not_null())
                    .col(ts(AuthSessions::CreatedAt))
                    .col(ts(AuthSessions::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthAccounts::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthAccounts::Id))
                    .col(ColumnDef::new(AuthAccounts::AccountId).string().not_null())
                    .col(ColumnDef::new(AuthAccounts::ProviderId).string().not_null())
                    .col(ColumnDef::new(AuthAccounts::UserId).uuid().not_null())
                    .col(ColumnDef::new(AuthAccounts::AccessTokenCiphertext).text())
                    .col(ColumnDef::new(AuthAccounts::RefreshTokenCiphertext).text())
                    .col(ColumnDef::new(AuthAccounts::IdTokenCiphertext).text())
                    .col(
                        ColumnDef::new(AuthAccounts::AccessTokenExpiresAt)
                            .timestamp_with_time_zone(),
                    )
                    .col(
                        ColumnDef::new(AuthAccounts::RefreshTokenExpiresAt)
                            .timestamp_with_time_zone(),
                    )
                    .col(ColumnDef::new(AuthAccounts::Scope).text())
                    .col(ColumnDef::new(AuthAccounts::PasswordHash).text())
                    .col(ts(AuthAccounts::CreatedAt))
                    .col(ts(AuthAccounts::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthOrganizations::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthOrganizations::Id))
                    .col(ColumnDef::new(AuthOrganizations::Name).string().not_null())
                    .col(ColumnDef::new(AuthOrganizations::Slug).string().not_null())
                    .col(ColumnDef::new(AuthOrganizations::Logo).string())
                    .col(ColumnDef::new(AuthOrganizations::MetadataJson).text())
                    .col(ts(AuthOrganizations::CreatedAt))
                    .col(ts(AuthOrganizations::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthOrganizationRoles::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthOrganizationRoles::Id))
                    .col(
                        ColumnDef::new(AuthOrganizationRoles::OrganizationId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AuthOrganizationRoles::Role)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AuthOrganizationRoles::PermissionsJson)
                            .text()
                            .not_null(),
                    )
                    .col(ts(AuthOrganizationRoles::CreatedAt))
                    .col(ts(AuthOrganizationRoles::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthMembers::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthMembers::Id))
                    .col(
                        ColumnDef::new(AuthMembers::OrganizationId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AuthMembers::UserId).uuid().not_null())
                    .col(ColumnDef::new(AuthMembers::Role).string().not_null())
                    .col(ts(AuthMembers::CreatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthTeams::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthTeams::Id))
                    .col(ColumnDef::new(AuthTeams::OrganizationId).uuid().not_null())
                    .col(ColumnDef::new(AuthTeams::Name).string().not_null())
                    .col(ts(AuthTeams::CreatedAt))
                    .col(ts(AuthTeams::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthTeamMembers::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthTeamMembers::Id))
                    .col(ColumnDef::new(AuthTeamMembers::TeamId).uuid().not_null())
                    .col(ColumnDef::new(AuthTeamMembers::UserId).uuid().not_null())
                    .col(ts(AuthTeamMembers::CreatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthInvitations::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthInvitations::Id))
                    .col(
                        ColumnDef::new(AuthInvitations::OrganizationId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AuthInvitations::Email).string().not_null())
                    .col(ColumnDef::new(AuthInvitations::Role).string().not_null())
                    .col(ColumnDef::new(AuthInvitations::Status).string().not_null())
                    .col(ColumnDef::new(AuthInvitations::InviterId).uuid().not_null())
                    .col(ts(AuthInvitations::ExpiresAt))
                    .col(ts(AuthInvitations::CreatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthVerifications::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthVerifications::Id))
                    .col(
                        ColumnDef::new(AuthVerifications::Identifier)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AuthVerifications::ValueHash)
                            .string()
                            .not_null(),
                    )
                    .col(ts(AuthVerifications::ExpiresAt))
                    .col(ts(AuthVerifications::CreatedAt))
                    .col(ts(AuthVerifications::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthTwoFactors::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthTwoFactors::Id))
                    .col(ColumnDef::new(AuthTwoFactors::UserId).uuid().not_null())
                    .col(
                        ColumnDef::new(AuthTwoFactors::SecretCiphertext)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AuthTwoFactors::BackupCodesHash).text())
                    .col(
                        ColumnDef::new(AuthTwoFactors::AttemptCount)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ts(AuthTwoFactors::CreatedAt))
                    .col(ts(AuthTwoFactors::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthAuditEvents::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthAuditEvents::Id))
                    .col(ColumnDef::new(AuthAuditEvents::ActorId).uuid().not_null())
                    .col(ColumnDef::new(AuthAuditEvents::TargetId).uuid())
                    .col(ColumnDef::new(AuthAuditEvents::Action).string().not_null())
                    .col(ts(AuthAuditEvents::CreatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthApiKeys::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthApiKeys::Id))
                    .col(ColumnDef::new(AuthApiKeys::Name).string())
                    .col(ColumnDef::new(AuthApiKeys::Prefix).string())
                    .col(ColumnDef::new(AuthApiKeys::KeyHash).string().not_null())
                    .col(ColumnDef::new(AuthApiKeys::UserId).uuid().not_null())
                    .col(ColumnDef::new(AuthApiKeys::Enabled).boolean().not_null())
                    .col(
                        ColumnDef::new(AuthApiKeys::RateLimitEnabled)
                            .boolean()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AuthApiKeys::RateLimitTimeWindow).big_integer())
                    .col(ColumnDef::new(AuthApiKeys::RateLimitMax).big_integer())
                    .col(ColumnDef::new(AuthApiKeys::RequestCount).big_integer())
                    .col(ColumnDef::new(AuthApiKeys::Remaining).big_integer())
                    .col(ColumnDef::new(AuthApiKeys::ExpiresAt).timestamp_with_time_zone())
                    .col(ColumnDef::new(AuthApiKeys::PermissionsJson).text())
                    .col(ColumnDef::new(AuthApiKeys::MetadataJson).text())
                    .col(ts(AuthApiKeys::CreatedAt))
                    .col(ts(AuthApiKeys::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuthPasskeys::Table)
                    .if_not_exists()
                    .col(uuid_pk(AuthPasskeys::Id))
                    .col(ColumnDef::new(AuthPasskeys::Name).string().not_null())
                    .col(ColumnDef::new(AuthPasskeys::UserId).uuid().not_null())
                    .col(ColumnDef::new(AuthPasskeys::PublicKey).text().not_null())
                    .col(
                        ColumnDef::new(AuthPasskeys::CredentialId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AuthPasskeys::Counter)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AuthPasskeys::DeviceType).string().not_null())
                    .col(ColumnDef::new(AuthPasskeys::BackedUp).boolean().not_null())
                    .col(ColumnDef::new(AuthPasskeys::Transports).text())
                    .col(ts(AuthPasskeys::CreatedAt))
                    .to_owned(),
            )
            .await
    }

    // r[impl auth.storage.unique-indexes]
    // r[impl auth.storage.lookup-indexes]
    async fn create_indexes(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        for index in [
            unique("idx_auth_users_email", AuthUsers::Table, [AuthUsers::Email]),
            unique(
                "idx_auth_users_username",
                AuthUsers::Table,
                [AuthUsers::Username],
            ),
            index(
                "idx_auth_sessions_user_id",
                AuthSessions::Table,
                [AuthSessions::UserId],
            ),
            unique(
                "idx_auth_sessions_token_hash",
                AuthSessions::Table,
                [AuthSessions::TokenHash],
            ),
            index(
                "idx_auth_sessions_expires_at",
                AuthSessions::Table,
                [AuthSessions::ExpiresAt],
            ),
            unique(
                "idx_auth_accounts_provider_account",
                AuthAccounts::Table,
                [AuthAccounts::ProviderId, AuthAccounts::AccountId],
            ),
            index(
                "idx_auth_accounts_user_id",
                AuthAccounts::Table,
                [AuthAccounts::UserId],
            ),
            unique(
                "idx_auth_organizations_slug",
                AuthOrganizations::Table,
                [AuthOrganizations::Slug],
            ),
            unique(
                "idx_auth_org_roles_org_role",
                AuthOrganizationRoles::Table,
                [
                    AuthOrganizationRoles::OrganizationId,
                    AuthOrganizationRoles::Role,
                ],
            ),
            index(
                "idx_auth_org_roles_org",
                AuthOrganizationRoles::Table,
                [AuthOrganizationRoles::OrganizationId],
            ),
            unique(
                "idx_auth_members_org_user",
                AuthMembers::Table,
                [AuthMembers::OrganizationId, AuthMembers::UserId],
            ),
            index(
                "idx_auth_teams_org",
                AuthTeams::Table,
                [AuthTeams::OrganizationId],
            ),
            unique(
                "idx_auth_team_members_team_user",
                AuthTeamMembers::Table,
                [AuthTeamMembers::TeamId, AuthTeamMembers::UserId],
            ),
            index(
                "idx_auth_team_members_user",
                AuthTeamMembers::Table,
                [AuthTeamMembers::UserId],
            ),
            index(
                "idx_auth_invitations_org",
                AuthInvitations::Table,
                [AuthInvitations::OrganizationId],
            ),
            index(
                "idx_auth_invitations_email",
                AuthInvitations::Table,
                [AuthInvitations::Email],
            ),
            index(
                "idx_auth_invitations_expires_at",
                AuthInvitations::Table,
                [AuthInvitations::ExpiresAt],
            ),
            index(
                "idx_auth_verifications_identifier",
                AuthVerifications::Table,
                [AuthVerifications::Identifier],
            ),
            index(
                "idx_auth_verifications_expires_at",
                AuthVerifications::Table,
                [AuthVerifications::ExpiresAt],
            ),
            unique(
                "idx_auth_two_factors_user_id",
                AuthTwoFactors::Table,
                [AuthTwoFactors::UserId],
            ),
            index(
                "idx_auth_audit_events_actor",
                AuthAuditEvents::Table,
                [AuthAuditEvents::ActorId],
            ),
            index(
                "idx_auth_audit_events_target",
                AuthAuditEvents::Table,
                [AuthAuditEvents::TargetId],
            ),
            index(
                "idx_auth_audit_events_created_at",
                AuthAuditEvents::Table,
                [AuthAuditEvents::CreatedAt],
            ),
            unique(
                "idx_auth_api_keys_hash",
                AuthApiKeys::Table,
                [AuthApiKeys::KeyHash],
            ),
            index(
                "idx_auth_api_keys_user_id",
                AuthApiKeys::Table,
                [AuthApiKeys::UserId],
            ),
            index(
                "idx_auth_api_keys_expires_at",
                AuthApiKeys::Table,
                [AuthApiKeys::ExpiresAt],
            ),
            unique(
                "idx_auth_passkeys_credential_id",
                AuthPasskeys::Table,
                [AuthPasskeys::CredentialId],
            ),
            index(
                "idx_auth_passkeys_user_id",
                AuthPasskeys::Table,
                [AuthPasskeys::UserId],
            ),
        ] {
            manager.create_index(index).await?;
        }

        Ok(())
    }

    fn uuid_pk<T>(name: T) -> ColumnDef
    where
        T: IntoIden,
    {
        let mut column = ColumnDef::new(name);
        column.uuid().not_null().primary_key();
        column
    }

    fn ts<T>(name: T) -> ColumnDef
    where
        T: IntoIden,
    {
        let mut column = ColumnDef::new(name);
        column.timestamp_with_time_zone().not_null();
        column
    }

    fn index<T, C, const N: usize>(name: &str, table: T, columns: [C; N]) -> IndexCreateStatement
    where
        T: IntoIden + 'static,
        C: IntoIden,
    {
        let mut index = Index::create();
        index.name(name).table(table).if_not_exists();
        for column in columns {
            index.col(column);
        }
        index.to_owned()
    }

    fn unique<T, C, const N: usize>(name: &str, table: T, columns: [C; N]) -> IndexCreateStatement
    where
        T: IntoIden + 'static,
        C: IntoIden,
    {
        let mut index = Index::create();
        index.name(name).table(table).unique().if_not_exists();
        for column in columns {
            index.col(column);
        }
        index.to_owned()
    }

    #[derive(Iden)]
    enum AuthUsers {
        Table,
        Id,
        Email,
        Name,
        EmailVerified,
        Image,
        Username,
        DisplayUsername,
        TwoFactorEnabled,
        Role,
        Banned,
        BanReason,
        BanExpires,
        MetadataJson,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(Iden)]
    enum AuthSessions {
        Table,
        Id,
        UserId,
        TokenHash,
        ExpiresAt,
        IpAddress,
        UserAgent,
        ImpersonatedBy,
        ActiveOrganizationId,
        Active,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(Iden)]
    enum AuthAccounts {
        Table,
        Id,
        AccountId,
        ProviderId,
        UserId,
        AccessTokenCiphertext,
        RefreshTokenCiphertext,
        IdTokenCiphertext,
        AccessTokenExpiresAt,
        RefreshTokenExpiresAt,
        Scope,
        PasswordHash,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(Iden)]
    enum AuthOrganizations {
        Table,
        Id,
        Name,
        Slug,
        Logo,
        MetadataJson,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(Iden)]
    enum AuthOrganizationRoles {
        Table,
        Id,
        OrganizationId,
        Role,
        PermissionsJson,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(Iden)]
    enum AuthMembers {
        Table,
        Id,
        OrganizationId,
        UserId,
        Role,
        CreatedAt,
    }

    #[derive(Iden)]
    enum AuthTeams {
        Table,
        Id,
        OrganizationId,
        Name,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(Iden)]
    enum AuthTeamMembers {
        Table,
        Id,
        TeamId,
        UserId,
        CreatedAt,
    }

    #[derive(Iden)]
    enum AuthInvitations {
        Table,
        Id,
        OrganizationId,
        Email,
        Role,
        Status,
        InviterId,
        ExpiresAt,
        CreatedAt,
    }

    #[derive(Iden)]
    enum AuthVerifications {
        Table,
        Id,
        Identifier,
        ValueHash,
        ExpiresAt,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(Iden)]
    enum AuthTwoFactors {
        Table,
        Id,
        UserId,
        SecretCiphertext,
        BackupCodesHash,
        AttemptCount,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(Iden)]
    enum AuthAuditEvents {
        Table,
        Id,
        ActorId,
        TargetId,
        Action,
        CreatedAt,
    }

    #[derive(Iden)]
    enum AuthApiKeys {
        Table,
        Id,
        Name,
        Prefix,
        KeyHash,
        UserId,
        Enabled,
        RateLimitEnabled,
        RateLimitTimeWindow,
        RateLimitMax,
        RequestCount,
        Remaining,
        ExpiresAt,
        PermissionsJson,
        MetadataJson,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(Iden)]
    enum AuthPasskeys {
        Table,
        Id,
        Name,
        UserId,
        PublicKey,
        CredentialId,
        Counter,
        DeviceType,
        BackedUp,
        Transports,
        CreatedAt,
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    use sea_orm_migration::MigratorTrait;

    use super::Migrator;

    // r[verify auth.storage.migrations-create-tables]
    // r[verify auth.storage.unique-indexes]
    // r[verify auth.storage.lookup-indexes]
    #[tokio::test]
    async fn migrator_applies_and_rolls_back() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory database");

        Migrator::up(&db, None)
            .await
            .expect("apply auth migrations");
        Migrator::down(&db, None)
            .await
            .expect("roll back auth migrations");
    }

    // r[verify auth.storage.migration-compatibility]
    // r[verify auth.storage.database-matrix]
    #[tokio::test]
    async fn sqlite_migration_is_idempotent_for_deployed_schema() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory database");

        Migrator::up(&db, None)
            .await
            .expect("apply auth migrations first time");
        Migrator::up(&db, None)
            .await
            .expect("re-apply migrations against deployed schema");

        let tables = sqlite_names(
            &db,
            "select name from sqlite_master where type = 'table' and name like 'auth_%' order by name",
        )
        .await;
        for table in [
            "auth_accounts",
            "auth_api_keys",
            "auth_audit_events",
            "auth_invitations",
            "auth_members",
            "auth_organization_roles",
            "auth_organizations",
            "auth_passkeys",
            "auth_sessions",
            "auth_team_members",
            "auth_teams",
            "auth_two_factors",
            "auth_users",
            "auth_verifications",
        ] {
            assert!(tables.contains(&table.to_owned()), "{table} table exists");
        }

        let indexes = sqlite_names(
            &db,
            "select name from sqlite_master where type = 'index' and name like 'idx_auth_%' order by name",
        )
        .await;
        for index in [
            "idx_auth_users_email",
            "idx_auth_sessions_token_hash",
            "idx_auth_accounts_provider_account",
            "idx_auth_organizations_slug",
            "idx_auth_team_members_team_user",
            "idx_auth_api_keys_hash",
            "idx_auth_passkeys_credential_id",
        ] {
            assert!(indexes.contains(&index.to_owned()), "{index} index exists");
        }
    }

    async fn sqlite_names(db: &sea_orm::DatabaseConnection, sql: &str) -> Vec<String> {
        db.query_all(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
            .await
            .expect("query sqlite schema")
            .into_iter()
            .map(|row| row.try_get::<String>("", "name").expect("schema name"))
            .collect()
    }
}

/// Append-only trail of every email an account has held.
///
/// A SEPARATE migration rather than a column on the original table: that
/// one has already run everywhere, and editing an applied migration means
/// existing databases silently never get the change. Sea-orm tracks these
/// by name, so a new one is the only thing that reaches a live server.
mod m20260809_000001_create_email_history {
    use sea_orm_migration::prelude::*;

    pub struct Migration;

    // NOT `#[derive(DeriveMigrationName)]`. That derives the name from the
    // FILE, and both migrations live in this one — so deriving gives them
    // the same version string and the second insert trips
    // `UNIQUE constraint failed: seaql_migrations.version`, taking auth
    // down on boot. Named explicitly instead, which is also what makes it
    // safe to keep adding migrations to this file.
    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m20260809_000001_create_email_history"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table(AuthUserEmailHistory::Table)
                        .if_not_exists()
                        .col(
                            ColumnDef::new(AuthUserEmailHistory::Id)
                                .uuid()
                                .not_null()
                                .primary_key(),
                        )
                        .col(ColumnDef::new(AuthUserEmailHistory::UserId).uuid().not_null())
                        // Nullable: `auth_users.email` is nullable, so an
                        // account can genuinely have had no address before.
                        .col(ColumnDef::new(AuthUserEmailHistory::PreviousEmail).string())
                        .col(
                            ColumnDef::new(AuthUserEmailHistory::NewEmail)
                                .string()
                                .not_null(),
                        )
                        // NULL = the user changed their own address.
                        .col(ColumnDef::new(AuthUserEmailHistory::ChangedBy).uuid())
                        .col(ColumnDef::new(AuthUserEmailHistory::Reason).text())
                        .col(
                            ColumnDef::new(AuthUserEmailHistory::CreatedAt)
                                .timestamp_with_time_zone()
                                .not_null(),
                        )
                        .to_owned(),
                )
                .await?;

            // Both directions are real queries: "what has this account
            // been called" and "who was old@example.com".
            manager
                .create_index(
                    Index::create()
                        .name("idx_auth_user_email_history_user_id")
                        .table(AuthUserEmailHistory::Table)
                        .col(AuthUserEmailHistory::UserId)
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
            manager
                .create_index(
                    Index::create()
                        .name("idx_auth_user_email_history_previous_email")
                        .table(AuthUserEmailHistory::Table)
                        .col(AuthUserEmailHistory::PreviousEmail)
                        .if_not_exists()
                        .to_owned(),
                )
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .drop_table(
                    Table::drop()
                        .table(AuthUserEmailHistory::Table)
                        .if_exists()
                        .to_owned(),
                )
                .await
        }
    }

    #[derive(Iden)]
    enum AuthUserEmailHistory {
        Table,
        Id,
        UserId,
        PreviousEmail,
        NewEmail,
        ChangedBy,
        Reason,
        CreatedAt,
    }
}
