// The Board's Preact page (docs/content/architecture/pages.md): Preact
// itself, the files in vendor/ (no Node, no bundler), rendering to
// architect/dom's document in the page's own runtime — QuickJS in the
// app, a Web Worker in a browser (apps/sandbox/web). What it does to that
// document reaches the pane as any page's edits; a click comes back to
// its listener as a DOM event.
import { h, render } from "./vendor/preact.js";
import { useState } from "./vendor/hooks.js";
import { document } from "architect/dom";

const row = "display: flex; gap: 8px; align-items: center;";
const button = {
  padding: "5px 12px",
  borderRadius: "6px",
  border: "1px solid var(--border)",
  background: "var(--secondary)",
  color: "var(--secondary-foreground)",
};

function Tally() {
  const [count, setCount] = useState(0);
  const [items, setItems] = useState<string[]>([]);
  const [draft, setDraft] = useState("");
  const add = () => {
    setItems([...items, draft.trim() || `Item ${items.length + 1}`]);
    setDraft("");
  };
  return (
    <div id="preact-page" style={{ padding: "16px", display: "flex", flexDirection: "column", gap: "12px" }}>
      <h2 id="preact-title" style={{ margin: 0, fontSize: "20px" }}>
        Preact
      </h2>
      <div style={{ color: "var(--muted-foreground)" }}>Preact renders to a DOM in this page's runtime; the pane draws its edits.</div>
      <div style={row}>
        <button id="preact-inc" style={button} onClick={() => setCount((c) => c + 1)}>
          +1
        </button>
        <button id="preact-reset" style={button} onClick={() => setCount(0)}>
          Reset
        </button>
        <span id="preact-count" style={{ fontVariantNumeric: "tabular-nums", fontSize: "16px" }}>
          {count}
        </span>
      </div>
      <div style={row}>
        <input
          id="preact-draft"
          value={draft}
          placeholder="Something to list"
          style={{ ...button, background: "var(--background)", color: "var(--foreground)", flex: "1" }}
          onInput={(e: any) => setDraft(e.currentTarget.value)}
        />
        <button id="preact-add" style={button} onClick={add}>
          Add
        </button>
      </div>
      <ul id="preact-items" style={{ margin: 0, paddingLeft: "20px" }}>
        {items.map((t, i) => (
          <li key={i} class="preact-item">
            {t}
          </li>
        ))}
      </ul>
    </div>
  );
}

render(<Tally />, document.body);
export default document.body;
