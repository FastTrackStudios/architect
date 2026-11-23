// A ```showcase-query fence, drawn as a table: the fence's text is the
// query, its answer the vault's own (Dataview-lite's query action).
import { h, useEmbed, useData } from "architect";

type Row = { path: string; name: string; values?: Record<string, unknown> };
type Answer = { kind: string; columns: string[]; rows: Row[] };

const cell = { padding: "4px 10px", borderBottom: "1px solid $border", textAlign: "left" };

/** A list either way: an empty Luau table arrives as {}, not []. */
function list<T>(v: unknown): T[] {
  return Array.isArray(v) ? (v as T[]) : [];
}

function show(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (Array.isArray(v)) return v.map(show).join(", ");
  return String(v);
}

export default function Query() {
  const source = useEmbed()?.source ?? "";
  const answer = useData<Answer>("SHOWCASE_DATAVIEW_QUERY", { source });
  if (answer.error) return <div style={{ color: "$destructive", padding: 8 }}>{answer.error}</div>;
  const a = answer.value;
  if (!a) return <div style={{ color: "$muted-foreground", padding: 8 }}>Querying…</div>;
  const columns = list<string>(a.columns).filter((c) => c !== "file.name");
  const rows = list<Row>(a.rows);
  return (
    <div class="showcase-query" style={{ padding: 4 }}>
      <div style={{ color: "$muted-foreground", fontSize: 12, padding: "2px 10px" }}>
        {a.kind}: {rows.length} {rows.length === 1 ? "note" : "notes"}
      </div>
      <table style={{ borderCollapse: "collapse", width: "100%" }}>
        <thead>
          <tr>
            <th style={cell}>Note</th>
            {columns.map((c) => <th key={c} style={cell}>{c}</th>)}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.path}>
              <td style={cell}>{r.name}</td>
              {columns.map((c) => <td key={c} style={cell}>{show(r.values?.[c])}</td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
