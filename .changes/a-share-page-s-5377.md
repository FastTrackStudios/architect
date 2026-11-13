---
"architect-desktop" = "patch"
---

A share page's sign-in (the viewer check, starting a sign-in and polling it) gives up on an issuer that does not answer within the node's 10 s issuer timeout, answering 502 and keeping the sign-in pending, instead of hanging the page.
