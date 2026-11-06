---
"architect-accounts" = "minor"
"architect-app" = "minor"
"architect-orgs" = "minor"
---

Added orgs to architect-accounts: each account's orgs from the issuer (Accounts::refresh_orgs, Issuer::organizations), the current org kept per account in accounts.json (Account::orgs, AccountStore::switch_org, Accounts::switch_org, told to the issuer as the session's active org), ActiveSession::org/scope/account as the seam vaults, spaces and vox clients key off, device_org for the person's own org on this device, the org switcher in AccountsUi::shell_account, and the ORG_LIST and ORG_SWITCH actions. architect-orgs adds Orgs::find and OrgHandle::replace. The Architect app's window orgs follow the active account (use_account_orgs).
