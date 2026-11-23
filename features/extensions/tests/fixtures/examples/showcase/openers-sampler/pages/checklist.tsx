// The Checklist base view: a base's rows as a checklist, a row ticked
// when its `status` is the view's `doneWhen` (done). Bases hands the page
// its rows as an inline embed's props (useEmbed).
import { h, useEmbed } from "architect";

type Row = { path: string; name: string; values?: Record<string, unknown> };
type Input = { view?: { options?: Record<string, unknown> }; rows?: { groups?: { label?: string; rows?: Row[] }[] } };

export default function Checklist() {
  const input = (useEmbed()?.props ?? {}) as Input;
  const doneWhen = String(input.view?.options?.doneWhen ?? "done");
  const groups = input.rows?.groups ?? [];
  const rows = groups.flatMap((g) => g.rows ?? []);
  const done = rows.filter((r) => String(r.values?.status ?? "") === doneWhen).length;
  return (
    <div id="checklist" style={{ display: "flex", flexDirection: "column", gap: 6, padding: 12 }}>
      <div style={{ color: "$muted-foreground" }}>{done} of {rows.length} done</div>
      {rows.map((r) => {
        const on = String(r.values?.status ?? "") === doneWhen;
        return (
          <div key={r.path} class="item" style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <span style={{ width: 16, textAlign: "center", color: on ? "$primary" : "$muted-foreground" }}>{on ? "☑" : "☐"}</span>
            <span style={{ textDecoration: on ? "line-through" : "none", color: on ? "$muted-foreground" : "$foreground" }}>{r.name}</span>
          </div>
        );
      })}
    </div>
  );
}
