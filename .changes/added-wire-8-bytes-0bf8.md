---
"architect-extensions" = "minor"
"architect-scene" = "minor"
"architect-pages" = "none"
"architect-sites" = "none"
---

Added wire 8 (BYTES_WIRE): a process view's scene batches cross as bytes beside its edits (Frame::Batches, ProcessExtension::scene_view, scene::outbox, scene::layers_marker), at the edit that names each, instead of base64 in data-architect-layers; older peers keep the attribute. architect_scene::ops::Receiver::apply_owned keeps a font or image of at least SHARE_AT bytes as a share of its batch, not a copy (Resource::from_shared, Resources::receive_shared, Receiver::resources) (#835).
