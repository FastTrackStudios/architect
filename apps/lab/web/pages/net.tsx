// A page's reach in a browser (apps/lab/web), as features/pages/tests/net.rs
// checks it natively: its extension (`lab.web-checks`) declares
// `capabilities.hosts = ["127.0.0.1"]`, so `net.fetch` reaches the lab's
// own server there and is refused `localhost`; the browser's own network
// is not in its worker at all; and the lab's Content-Security-Policy,
// which its worker inherits, refuses an `import()` of another host and
// eval.
import { h, useState, useEffect, net, state, actions } from "architect";

export default function Net() {
  const [out, setOut] = useState("");
  const [ambient, setAmbient] = useState("");
  const [run, setRun] = useState("");
  const [loads, setLoads] = useState("");
  useEffect(() => {
    const port = state.get("lab.port");
    const ours = `http://127.0.0.1:${port}/hello.txt`;
    const other = `http://localhost:${port}/hello.txt`;
    net.fetch(ours).then(
      (r: any) =>
        net.fetch(other).then(
          () => setOut(String(r.body).trim() + "|reached"),
          (e: any) => setOut(String(r.body).trim() + "|" + String(e.message)),
        ),
      (e: any) => setOut("failed|" + String(e.message)),
    );
    const g: any = globalThis;
    setAmbient(
      ["fetch", "XMLHttpRequest", "WebSocket", "importScripts", "indexedDB", "Worker", "postMessage"]
        .map((k) => `${k}:${typeof g[k]}`)
        .join(" "),
    );
    let evals = "eval:ran";
    try {
      new Function("return 1")();
    } catch (e: any) {
      evals = "eval:" + String(e.name);
    }
    const away = "http://localhost:" + port + "/start.js";
    import(away).then(
      () => setLoads("import:ran " + evals),
      (e: any) => setLoads("import:" + String(e.name) + " " + evals),
    );
    actions.run("LAB_BOARD_STATE").then(
      () => setRun("reached"),
      (e: any) => setRun(String(e.message)),
    );
  }, []);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: 24, fontFamily: "ui-monospace, monospace" }}>
      <div style={{ fontSize: 18, fontWeight: 700, fontFamily: "system-ui" }}>A page's reach</div>
      <div>net.fetch: <span id="out">{out}</span></div>
      <div>actions.run another extension's: <span id="run">{run}</span></div>
      <div>the worker's own: <span id="ambient">{ambient}</span></div>
      <div>another host's code: <span id="loads">{loads}</span></div>
    </div>
  );
}
