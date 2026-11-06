---
"architect-app" = "minor"
"architect-capture" = "minor"
"architect-extensions" = "minor"
"architect-guide" = "minor"
"architect-navigator-extension" = "minor"
"architect-projects" = "minor"
"architect-sandbox-board" = "minor"
"architect-sandbox-media" = "minor"
"architect-tasks" = "minor"
"architect-time-tracker" = "minor"
---

Added extension tours (#379): the [[tours]] manifest point (TourDecl, Manifest::tours, Contributions::tours), a guide scenario in the package checked at load and played as the extension, withdrawn when it is turned off; Tours::offer, Tours::invite and TourInfo::owner in architect-guide; TOURS_LIST and TOUR_START in the Architect app, the manager's "Take the tour" (op tour, Host::on_tour / Host::start_tour) and a one-time invitation after an install or enable (Extensions TURNED_ON); ⌘K rows for tours (a `tours:` palette source; TOUR_START with no tour opens it); tours for Projects, Tasks, the Time Tracker, the Navigator, capture (architect_capture::TOUR, with CAPTURE_CLOSE) and the sandbox's Board, each played as a test (architect_app::builtin_tours). Fixed a pane that switches between two views of one linked extension reusing the first view's hooks (a panic opening a project's page from the projects list).
