---
"architect-extensions" = "minor"
"architect-web" = "minor"
---

Added the document API, document events and kept data in the web app (#495 H1, H2, H4): `WebDocuments` over the web vault (the demo's and the signed-in org's, edits sent to the node), the person's edits raised as `doc.*`, `hostapi::DataStore` with `Host::with_data_store` (extension data in the browser's `localStorage`), `ui::use_page_setting_values` (extensions' settings as the settings page keeps them); a demo note now follows changes made to it elsewhere.
