---
"architect-desktop" = "minor"
"crdt" = "minor"
---

Added live sync between peers: `crdt::peer::bridge` keeps one document live between two registries, `crdt::peer::adopt_lineage` and `pull_with(.., Foreign::AdoptFirst)` settle a document seeded on two nodes (the second seed adopts the first's lineage), and `architect-desktop serve` keeps a bridge per note with each peer (`ARCHITECT_LIVE_MAX`), sweeping only as the fallback
