---
"architect-files" = "minor"
"architect-extensions" = "minor"
---

Added a project's files for an org's signed-in members: the node serves a vault's folder or a held root's at `/org/{slug}/held/…` (`architect_files::held` `HeldFiles`: `GET list/{place}?folder=` with path, size, sha256 and a ticketed url per file, `POST ticket/{place}/{*path}`, `GET content/{place}/{*path}?ticket=` ranged; `{place}` `vault:<name>` or `root:<id>`), for the org lane's bearer in a header or an hour's read ticket, under the lane's gate. Extensions in the web app reach it with the host calls `file_list {place, folder?}` and `file_url {place, path}` (`HostRequest::FileList`, `HostRequest::FileUrl`, `reads = ["files"]`), answered by the app's `architect_extensions::held::HeldFilesSource` (`Host::set_held_files`); a host with none, a share link's guest's included, answers `unsupported`
