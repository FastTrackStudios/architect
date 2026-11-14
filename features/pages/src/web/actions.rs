//! TypeScript actions in a browser (#536): a `typescript` extension's
//! actions, each run in a Web Worker of its own on the browser's engine —
//! the worker a page runs in (`web/page.js`), with the same prelude and
//! the same bridge, its host calls answered by the extension's
//! [`WebHost`] after the checks every host makes. No `QuickJS`, no JSPI:
//! any browser with module workers runs them, Safari included.
//!
//! [`WebPages::run_action`] runs one module's handler and answers its
//! JSON; [`WebPages::register_actions`] puts each of a manifest's
//! TypeScript actions on an Action List, so the web app runs them as any
//! action (⌘K, keys, a page's `actions.run`). Its limits are a page's: each
//! unit of its work within [`Limits::frame`] (past it the worker is
//! terminated, as a browser cannot interrupt one call), loading within
//! [`Limits::load`]; a run whose promise nothing will settle is ended and
//! says so. A browser cannot cap one worker's memory.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use architect::action::{ActionCaller, ActionError, ActionFuture, ActionList};
use architect_extensions::manifest::is_ts_script;
use architect_extensions::web::WebHost;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use serde_json::{Value as Json, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use super::{
    App, Limits, PageFolder, Prepared, from_js, millis, out_of_time, prepare, set_shared, to_js,
};
use crate::api::Notice;
use crate::tsx;

/// One action's run in its worker.
struct Running {
    worker: web_sys::Worker,
    host: WebHost,
    app: std::rc::Weak<App>,
    prepared: Prepared,
    limits: Limits,
    loaded: Cell<bool>,
    timer: Cell<Option<i32>>,
    done: RefCell<Option<oneshot::Sender<Result<String, String>>>>,
    on_time: RefCell<Option<Closure<dyn FnMut()>>>,
    on_message: RefCell<Option<Closure<dyn FnMut(web_sys::MessageEvent)>>>,
}

impl Running {
    fn said(&self, text: &str) -> String {
        let maps: Vec<(&str, &tsx::SourceMap)> = self
            .prepared
            .maps
            .iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        tsx::remap(text, super::ROOT, &maps)
    }

    fn arm(&self, limit: std::time::Duration) {
        self.disarm();
        let Some(f) = self.on_time.borrow().as_ref().map(|c| c.as_ref().clone()) else {
            return;
        };
        let id = web_sys::window().and_then(|w| {
            w.set_timeout_with_callback_and_timeout_and_arguments_0(
                f.unchecked_ref(),
                millis(limit),
            )
            .ok()
        });
        self.timer.set(id);
    }

    fn disarm(&self) {
        if let (Some(id), Some(w)) = (self.timer.take(), web_sys::window()) {
            w.clear_timeout_with_handle(id);
        }
    }

    /// It is over: its worker goes, and its answer (or why) is said.
    fn end(&self, out: Result<String, String>) {
        self.disarm();
        self.worker.terminate();
        if let Some(tx) = self.done.borrow_mut().take() {
            let _ = tx.send(out);
        }
        // Its closures go with it once the last message is handled.
        let on_message = self.on_message.borrow_mut().take();
        let on_time = self.on_time.borrow_mut().take();
        spawn_local(async move {
            drop((on_message, on_time));
        });
    }

    fn message(self: &Rc<Self>, m: &Json) {
        let text = |k: &str| {
            m.get(k)
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match m.get("t").and_then(Json::as_str).unwrap_or_default() {
            "loaded" => {
                self.loaded.set(true);
                self.disarm();
            }
            "busy" => {
                if self.loaded.get() {
                    self.arm(self.limits.frame);
                }
            }
            "idle" => {
                if self.loaded.get() {
                    self.disarm();
                }
            }
            "done" => {
                let ok = m.get("ok").and_then(Json::as_bool).unwrap_or(false);
                let said = text("text");
                if ok {
                    self.end(Ok(said));
                } else {
                    let why = self.said(&said);
                    self.host.log("error", "script", why.clone());
                    self.end(Err(why));
                }
            }
            "failed" => {
                let why = self.said(&text("why"));
                self.host.log("error", "script", why.clone());
                self.end(Err(why));
            }
            "call" => {
                let (call, kind, a, b) = (
                    m.get("id").cloned().unwrap_or(Json::Null),
                    text("kind"),
                    text("a"),
                    text("b"),
                );
                let me = Rc::clone(self);
                spawn_local(async move {
                    let result = super::answer(&me.host, &me.app, &kind, &a, &b).await;
                    let (ok, text) = match result {
                        Ok(t) => (true, t),
                        Err(e) => (false, e),
                    };
                    let _ = me.worker.post_message(&to_js(
                        &json!({ "t": "answer", "id": call, "ok": ok, "text": text }),
                    ));
                });
            }
            "set_shared" => {
                if let Some(app) = self.app.upgrade() {
                    set_shared(&app, &text("key"), &text("value"));
                }
            }
            "notify" => {
                let notice = Notice {
                    text: text("text"),
                    kind: text("kind"),
                };
                let sink = self.app.upgrade().and_then(|a| a.notify.borrow().clone());
                if let Some(sink) = sink {
                    sink(&notice);
                }
            }
            "log" => {
                let level = text("level");
                let said = if level == "error" {
                    self.said(&text("text"))
                } else {
                    text("text")
                };
                self.host.log(&level, "console", said);
            }
            _ => {}
        }
    }
}

impl super::WebPages {
    /// Runs the TypeScript action whose module is `folder.entry`: its
    /// export `export` (`default`) called with `args` and `ctx` in a Web
    /// Worker of its own, as `host`'s extension. Its answer, as JSON; or
    /// why it failed, at its source's line and column.
    ///
    /// # Errors
    ///
    /// Its module did not load, it threw or rejected, it ran past its
    /// time, or it waits on nothing that will answer.
    pub async fn run_action(
        &self,
        folder: &PageFolder,
        export: &str,
        args: &Json,
        ctx: &Json,
        host: WebHost,
    ) -> Result<String, String> {
        let app = &self.inner;
        let options = web_sys::WorkerOptions::new();
        options.set_type(web_sys::WorkerType::Module);
        let worker = web_sys::Worker::new_with_options(&super::script(app)?, &options)
            .map_err(|e| format!("its Web Worker did not start: {e:?}"))?;
        let limits = app.limits.get();
        let (tx, rx) = oneshot::channel();
        let running = Rc::new(Running {
            worker,
            host,
            app: Rc::downgrade(app),
            prepared: prepare(folder),
            limits,
            loaded: Cell::new(false),
            timer: Cell::new(None),
            done: RefCell::new(Some(tx)),
            on_time: RefCell::new(None),
            on_message: RefCell::new(None),
        });
        let weak = Rc::downgrade(&running);
        *running.on_message.borrow_mut() = Some(Closure::new(move |e: web_sys::MessageEvent| {
            if let Some(r) = weak.upgrade() {
                r.message(&from_js(&e.data()));
            }
        }));
        let weak = Rc::downgrade(&running);
        *running.on_time.borrow_mut() = Some(Closure::new(move || {
            if let Some(r) = weak.upgrade() {
                let limit = if r.loaded.get() {
                    r.limits.frame
                } else {
                    r.limits.load
                };
                r.host.log("error", "script", out_of_time(limit));
                r.end(Err(out_of_time(limit)));
            }
        }));
        if let Some(f) = running.on_message.borrow().as_ref() {
            running
                .worker
                .set_onmessage(Some(f.as_ref().unchecked_ref()));
        }
        let mut start = running.prepared.start.clone();
        let shared: Vec<Json> = app
            .shared
            .borrow()
            .iter()
            .map(|(k, v)| json!([k, v]))
            .collect();
        if let Some(fields) = start.as_object_mut() {
            fields.insert("shared".to_owned(), Json::Array(shared));
            fields.insert("embed".to_owned(), json!("null"));
            fields.insert(
                "theme".to_owned(),
                json!(serde_json::to_string(&*app.theme.borrow()).unwrap_or_default()),
            );
            fields.insert(
                "workerTime".to_owned(),
                json!(limits.worker_time.as_millis()),
            );
            fields.insert(
                "action".to_owned(),
                json!({ "export": export, "args": args.to_string(), "ctx": ctx.to_string() }),
            );
        }
        running.arm(limits.load);
        let _ = running.worker.post_message(&to_js(&start));
        let out = rx
            .await
            .unwrap_or_else(|_| Err("its run stopped before it answered".to_owned()));
        drop(running);
        out
    }

    /// Puts each TypeScript action of `host`'s extension on `list`: run,
    /// each in a Web Worker of its own, from `files` (its folder, as text:
    /// every file by its `/`-separated path). Actions with no `.ts`/`.js`
    /// script are left to their own runner.
    ///
    /// # Errors
    ///
    /// An action id already on the list.
    pub fn register_actions(
        &self,
        list: &ActionList,
        files: &PageFolder,
        host: &WebHost,
    ) -> Result<Vec<String>, ActionError> {
        struct Run {
            action: String,
            file: String,
            args: Json,
            ctx: Json,
            reply: oneshot::Sender<Result<String, String>>,
        }
        let (tx, mut rx) = mpsc::unbounded::<Run>();
        let (pages, folder, runs_as) = (self.clone(), files.clone(), host.clone());
        spawn_local(async move {
            while let Some(run) = rx.next().await {
                let (pages, mut folder, host) = (pages.clone(), folder.clone(), runs_as.clone());
                spawn_local(async move {
                    folder.entry = run.file;
                    let mut ctx = run.ctx;
                    if let Json::Object(map) = &mut ctx {
                        map.insert("action".to_owned(), Json::String(run.action));
                        map.insert(
                            "extension".to_owned(),
                            Json::String(host.manifest().id.clone()),
                        );
                    }
                    let out = pages
                        .run_action(&folder, "default", &run.args, &ctx, host)
                        .await;
                    let _ = run.reply.send(out);
                });
            }
        });
        let manifest = host.manifest().clone();
        let mut added = Vec::new();
        for decl in &manifest.actions {
            let Some(file) = decl.script.clone().filter(|s| is_ts_script(s)) else {
                continue;
            };
            let (tx, id) = (tx.clone(), decl.id.clone());
            let info = architect::action::ActionInfo {
                id: decl.id.clone(),
                display_name: decl.name.clone(),
                description: decl.description.clone(),
                category: manifest.name.clone(),
                group: decl.group.clone(),
                shortcut: decl.shortcut.clone(),
                effect: match decl.effect {
                    architect_extensions::manifest::Reach::ReadOnly => {
                        architect::action::Effect::ReadOnly
                    }
                    architect_extensions::manifest::Reach::Mutating => {
                        architect::action::Effect::Mutating
                    }
                    architect_extensions::manifest::Reach::Destructive => {
                        architect::action::Effect::Destructive
                    }
                },
                params_schema: decl.params.clone(),
                result_schema: None,
                enabled: true,
                disabled_reason: None,
                checked: None,
            };
            let handler = std::sync::Arc::new(move |caller: &ActionCaller, args: &[u8]| {
                let args: Json = if args.iter().all(u8::is_ascii_whitespace) {
                    Json::Null
                } else {
                    serde_json::from_slice(args).map_err(|e| ActionError::BadArgs {
                        id: id.clone(),
                        message: e.to_string(),
                        expected: None,
                    })?
                };
                let (reply, answer) = oneshot::channel();
                let sent = tx.unbounded_send(Run {
                    action: id.clone(),
                    file: file.clone(),
                    args,
                    ctx: architect_extensions::caller_json(caller),
                    reply,
                });
                let id = id.clone();
                let run: ActionFuture = Box::pin(async move {
                    let failed = |message: String| ActionError::Failed {
                        id: id.clone(),
                        message,
                    };
                    sent.map_err(|_| failed("it was unloaded".to_owned()))?;
                    answer
                        .await
                        .map_err(|_| failed("it was unloaded".to_owned()))?
                        .map(String::into_bytes)
                        .map_err(failed)
                });
                Ok(run)
            });
            list.register_forwarded_with(&info, handler)?;
            added.push(decl.id.clone());
        }
        Ok(added)
    }
}
