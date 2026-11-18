---
"architect-desktop" = "minor"
"architect-extensions" = "minor"
"crdt" = "minor"
---

An extension's presence is relayed by its host to wherever the document says its presence is hosted (crdt::presence::presence_host): this node's registry, or the node it names, dialled over the org's iroh lane by endpoint id (guest::presence, DocLane::node, Host::set_presence_link, Running::presence_link; SelfPresenceLane). A leader names its node; followers follow through the org with no address given (session#192).
