---
"architect" = "minor"
"architect-accounts" = "minor"
"architect-desktop" = "minor"
"architect-sites" = "minor"
"architect-spaces" = "minor"
---

Added share audiences, the files page and Download all (#510): a `Share` carries named `ShareAudience`s, each a Bases filter over its scope on a link of its own (`ShareLinks::add_audience` / `remove_audience`; `SHARE_AUDIENCE_ADD`, `SHARE_AUDIENCE_REMOVE`, `SHARE_AUDIENCES`), built as its own site and checked against the vault on every request. Every link has `<link>_files/` (views `?by=order|note|property:<key>`) and `<link>_archive.zip`, a zip the node streams (`SHARE_ARCHIVE` saves it). New: `architect_sites::Options::filter`, `architect_sites::listing` (`parse_filter`, `select`, `groups`, `Grouping`), `architect_sites::archive::ZipWriter`, `Scope::ordered`, `architect::http::HttpClient::download`. Source changes for consumers: `Share` has a new field `audiences` (add `audiences: Vec::new()` to a literal), `ShareLinks` two new methods, `SiteError` a `Filter` variant, `Options` a `filter` field.
