---
"crdt" = "major"
---

Added one `architect-desktop serve` process for many orgs on one origin (#538): `host_orgs` (`ARCHITECT_HOST_ORGS=a,b`, or `*` for every org the session is in) beside `host_org`, a lane each at `/org/{slug}/vox` (`node::org::mount`), each serving only its own org's vaults, notes, index, live documents and share links (`node::org::VaultOrgs`: `of`, `holds`, `serves`, `of_name`); a vault is the org's whose slug is its id, or the one `vault_orgs` (`ARCHITECT_VAULT_ORGS`) names (`Config::hosted`, `Hosted`, `Config::org_of_vault`); `enrol_orgs` (`ARCHITECT_ENROL_ORGS`) enrols a node in several orgs at one host; `NodeStatus::hosted` lists the hosted orgs; a share link's code at the root of the origin, `/<code>`, is sent to `/s/<code>/` when the node holds it. `crdt::p2p::NodeAccess::org_docs_of` holds an org's enrolled devices to the documents that are its org's (`OrgDocs::holds`, `HoldsDoc`).

Migration: `crdt::p2p::NodeAccess::org` (an `Option<OrgDocs>`) is now `NodeAccess::orgs` (a `Vec<OrgDocs>`), and `OrgDocs` has a new field `holds`; `NodeAccess::org_docs` is unchanged. `node::shares::Shares::new` takes a `VaultOrgs` where it took an `Option<NodeOrg>`; `node::service::Access::key` takes the vault too; `node::VaultCatalog::new` is no longer `const`.
