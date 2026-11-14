// Templater-lite's tab: the templates in Templates/, each with the
// fields it asks for and a button that makes a note from it. A TSX page
// on QuickJS in the app (a Web Worker in a browser); the work is its
// Luau actions'.
import { h, useData, actions, notify } from "architect";

type Template = { path: string; title: string; fields: string[] };

const button = {
  padding: "4px 10px",
  borderRadius: 6,
  border: "1px solid $border",
  background: "$secondary",
  color: "$secondary-foreground",
};

export default function Templates() {
  const list = useData<Template[]>("SHOWCASE_TEMPLATER_LIST", {});
  const make = async (t: Template) => {
    const path = await actions.run("SHOWCASE_TEMPLATER_NEW", { template: t.path });
    if (path) notify(`Made ${path}`);
  };
  const templates = list.value ?? [];
  return (
    <div id="templates" style={{ display: "flex", flexDirection: "column", gap: 10, padding: 20 }}>
      <div style={{ fontSize: 18, fontWeight: 700 }}>Templates</div>
      <div style={{ color: "$muted-foreground" }}>
        Notes in Templates/. Each makes a note in Inbox/, asking for its fields.
      </div>
      {templates.length === 0 ? (
        <div style={{ color: "$muted-foreground" }}>No templates yet.</div>
      ) : (
        templates.map((t) => (
          <div
            key={t.path}
            class="template"
            style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap", padding: "8px 12px", borderRadius: 8, border: "1px solid $border", background: "$card" }}
          >
            <span style={{ fontWeight: 600 }}>{t.title}</span>
            <span style={{ color: "$muted-foreground" }}>{t.fields.join(", ")}</span>
            <button style={button} onClick={() => make(t)}>New</button>
          </div>
        ))
      )}
    </div>
  );
}
