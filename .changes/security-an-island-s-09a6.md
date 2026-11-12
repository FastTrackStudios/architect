---
"architect-desktop" = "minor"
"architect-extensions" = "minor"
"architect-sites" = "minor"
---

Security: an island's web view frame gets no WebRTC by default: its script removes RTCPeerConnection and its kin (by name and by pattern), the Presentation API and sendBeacon before any bundle code runs, the control on every engine (its CSP's webrtc 'block' is forward-looking only: no engine enforces it); x-dns-prefetch-control off. Added the WebRTC capability: an extension declaring capabilities.webrtc = ["peer-connections"] and granted by its operator (SiteIsland::grant_webrtc) runs in its own document (WebRtc::Granted, webview_path_for, webview-rtc.<hash>.html) that allows it, served only on a node where some island is granted it. A web view frame is answered and booted once, sizes once a frame, answers a heartbeat, and is taken down (the player's own controls back) when it is not ready in 15 s, falls silent (6 to 10 s), errors or leaves its document; never for time the page spent hidden. Sign-out (/s/_signout) is a form posted from the node's own page with its Origin checked (a GET only asks), on every browser; /s/_signin answers only Sec-Fetch-Site same-origin or none. document_headers and webview_headers return a Vec
