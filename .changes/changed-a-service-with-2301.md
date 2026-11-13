---
"architect" = "patch"
"architect-rpc-derive" = "patch"
---

Changed a service with a vox face so its generated http::router, http::mount and handlers build only under the defining crate's vox feature, as its BindHttp already did; its <Trait>HttpClient still builds without vox (fixes architect-spaces without vox, e.g. through architect-cli)
