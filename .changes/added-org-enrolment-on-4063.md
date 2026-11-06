---
"architect-accounts" = "minor"
"architect-desktop" = "minor"
"crdt" = "minor"
---

Added org enrolment on the headless app: `architect-desktop serve` with `host_org` serves `DeviceEnrolment` at `/org/{slug}/vox` (an issuer session is the bearer), a node with `enrol_at`/`enrol_org` enrols itself there once, an enrolled device syncs the vaults as its role allows (`crdt::p2p::NodeAccess::org_docs`), and the app lists, enrols and revokes with `ORG_DEVICES`, `ORG_ENROL` and `ORG_REVOKE` and the Account menu's org devices page (`Account::org_hosts`, `Sheet::OrgDevices`)
