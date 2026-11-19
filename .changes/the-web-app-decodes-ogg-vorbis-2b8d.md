---
"architect-web" = "minor"
---

Added the web app's own Ogg Vorbis decoding, so a vault's recordings play on every browser (an iPhone's Safari before iOS 18.4 plays none): decoded in the page a couple of seconds ahead of the playhead and played through an AudioWorklet whose frame count is the clock the deck, a chart and lyrics follow; a seek lands on the exact frame. What it cannot open, and an audio context that does not start within two seconds of a press, goes to the browser's own player as before. The Media Session gets the item's cover and position (#909). Not yet heard through the worklet in a real browser.
