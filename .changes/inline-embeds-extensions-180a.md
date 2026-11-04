---
"architect-extensions" = "minor"
"architect-pages" = "minor"
---

Inline embeds (#180): `[[embeds]]` (`EmbedDecl`: a view or a page, `claims = { fence, note_type, embed_type }`, `max_height`) as a contribution, declared or at run time and withdrawn on unload; `Embeds::resolve(&Claim) -> Option<EmbedSpec>` (`Host::embeds`), `EXT_EMBEDS`; `EmbedView { claim, source, props, fallback }` mounts a linked view in the host's document or a page or process view as a sub-document sized to its content (`RemoteDocument::fit_content`/`content_height`, `RemoteEmbed`, `PageEmbed`, `use_embed_pages`); a view reads its input with `linked::use_embed`, a page with `useEmbed()` / Luau `embed()`. Breaking: `Contribution` gains `embeds`; pages' `Services` gains `embed`.
