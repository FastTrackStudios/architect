---
"architect-desktop" = "minor"
"architect-extensions" = "minor"
"architect-sites" = "minor"
---

Security: an island's web view frame gets no WebRTC by default (CSP webrtc 'block', and its script removes RTCPeerConnection and its kin, the Presentation API and sendBeacon before any bundle code runs; x-dns-prefetch-control off). Added the WebRTC capability: an extension declaring capabilities.webrtc = ["peer-connections"] and granted by its operator (SiteIsland::grant_webrtc) runs in its own document (WebRtc::Granted, webview_path_for, webview-rtc.<hash>.html) that allows it. A web view frame is answered and booted once, sizes once a frame, answers a heartbeat, and is taken down (the player's own controls back) when it is not ready in 15 s, stops answering, errors or leaves its document. /s/_signin and /s/_signout answer only Sec-Fetch-Site same-origin or none. document_headers and webview_headers return a Vec
