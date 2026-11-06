// The Board's tab, as a page (docs/content/architecture/pages.md): a TSX
// file on QuickJS in the app, re-rendered live when it is saved, and the
// same file in a Web Worker in a browser (apps/web). Its types are
// <app dir>/architect.d.ts. The Board's data and actions are its Rust
// extension's (apps/sandbox/board/src/lib.rs); this file only draws them.
import { h, useData, useState, actions, notify, worker, theme } from "architect";

type Board = {
  pins: string[];
  chart: string | null;
  issues: string[];
  process: number;
};

const row = { display: "flex", gap: 8, alignItems: "center" };
const button = {
  padding: "5px 12px",
  borderRadius: 6,
  border: "1px solid $border",
  background: "$secondary",
  color: "$secondary-foreground",
};

function Pin({ title }: { title: string }) {
  return (
    <div class="pin" style={{ padding: "8px 12px", borderRadius: 8, border: "1px solid $border", background: "$card" }}>
      📌 {title}
    </div>
  );
}

/** Heavy work goes to a worker: the page keeps answering while it runs. */
function Primes() {
  const [state, setState] = useState("Not counted yet.");
  const [at, setAt] = useState(0);
  const count = () => {
    setState("Counting in a worker…");
    setAt(0);
    const w = worker.spawn("primes.js", { below: 1_000_000 });
    w.onMessage((m: any) => setAt(m.at));
    w.onDone((found, error) =>
      setState(error ? `The worker failed: ${error}` : `${found} primes below 1,000,000.`),
    );
  };
  return (
    <div style={{ ...row, color: "$muted-foreground" }}>
      <button id="primes" style={button} onClick={count}>Count primes</button>
      <span id="primes-state">{state}</span>
      {at > 0 && <span id="primes-at">(at {at.toLocaleString()})</span>}
    </div>
  );
}

export default function BoardPage() {
  const board = useData<Board>("LAB_BOARD_STATE", {}, { every: 500 });
  const [title, setTitle] = useState("");
  const pins = board.value?.pins ?? [];
  const pin = async () => {
    await actions.run("LAB_BOARD_PIN", { title: title || "a page's pin" });
    setTitle("");
    board.refresh();
  };
  const clear = async () => {
    await actions.run("LAB_BOARD_CLEAR");
    notify("The board is clear");
    board.refresh();
  };
  return (
    <div id="board-page" style={{ display: "flex", flexDirection: "column", gap: 10, padding: 24 }}>
      <div id="board-title" style={{ fontSize: 18, fontWeight: 700 }}>Board</div>
      <div style={{ color: "$muted-foreground" }}>
        This tab is a page file (board.tsx), drawn in the {theme.scheme} theme {theme.id}: QuickJS runs it in
        the app, a Web Worker in a browser. Save it and the pane follows; break it and the last good page stays.
      </div>
      <div style={row}>
        <input
          id="pin-title"
          value={title}
          placeholder="What to pin"
          style={{ padding: "5px 8px", borderRadius: 6, border: "1px solid $input", background: "$background", color: "$foreground" }}
          onInput={(e) => setTitle(e.value ?? "")}
        />
        <button id="pin" style={button} onClick={pin}>Pin</button>
        <button id="clear" style={button} onClick={clear}>Clear</button>
      </div>
      {pins.length === 0 ? (
        <div style={{ color: "$muted-foreground" }}>Nothing pinned yet.</div>
      ) : (
        pins.map((p, i) => <Pin key={`${i}-${p}`} title={p} />)
      )}
      {board.value?.chart && <div style={{ color: "$muted-foreground" }}>Last chart: {board.value.chart}</div>}
      <Primes />
    </div>
  );
}
