//! TypeScript actions (#536): a `typescript` extension's actions and
//! hooks as `.ts` modules on the pages' `QuickJS` runtime, through the
//! extensions host as any extension's actions are — its answer typed
//! JSON, its errors at its file's own line and column, its capabilities
//! checked, its limits kept, one undo step per run.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostui::FakeUi;
use architect_extensions::install::Installs;
use architect_pages::host::AppHost;
use architect_pages::js::action::{ActionLimits, TsRunner};
use architect_scripting::Transactions;
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

#[path = "../../extensions/tests/support/server.rs"]
mod server;

const ID: &str = "lab.recipes";

const MANIFEST: &str = r#"
id = "lab.recipes"
name = "Recipes"
version = "0.1.0"
kind = "typescript"
api = "0.1"

[capabilities]
actions = ["LAB_PANTRY_"]
effect = "mutating"
ui = ["notify", "prompts"]
hosts = ["127.0.0.1"]
events = ["recipes.*"]

[[actions]]
id = "LAB_RECIPES_SCALE"
name = "Scale"
params = '{"type":"object","properties":{"servings":{"type":"number"},"items":{"type":"array"}}}'
script = "actions/scale.ts"

[[actions]]
id = "LAB_RECIPES_FAIL"
name = "Fail"
script = "actions/fail.ts"

[[actions]]
id = "LAB_RECIPES_REACH"
name = "Reach"
script = "actions/reach.ts"

[[actions]]
id = "LAB_RECIPES_ASK"
name = "Ask"
effect = "mutating"
script = "actions/ask.ts"

[[actions]]
id = "LAB_RECIPES_FETCH"
name = "Fetch"
params = '{"type":"object","properties":{"url":{"type":"string"}}}'
script = "actions/fetch.js"

[[actions]]
id = "LAB_RECIPES_SPIN"
name = "Spin"
script = "actions/spin.ts"

[[actions]]
id = "LAB_RECIPES_HANG"
name = "Hang"
script = "actions/hang.ts"

[[actions]]
id = "LAB_RECIPES_HOARD"
name = "Hoard"
script = "actions/hoard.ts"

[[actions]]
id = "LAB_RECIPES_BOUND"
name = "Bound"
params = '{"type":"object","properties":{"n":{"type":"number"}}}'
script = "actions/bound.ts"

[[actions]]
id = "LAB_RECIPES_MISBOUND"
name = "Misbound"
script = "actions/bound.ts"

[[actions]]
id = "LAB_RECIPES_TIMER"
name = "Timer"
script = "actions/timer.ts"

[[hooks]]
on = "app:recipes.cooked"
script = "hooks/cooked.ts"
"#;

const SCALE: &str = r#"import { defineAction, type ActionContext } from "architect";
import { scale, type Item } from "../lib/scale";

type Args = { servings?: number; items?: Item[] };

export default defineAction<Args>(async (args: Args, ctx: ActionContext) => {
  const factor = (args.servings ?? 4) / 2;
  console.log("scaling by", factor);
  return { factor, items: scale(args.items ?? [], factor), action: ctx.action, extension: ctx.extension, source: ctx.source };
});
"#;

const LIB: &str = r"export interface Item { name: string; qty: number; unit?: string }
export function scale(items: Item[], factor: number): Item[] {
  return items.map((i) => ({ ...i, qty: Math.round(i.qty * factor * 100) / 100 }));
}
";

const FAIL: &str = r#"export default async function fail(): Promise<number> {
  const n: number = 1;
  throw new Error("no recipe here");
}
"#;

const REACH: &str = r#"import { actions } from "architect";
export default async () => {
  const mine = await actions.run("LAB_PANTRY_ADD", { name: "flour" });
  try {
    await actions.run("LAB_OTHER_SECRET", {});
    return { mine, other: "ran" };
  } catch (e) {
    return { mine, other: String((e as Error).message) };
  }
};
"#;

