---
"architect-accounts" = "minor"
"architect-app" = "minor"
---

Added architect-accounts: accounts on an issuer for client apps — sessions in the platform keychain (Keychain, ARCHITECT_KEYCHAIN_SERVICE), sign-in in the browser by code or by email and password, switch, sign out revoked at the issuer, the account's devices (Accounts, AccountStore, ActiveSession), the shell's Account item with the switcher and the sign-in sheet (ui::use_accounts, AccountsUi::shell_account, AccountSheet), and ACCOUNT_LIST, ACCOUNT_SIGN_IN, ACCOUNT_SWITCH, ACCOUNT_SIGN_OUT, ACCOUNT_DEVICES, ACCOUNT_FORGET_DEVICE. The Architect app signs in with it against ARCHITECT_AUTH_ISSUER (auth.fasttrackstudio.app by default).
