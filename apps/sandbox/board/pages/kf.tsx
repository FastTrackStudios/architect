// A ```kf fence in a note, drawn as a chart: the Board's inline embed
// (extension.toml [[embeds]] id = "kf"). The host gives the fence's body
// (useEmbed().source) and draws this page in a sub-document as tall as
// what it renders; editing the fence draws it again, and it grows or
// shrinks with it. A line `[Verse]` starts a section; `| C | G | Am |`
// is bars, each bar's chords split by spaces.
import { h, useEmbed } from "architect";

type Section = { name: string; bars: string[] };

function parse(source: string): Section[] {
  const out: Section[] = [];
  for (const raw of source.split("\n")) {
    const line = raw.trim();
    if (line === "") continue;
    const head = line.match(/^\[(.+)\]$/);
    if (head) {
      out.push({ name: head[1], bars: [] });
      continue;
    }
    if (out.length === 0) out.push({ name: "", bars: [] });
    const bars = line.split("|").map((b) => b.trim()).filter((b) => b !== "");
    out[out.length - 1].bars.push(...bars);
  }
  return out;
}

const bar = {
  minWidth: 64,
  padding: "8px 10px",
  borderRadius: 6,
  border: "1px solid $border",
  background: "$background",
  color: "$foreground",
  fontSize: 14,
  fontWeight: 600,
};

export default function Chart() {
  const embed = useEmbed();
  const sections = parse(embed?.source ?? "");
  const count = sections.reduce((n, s) => n + s.bars.length, 0);
  return (
    <div id="kf-chart" style={{ display: "flex", flexDirection: "column", gap: 10, padding: 14, borderRadius: 10, border: "1px solid $border", background: "$card", color: "$card-foreground" }}>
      <div style={{ display: "flex", gap: 8, alignItems: "baseline" }}>
        <span style={{ fontSize: 12, fontWeight: 700, color: "$primary" }}>♫ KEYFLOW</span>
        <span id="kf-count" style={{ fontSize: 12, color: "$muted-foreground" }}>
          {count} {count === 1 ? "bar" : "bars"} · drawn by lab.board
        </span>
      </div>
      {sections.length === 0 ? (
        <div style={{ color: "$muted-foreground", fontSize: 13 }}>An empty chart: write [Verse] and | C | G | below.</div>
      ) : (
        sections.map((s, i) => (
          <div key={`${i}`} class="kf-section" style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {s.name !== "" && <div style={{ fontSize: 12, fontWeight: 700, color: "$muted-foreground" }}>{s.name}</div>}
            <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
              {s.bars.map((b, j) => (
                <div key={`${j}`} class="kf-bar" style={bar}>{b}</div>
              ))}
            </div>
          </div>
        ))
      )}
    </div>
  );
}