const ASK: &str = r#"import { ui, notify } from "architect";
export default async () => {
  const name = await ui.input("Recipe name", { placeholder: "Pancakes" });
  await ui.notify("made " + name, "success");
  return name;
};
"#;

const FETCH: &str = r#"import { net } from "architect";
export default async ({ url }) => {
  const r = await net.fetch(url);
  return r.body;
};
"#;

const SPIN: &str = "export default () => { for (;;) {} };\n";

const HANG: &str = "export default () => new Promise(() => {});\n";

const HOARD: &str = r"export default () => {
  const kept: string[] = [];
  for (let i = 0; ; i++) kept.push(String(i).repeat(1 << 20));
};
";

const BOUND: &str = r#"import { defineAction } from "architect";
export default defineAction("LAB_RECIPES_BOUND", (args: { n: number }) => args.n * 2);
"#;

const TIMER: &str = r#"export default () => new Promise((done) => setTimeout(() => done("rang"), 30));
"#;

const COOKED: &str = r#"import { actions, type ActionContext } from "architect";
export default async (detail: unknown, ctx: ActionContext) => {
  await actions.run("LAB_PANTRY_ADD", { name: "from " + ctx.event });
};
"#;

/// The pantry: an app action the extension may run, keeping what it was
/// given.
#[derive(Clone, Default)]
struct Pantry(Arc<Mutex<Vec<String>>>);

