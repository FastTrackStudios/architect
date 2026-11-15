---
"architect-extensions" = "minor"
---

Added node actions (#583): an `[[actions]]` entry with `runs = "node"` and `secrets = [...]` runs on the org's node, holding secrets the person's devices never have; `Manifest::node_part`, the `NodeActions` service (`node::PERMITS`), `node::NodeHost` (feature `node`), and `Host::with_node` / `Host::set_node` to forward them from an app
