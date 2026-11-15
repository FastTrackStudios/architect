//! TypeScript extensions in the web app's host (#495): the extensions
//! host's runner for `kind = "typescript"` in a browser, [`WebTsRunner`].
//!
//! The same host a desk runs (`architect_extensions::host::Host`) loads an
//! installed TypeScript extension; this runner runs each of its actions
//! and hooks in a Web Worker of its own ([`super::WebPages::run_action`]),
//! with the files the host read from the package (a browser's installs are
//! in its storage, `architect_extensions::disk::MemDisk`). Every host call
//! the code makes is the app's host service for that extension
//! ([`WebHost::through`] its `ExtensionApi`), checked against its
//! capabilities as the person granted them: no second host.
//!
//! A runner must be `Send`; a browser's workers are not. The runner keeps
//! only a channel's sender: each run is a message to one task on the
//! page's thread that holds the [`super::WebPages`].

use architect::action::{ActionCaller, ActionError, ActionFuture, Principal};
use architect_extensions::contrib::HookDecl;
use architect_extensions::host::{Context, Instance, Runner};
use architect_extensions::hostapi::ExtensionApi;
use architect_extensions::manifest::{ActionDecl, Kind, Manifest, is_ts_script};
use architect_extensions::package::Package;
use architect_extensions::web::WebHost;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use serde_json::Value as Json;
use wasm_bindgen_futures::spawn_local;

use super::{PageFolder, WebPages};

/// One run: which module and export, as whom, its answer's way back.
struct Job {
    folder: PageFolder,
    file: String,
    export: String,
    manifest: Manifest,
    list: architect::action::ActionList,
    api: ExtensionApi,
    args: Json,
    ctx: Json,
    reply: oneshot::Sender<Result<String, String>>,
}

/// The web app's runner for TypeScript extensions: their actions and
/// hooks in Web Workers, on the browser's own engine.
#[derive(Clone)]
pub struct WebTsRunner {
    jobs: mpsc::UnboundedSender<Job>,
}

impl std::fmt::Debug for WebTsRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebTsRunner")
    }
}

impl WebTsRunner {
    /// Runs on `pages` (the app's: its theme, shared state and limits).
    /// Call it on the page's thread.
    #[must_use]
    pub fn new(pages: WebPages) -> Self {
        let (jobs, mut rx) = mpsc::unbounded::<Job>();
        spawn_local(async move {
            while let Some(job) = rx.next().await {
                let pages = pages.clone();
                spawn_local(async move {
                    let host = WebHost::new(job.manifest, job.list).through(job.api);
                    let mut folder = job.folder;
                    folder.entry = job.file;
                    let out = pages
                        .run_action(&folder, &job.export, &job.args, &job.ctx, host)
                        .await;
                    let _ = job.reply.send(out);
                });
            }
        });
        Self { jobs }
    }
}

impl Runner for WebTsRunner {
    fn kind(&self) -> Kind {
        Kind::TypeScript
    }

    fn start(&self, package: &Package, cx: Context<'_>) -> Result<Box<dyn Instance>, String> {
        let texts = package.texts().ok_or_else(|| {
            "its files are in a folder, which a browser cannot read".to_owned()
        })?;
        let folder = PageFolder {
            entry: String::new(),
            files: texts.into_iter().collect(),
        };
        let manifest = &package.manifest;
        let scripts = manifest
            .actions
            .iter()
            .filter_map(|a| a.script.as_ref())
            .chain(manifest.contributes.hooks.iter().filter_map(|h| h.script.as_ref()))
            .filter(|s| is_ts_script(s));
        for script in scripts {
            if !folder.files.contains_key(script) {
                return Err(format!("{script} is not in the package"));
            }
        }
        Ok(Box::new(TsInstance {
            jobs: self.jobs.clone(),
            folder,
            manifest: manifest.clone(),
            list: cx.list.clone(),
            api: cx.api,
        }))
    }
}

struct TsInstance {
    jobs: mpsc::UnboundedSender<Job>,
    folder: PageFolder,
    manifest: Manifest,
    list: architect::action::ActionList,
    api: ExtensionApi,
}

impl TsInstance {
    /// Runs `file`'s `export` with `args` as `caller` (`what` names it in
    /// `ctx` and errors).
    fn run(
        &self,
        what: String,
        file: String,
        export: String,
        caller: &ActionCaller,
        args: Json,
    ) -> ActionFuture {
        let mut ctx = architect_extensions::caller_json(caller);
        if let Json::Object(map) = &mut ctx {
            map.insert("action".to_owned(), Json::String(what.clone()));
            map.insert(
                "extension".to_owned(),
                Json::String(self.manifest.id.clone()),
            );
        }
        let who: Option<Principal> = caller.principal().cloned();
        let (reply, answer) = oneshot::channel();
        let sent = self.jobs.unbounded_send(Job {
            folder: self.folder.clone(),
            file,
            export,
            manifest: self.manifest.clone(),
            list: self.list.clone(),
            api: self.api.clone().acting_for(who),
            args,
            ctx,
            reply,
        });
        Box::pin(async move {
            let failed = |message: String| ActionError::Failed {
                id: what.clone(),
                message,
            };
            sent.map_err(|_| failed("the browser's runner stopped".to_owned()))?;
            answer
                .await
                .map_err(|_| failed("it was unloaded".to_owned()))?
                .map(String::into_bytes)
                .map_err(failed)
        })
    }
}

impl Instance for TsInstance {
    fn action(
        &self,
        decl: &ActionDecl,
    ) -> Result<architect::action::CallerAsyncHandler, String> {
        let file = decl
            .script
            .clone()
            .filter(|s| is_ts_script(s))
            .ok_or_else(|| format!("{} has no TypeScript module", decl.id))?;
        let me = Self {
            jobs: self.jobs.clone(),
            folder: self.folder.clone(),
            manifest: self.manifest.clone(),
            list: self.list.clone(),
            api: self.api.clone(),
        };
        let id = decl.id.clone();
        Ok(std::sync::Arc::new(move |caller: &ActionCaller, args: &[u8]| {
            let args: Json = if args.iter().all(u8::is_ascii_whitespace) {
                Json::Null
            } else {
                serde_json::from_slice(args).map_err(|e| ActionError::BadArgs {
                    id: id.clone(),
                    message: e.to_string(),
                    expected: None,
                })?
            };
            Ok(me.run(id.clone(), file.clone(), "default".to_owned(), caller, args))
        }))
    }

    fn hook(&self, decl: &HookDecl, caller: &ActionCaller, args: Json) -> Option<ActionFuture> {
        let file = decl.script.clone().filter(|s| is_ts_script(s))?;
        let export = decl.function.clone().unwrap_or_else(|| "default".to_owned());
        Some(self.run(format!("hook {}", decl.on), file, export, caller, args))
    }
}