const fn meta(id: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: id,
        display_name: id,
        description: "",
        category: "Lab",
        group: "Lab",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static ADD: ActionMeta = meta("LAB_PANTRY_ADD");
static SECRET: ActionMeta = meta("LAB_OTHER_SECRET");

#[derive(facet::Facet)]
struct AddArgs {
    name: String,
}

#[derive(facet::Facet)]
struct NoArgs {}

impl Pantry {
    fn install(&self, list: &ActionList) {
        let me = self.clone();
        list.register_with(
            &ADD,
            Effect::Mutating,
            move |_: &ActionCaller, a: AddArgs| {
                me.0.lock().unwrap().push(a.name.clone());
                Ok::<_, String>(RawJson::from(json!({ "added": a.name }).to_string()))
            },
        )
        .unwrap();
        list.register_with(&SECRET, Effect::ReadOnly, |_: &ActionCaller, _: NoArgs| {
            Ok::<_, String>(RawJson::from("\"leaked\"".to_owned()))
        })
        .unwrap();
    }

    fn items(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// Undo steps opened, committed and taken back.
#[derive(Default)]
struct Steps {
    begun: Mutex<Vec<String>>,
    committed: AtomicUsize,
    aborted: AtomicUsize,
}

impl Transactions for Steps {
    fn begin(&self, name: &str) -> bool {
        self.begun.lock().unwrap().push(name.to_owned());
        true
    }

    fn commit(&self) -> Option<String> {
        self.committed.fetch_add(1, Ordering::Relaxed);
        None
    }

    fn abort(&self) -> Option<String> {
        self.aborted.fetch_add(1, Ordering::Relaxed);
        None
    }
}

struct Lab {
    list: ActionList,
    ext: Extensions,
    ui: FakeUi,
    pantry: Pantry,
    steps: Arc<Steps>,
    _tmp: tempfile::TempDir,
}

fn write(dir: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn package(dir: &Path) {
    write(
        dir,
        &[
            ("extension.toml", MANIFEST),
            ("actions/scale.ts", SCALE),
            ("lib/scale.ts", LIB),
            ("actions/fail.ts", FAIL),
            ("actions/reach.ts", REACH),
            ("actions/ask.ts", ASK),
            ("actions/fetch.js", FETCH),
            ("actions/spin.ts", SPIN),
            ("actions/hang.ts", HANG),
            ("actions/timer.ts", TIMER),
            ("actions/hoard.ts", HOARD),
            ("actions/bound.ts", BOUND),
            ("hooks/cooked.ts", COOKED),
        ],
    );
}

fn lab_with(ui: FakeUi, edit: impl FnOnce(&Path)) -> Lab {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    // The app's own actions are on its list, not the extensions host's
    // (as `VAULT_*` are in the app): reached through the host service.
    let app = ActionList::new();
    let pantry = Pantry::default();
    pantry.install(&app);
    let steps = Arc::new(Steps::default());
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_ui(ui.clone());
    host.add_runner(
        TsRunner::new(AppHost::new(list.clone()))
            .with_transactions({
                let steps: Arc<dyn Transactions> = steps.clone();
                steps
            })
            .with_limits(ActionLimits {
                unit: Duration::from_millis(300),
                ..ActionLimits::default()
            }),
    );
    host.set_app_list(app);
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    // Its host-service calls (`EXT_PAGE_CALL`).
    ext.install_contribution_actions().unwrap();
    let dir = tmp.path().join("dev/recipes");
    package(&dir);
    edit(&dir);
    let _ = ext.dev(&dir);
    Lab {
        list,
        ext,
        ui,
        pantry,
        steps,
        _tmp: tmp,
    }
}

fn lab() -> Lab {
    let lab = lab_with(FakeUi::new(), |_| {});
    assert!(lab.ext.host().get(ID).is_some(), "{:?}", lab.ext.problems());
    lab
}

fn run(lab: &Lab, id: &str, args: &Value) -> Result<Value, ActionError> {
    block_on(
        lab.list
            .run_json_async_with(&ActionCaller::app(), id, args.to_string().as_bytes()),
    )
    .map(|out| serde_json::from_slice(&out).unwrap())
}

fn failure(lab: &Lab, id: &str) -> String {
    match run(lab, id, &json!({})) {
        Err(ActionError::Failed { message, .. }) => message,
        other => panic!("{id} did not fail: {other:?}"),
    }
}

#[test]
fn a_ts_action_answers_typed_json_with_its_imports_and_its_context() {
    let lab = lab();
    let out = run(
        &lab,
        "LAB_RECIPES_SCALE",
        &json!({ "servings": 6, "items": [{ "name": "flour", "qty": 1.5, "unit": "cup" }] }),
    )
    .unwrap();
    assert_eq!(out["factor"], json!(3));
    assert_eq!(
        out["items"],
        json!([{ "name": "flour", "qty": 4.5, "unit": "cup" }])
    );
    assert_eq!(out["action"], "LAB_RECIPES_SCALE");
    assert_eq!(out["extension"], ID);
    // Its console goes to its extension's log.
    let lines = lab.ext.host().logs().since(ID, None, 50);
    assert!(
        lines.iter().any(|l| l.text.contains("scaling by 3")),
        "{lines:?}"
    );
    // One undo step, named for it, committed.
    assert_eq!(
        lab.steps.begun.lock().unwrap().as_slice(),
        ["Recipes: Scale"]
    );
    assert_eq!(lab.steps.committed.load(Ordering::Relaxed), 1);
}

#[test]
fn what_it_throws_says_its_own_line_and_column_and_undoes_its_step() {
    let lab = lab();
    let why = failure(&lab, "LAB_RECIPES_FAIL");
    assert!(why.contains("no recipe here"), "{why}");
    assert!(why.contains("actions/fail.ts:3:"), "{why}");
    assert_eq!(lab.steps.aborted.load(Ordering::Relaxed), 1);
}

#[test]
fn a_module_bound_to_its_id_runs_as_that_action_only() {
    let lab = lab();
    assert_eq!(
        run(&lab, "LAB_RECIPES_BOUND", &json!({ "n": 21 })).unwrap(),
        json!(42)
    );
    let why = failure(&lab, "LAB_RECIPES_MISBOUND");
    assert!(
        why.contains("defineAction(\"LAB_RECIPES_BOUND\"") && why.contains("LAB_RECIPES_MISBOUND"),
        "{why}"
    );
}

#[test]
fn it_runs_what_its_capabilities_reach_and_is_refused_the_rest() {
    let lab = lab();
    let out = run(&lab, "LAB_RECIPES_REACH", &json!({})).unwrap();
    assert_eq!(out["mine"], json!({ "added": "flour" }));
    let other = out["other"].as_str().unwrap();
    assert!(other.contains("not in a namespace"), "{other}");
    assert_eq!(lab.pantry.items(), ["flour"]);
}

#[test]
fn it_asks_the_person_and_notifies_through_the_host_service() {
    let ui = FakeUi::new().answer(json!("Waffles"));
    let lab = lab_with(ui, |_| {});
    let out = run(&lab, "LAB_RECIPES_ASK", &json!({})).unwrap();
    assert_eq!(out, json!("Waffles"));
    let notices = lab.ui.notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].text, "made Waffles");
    assert_eq!(notices[0].from.id, ID);
}

#[test]
fn a_js_action_fetches_from_its_host_and_is_refused_another() {
    let server = server::serve();
    let lab = lab();
    let base = format!("http://127.0.0.1:{}/hello", server.port);
    let out = run(&lab, "LAB_RECIPES_FETCH", &json!({ "url": base })).unwrap();
    assert_eq!(out, json!("hello from the server"));
    let other = format!("http://localhost:{}/hello", server.port);
    let why = match run(&lab, "LAB_RECIPES_FETCH", &json!({ "url": other })) {
        Err(ActionError::Failed { message, .. }) => message,
        other => panic!("reached another host: {other:?}"),
    };
    assert!(why.contains("hosts"), "{why}");
}

#[test]
fn its_limits_stop_a_spin_a_hoard_and_a_promise_nothing_settles() {
    let lab = lab();
    let spin = failure(&lab, "LAB_RECIPES_SPIN");
    assert!(spin.contains("ran past its time"), "{spin}");
    let hoard = failure(&lab, "LAB_RECIPES_HOARD");
    assert!(hoard.contains("out of memory"), "{hoard}");
    let hang = failure(&lab, "LAB_RECIPES_HANG");
    assert!(hang.contains("never settles"), "{hang}");
    // A timer is something that answers.
    assert_eq!(
        run(&lab, "LAB_RECIPES_TIMER", &json!({})).unwrap(),
        json!("rang")
    );
}

#[test]
fn a_hook_runs_its_module_as_its_extension() {
    let lab = lab();
    assert_eq!(lab.ext.host().emit("app:recipes.cooked", "pancakes"), 1);
    let started = std::time::Instant::now();
    while lab.pantry.items().is_empty() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the hook did not run: {:?}",
            lab.ext.host().logs().since(ID, None, 50)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let items = lab.pantry.items();
    assert_eq!(items.len(), 1);
    assert!(
        items[0].starts_with("from ") && items[0].contains("recipes.cooked"),
        "{items:?}"
    );
}

#[test]
fn a_mistake_in_a_module_fails_the_load_at_its_place() {
    let lab = lab_with(FakeUi::new(), |dir| {
        write(
            dir,
            &[("lib/scale.ts", "export function scale(: number {\n")],
        );
    });
    assert!(lab.ext.host().get(ID).is_none());
    let problems = format!("{:?}", lab.ext.problems());
    assert!(problems.contains("lib/scale.ts:1:"), "{problems}");
}

#[test]
fn ext_check_loads_it_with_the_apps_typescript_runner() {
    let lab = lab();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("recipes");
    package(&dir);
    let report = lab.ext.host().checker().check(&dir);
    assert!(report.ok, "{:?}", report.problems);
    assert_eq!(report.kind.as_deref(), Some("typescript"));
    write(&dir, &[("actions/fail.ts", "export default () => {\n")]);
    let report = lab.ext.host().checker().check(&dir);
    assert!(!report.ok);
    let said = format!("{:?}", report.problems);
    assert!(said.contains("actions/fail.ts:"), "{said}");
}
