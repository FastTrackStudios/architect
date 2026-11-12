---
"architect-web" = "minor"
"xtask" = "none"
---

Changed the web bundle (scripts/web-bundle.sh, architect-web-<X.Y.Z>.tar.gz): its root is now the site (apps/site, prerendered): the landing page, /download, /changelog and /extensions, with the docs at /docs/ and the web app at /app/ as before. The bundle carries ROUTES.md (scripts/web-routes.md), the rules a server must apply: the path, then <path>.html, then <path>/index.html, else 404.html with a 404; the content-hashed assets/ may be cached for good. A release builds it after its Linux and node assets and fails unless /download describes that release. Removed the old landing page (apps/web/landing) and the Vercel deploy.
