// A page that runs away (apps/web): its render loops forever once
// "Spin" is clicked, or a click's handler does. In a browser its Web
// Worker is terminated past its frame's time and the page starts again
// from this file under a banner that says to use a worker; the rest of
// the app never waits.
import { h, useState } from "architect";

const button = { padding: "5px 12px", borderRadius: 6, border: "1px solid $border", background: "$secondary", color: "$secondary-foreground" };

export default function Runaway() {
  const [n, setN] = useState(0);
  const [spin, setSpin] = useState(false);
  if (spin) {
    for (;;) {}
  }
  return (
    <div id="runaway" style={{ display: "flex", flexDirection: "column", gap: 10, padding: 24 }}>
      <div style={{ fontSize: 18, fontWeight: 700 }}>A runaway page</div>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <button id="inc" style={button} onClick={() => setN(n + 1)}>Add one</button>
        <span id="count">{n}</span>
      </div>
      <div style={{ display: "flex", gap: 8 }}>
        <button id="spin" style={button} onClick={() => setSpin(true)}>Spin in its render</button>
        <button id="spin-handler" style={button} onClick={() => { for (;;) {} }}>Spin in a handler</button>
      </div>
    </div>
  );
}
