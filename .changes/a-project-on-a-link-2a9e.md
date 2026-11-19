---
"architect-spaces" = "major"
"architect-desktop" = "minor"
"architect-accounts" = "minor"
---

Added a project on a share link, what a link offers to keep, and one link per ask. `Share::projects` (`ShareProject {root, sessions, access}`, `ProjectAccess::{Read, Edit}`, `ShareChange::Projects`, `validate_projects`, `is_session_document`, `PROJECTS_MAX`): a link carries a file root's shared session documents and exactly the files they reference, listed at `GET <link>_project.json` (`GuestProjects`, `GuestProject`, `ProjectSession`, `ProjectRef`, `ProjectFile`), served by hash at `<link>_blob/<sha256>/<name>` and saved back with `PUT <link>_project/<n>/<path>` only with `Edit`, by a signed-in viewer (`node::share_project`). `Share::download` (`Download::{Everything, NotMedia, Nothing}`, `ShareChange::Download`, `is_media`): a file that is not offered is marked (`ListedFile::stream_only`, `"download"` in `_files.json`) and left out of the archive, and still plays. `Share::is_the_link_of`: sharing the same scope with the same audience, access and layer again answers the link that is already there. `SHARE_UPDATE` takes `download` and `projects`; a share's row has `download` and `projects`.

Migration: `Share` has two new fields (`download`, `projects`) and `ListedFile` one (`stream_only`), so code that builds either by literal adds them; `ShareChange` has two new variants, so an exhaustive `match` on it adds arms. `ShareLinks::create` may now answer an existing share (same code) where it made a second one.
