//! The extensions' management actions (#181, #239 X5, #178): consent,
//! previews, updates, pins, logs, and the host service as an agent tries
//! it.
//!
//! | action | what it does |
//! |---|---|
//! | `EXT_PREVIEW` | what `{id, version?}` would be and do, read before installing: kind, runner, capabilities (and which need a yes), contributions, hooks, network and files, its ref |
//! | `EXT_INFO` | everything about `{id}`: which copy runs (`built in, overridden by …`), versions, pin, grants, what it may reach now, what it adds |
//! | `EXT_UPDATE` | updates `{id?}` (every installed one) to the newest its pin allows; unchanged ones skipped, grown capabilities wait for a yes; `{check: true}` only says what is available: the newer version, its changelog line, a newer one past the pin, and what it asks past the grant |
//! | `EXT_INSTALL_FILE` | installs a signed `.archext` from `{path}` (a file, or what a peer sent): refused unless signed by a trusted key and unchanged since; consent as `EXT_INSTALL` |
//! | `EXT_PIN` | keeps `{id}` at `version` (a range; its version now when none), or `unpin` |
//! | `EXT_GRANT` | grants `{id}` what it asks, on the `device` or in an `org`: a person's yes (an agent gets a ticket) |
//! | `EXT_REVOKE` | takes `{id}`'s grant back (`scope`, or every one): at once |
//! | `EXT_GRANTS` | every grant, or `{id}`'s, and what it may reach now |
//! | `EXT_LOGS` | `{id, since?, limit?}`: its recent log lines and errors |
//! | `EXT_HOST_CALL` | one host-service call as extension `{id}`, for the caller: what it would be answered (`refused` outside its capabilities) |
//!
//! `EXT_INSTALL` asks for consent first when the app keeps it: an
//! extension that asks for more than was granted is answered
//! `needs_consent` with its preview, not installed; installed again with
//! `grant` (`device`, `org`), the grant and the install are one call. An
//! agent's call with `grant`, and its `EXT_GRANT`, need a person's yes
//! ([`Extensions::confirm_on`]): the agent is answered
//! `confirmation_required` and a ticket at once, never left waiting.

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use facet::Facet;
use semver::VersionReq;
use serde_json::{Value, json};

use crate::consent::Scope;
use crate::extensions::Extensions;
use crate::hostcall::{HostAnswer, HostRequest};
use crate::manage::Installed;

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Extensions",
        method_name: id,
        display_name: name,
        description,
        category: "Extensions",
        group: "Extensions",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static PREVIEW: ActionMeta = meta(
    "EXT_PREVIEW",
    "Preview an extension",
    "What extension `id` (the newest `version` allows) would be and do, read without installing or \
     running it: its kind and what runs it, the capabilities it asks for and which of them need a \
     person's yes, what it adds (actions, views, tabs, status items, menus, openers, slots), its \
     hooks, the network hosts and folders it may reach, where it comes from and its git ref.",
);
static INFO: ActionMeta = meta(
    "EXT_INFO",
    "About an extension",
    "Everything about extension `id`: which copy runs (built in, installed, in development, and \
     what it overrides), the versions the registries list, its pin, its grants, what it may reach \
     now and what it adds.",
);
static UPDATE: ActionMeta = meta(
    "EXT_UPDATE",
    "Update extensions",
    "Updates installed extension `id` (every installed one when none) to the newest version its \
     pin allows. Unchanged ones are skipped; one whose capabilities grow is not updated until a \
     person grants them (listed under needs_consent). With `check` true nothing is installed: the \
     answer lists each update available — the newer version, its changelog line, a newer one \
     past the pin, what it asks past what was granted (asks_more), and cannot_run when there is \
     no build for this platform.",
);
static INSTALL_FILE: ActionMeta = meta(
    "EXT_INSTALL_FILE",
    "Install an extension archive",
    "Installs the signed .archext archive at `path` (a file, or one a peer sent) and loads it. It \
     is refused, with the reason, unless it is signed by a key this app trusts (the org release \
     key, and the registries' own) and unchanged since signing, and has a build for this \
     platform. Consent as EXT_INSTALL: needs_consent with what it asks, `grant` (device, org) to \
     grant it, which a caller from outside needs a person's yes for.",
);
static PIN: ActionMeta = meta(
    "EXT_PIN",
    "Pin an extension",
    "Keeps installed extension `id` at `version` (a semver range like =1.2.0 or ~1.2; its version \
     now when none): EXT_UPDATE takes it no further. `unpin` lets it update again.",
);
static GRANT: ActionMeta = meta(
    "EXT_GRANT",
    "Grant an extension what it asks",
    "Grants extension `id` the capabilities it asks for, on this `device` (the default) or in an \
     organization (`org`: the current one, `org:<id>`). A person's decision: a caller from outside \
     is answered confirmation_required with a ticket for the person to approve.",
);
static REVOKE: ActionMeta = meta(
    "EXT_REVOKE",
    "Revoke an extension's grant",
    "Takes back what extension `id` was granted (`scope`: device, org, org:<id>; every grant when \
     none). It holds at once: its next host-service call, hook or script is refused what it lost.",
);
static GRANTS: ActionMeta = meta(
    "EXT_GRANTS",
    "Extensions' grants",
    "Every grant (or extension `id`'s): what was granted, where, when and by whom; with what each \
     extension may reach now and what it asks that is not granted.",
);
static LOGS: ActionMeta = meta(
    "EXT_LOGS",
    "An extension's logs",
    "Extension `id`'s recent log lines and errors, oldest first: a process's stdout and stderr, a \
     Luau script's print and the error it stopped with, failed hooks, refused host-service calls, \
     its own log lines. `since` (ms since the epoch) and `limit` (100 unless given) narrow them.",
);
static HOST_CALL: ActionMeta = meta(
    "EXT_HOST_CALL",
    "Call the host service as an extension",
    "Makes one host-service call as extension `id`, acting for the caller: `request` is a host \
     request ({\"op\": \"run\", \"action\": …}, {\"op\": \"selection\"}, {\"op\": \"setting\", \
     \"key\": …}, …). Answers what the extension would be answered: {ok} or {error: {kind: \
     refused, capability, why}} outside what it may reach.",
);

/// `EXT_PREVIEW`'s arguments.
#[derive(Debug, Facet)]
pub struct PreviewArgs {
    pub id: String,
    pub version: Option<String>,
}

/// `EXT_INFO`'s arguments.
#[derive(Debug, Facet)]
pub struct InfoArgs {
    pub id: String,
}

/// `EXT_UPDATE`'s arguments.
#[derive(Debug, Facet)]
pub struct UpdateArgs {
    pub id: Option<String>,
    /// Only say what updates are available; install nothing.
    pub check: Option<bool>,
}

/// `EXT_INSTALL_FILE`'s arguments.
#[derive(Debug, Facet)]
pub struct InstallFileArgs {
    /// The `.archext` file.
    pub path: String,
    /// Where a person grants what it asks: `device`, `org`, `org:<id>`.
    pub grant: Option<String>,
}

/// `EXT_PIN`'s arguments.
#[derive(Debug, Facet)]
pub struct PinArgs {
    pub id: String,
    pub version: Option<String>,
    pub unpin: Option<bool>,
}

/// `EXT_GRANT`'s arguments.
#[derive(Debug, Facet)]
pub struct GrantArgs {
    pub id: String,
    /// `device` (the default), `org`, `org:<id>`.
    pub scope: Option<String>,
    /// The version whose capabilities to grant (the loaded one's, or the
    /// newest listed).
    pub version: Option<String>,
}

/// `EXT_REVOKE`'s arguments.
#[derive(Debug, Facet)]
pub struct RevokeArgs {
    pub id: String,
    pub scope: Option<String>,
}

/// `EXT_GRANTS`'s arguments.
#[derive(Debug, Facet)]
pub struct GrantsArgs {
    pub id: Option<String>,
}

/// `EXT_LOGS`'s arguments.
#[derive(Debug, Facet)]
pub struct LogsArgs {
    pub id: String,
    pub since: Option<u64>,
    pub limit: Option<u64>,
}

/// `EXT_HOST_CALL`'s arguments: `request` is any host request, so they
/// are read as JSON (its schema is [`HOST_CALL_SCHEMA`]).
#[derive(Debug, serde::Deserialize)]
struct HostCallArgs {
    id: String,
    request: HostRequest,
}

/// `EXT_HOST_CALL`'s params schema.
const HOST_CALL_SCHEMA: &str = r#"{"type":"object","properties":{"id":{"type":"string","description":"The extension."},"request":{"type":"object","description":"A host request: {\"op\": \"run\", \"action\": …, \"args\"?}, {\"op\": \"org\"}, {\"op\": \"selection\"}, {\"op\": \"open\", \"path\"}, {\"op\": \"read_document\", \"path\"}, {\"op\": \"read_file\", \"path\"}, {\"op\": \"write_file\", \"path\", \"text\"}, {\"op\": \"setting\", \"key\"}, {\"op\": \"set_setting\", \"key\", \"value\"}, {\"op\": \"data\", \"key\"}, {\"op\": \"set_data\", \"key\", \"value\"?}, {\"op\": \"log\", \"text\"}."}},"required":["id","request"]}"#;

fn raw(v: &Value) -> RawJson {
    RawJson::from(v.to_string())
}

fn req(version: Option<&str>) -> Result<VersionReq, String> {
    match version {
        None | Some("") => Ok(VersionReq::STAR),
        Some(r) => VersionReq::parse(r).map_err(|e| format!("version {r}: {e}")),
    }
}

/// Who said yes, for the grant's record.
fn by(caller: &ActionCaller) -> String {
    caller.principal().map_or_else(
        || "person".to_owned(),
        architect::action::Principal::describe,
    )
}

impl Extensions {
    /// The answer of an install, as `EXT_INSTALL` gives it.
    #[must_use]
    pub fn installed_json(installed: &Installed) -> Value {
        match installed {
            Installed::Done {
                id,
                version,
                loaded,
                granted,
            } => json!({ "id": id, "version": version, "loaded": loaded, "granted": granted }),
            Installed::NeedsConsent { preview } => json!({
                "needs_consent": true,
                "id": preview.id,
                "version": preview.version,
                "asks": preview.needs_consent,
                "preview": preview,
                "how": "A person grants it: EXT_INSTALL again with grant (device, org), or \
                        EXT_GRANT then EXT_INSTALL. An agent's grant needs the person's yes \
                        (confirmation_required, a ticket).",
            }),
        }
    }

    /// Everything `EXT_INFO` says of `id`.
    #[must_use]
    pub fn info(&self, id: &str) -> Value {
        let chosen = self.chosen().into_iter().find(|c| c.runs.id == id);
        let loaded = self.host().get(id);
        let installed = self.installs().get(id);
        let versions: Vec<Value> = self
            .versions(id)
            .into_iter()
            .map(|(source, l)| json!({ "version": l.version.to_string(), "registry": source }))
            .collect();
        let grants = self.host().grants();
        let manifest = chosen.as_ref().map(|c| c.runs.package.manifest.clone());
        let about = manifest.as_ref().map(|m| {
            let listing = crate::index::Listing {
                id: m.id.clone(),
                name: m.name.clone(),
                version: m.version.clone(),
                kind: m.kind.name().to_owned(),
                description: m.description.clone(),
                category: String::new(),
                location: crate::index::Location::Path(String::new()),
                platforms: Vec::new(),
                changelog: String::new(),
            };
            self.preview_of(m, &listing, "")
        });
        json!({
            "id": id,
            "known": chosen.is_some() || !versions.is_empty(),
            "runs": chosen.as_ref().map(|c| self.describe(c)),
            "build": self.build_of(id),
            "origin": chosen.as_ref().map(|c| c.runs.origin.name()),
            "overrides": chosen.as_ref().map(|c| c.shadows.iter().map(|(o, v)| json!({ "origin": o.name(), "version": v.to_string() })).collect::<Vec<_>>()),
            "loaded": loaded.is_some(),
            "disabled": self.is_disabled(id),
            "installed": installed.as_ref().map(|i| json!({ "version": i.version.to_string(), "pin": i.pin, "source": i.source, "linked": i.linked })),
            "versions": versions,
            "consent_kept": grants.is_some(),
            "grants": grants.as_ref().map(|g| g.of(id)),
            "may_reach": self.host().capabilities(id),
            "about": about,
        })
    }

    /// Asks a person before `list` runs what grants capabilities for a
    /// caller from outside: `EXT_GRANT`, and `EXT_INSTALL` with `grant`.
    /// Call it on the extensions' own list and on any list their actions
    /// are offered on (the app's).
    pub fn confirm_on(list: &ActionList) {
        list.set_confirm("EXT_GRANT", |_| true);
        let grants = |args: &[u8]| {
            serde_json::from_slice::<Value>(args)
                .ok()
                .and_then(|v| v.get("grant").cloned())
                .is_some_and(|g| !g.is_null())
        };
        list.set_confirm("EXT_INSTALL", grants);
        list.set_confirm("EXT_INSTALL_FILE", grants);
    }

    /// Puts the management actions on the host's list.
    pub(crate) fn install_manage_actions(&self) -> Result<(), ActionError> {
        let list = self.host().list().clone();
        Self::confirm_on(&list);
        let me = self.clone();
        list.register_with(
            &PREVIEW,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: PreviewArgs| {
                let preview = me
                    .preview(&a.id, &req(a.version.as_deref())?)
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(raw(&serde_json::to_value(&preview).unwrap_or(Value::Null)))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &INFO,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: InfoArgs| Ok::<_, String>(raw(&me.info(&a.id))),
        )?;
        let me = self.clone();
        list.register_with(
            &UPDATE,
            Effect::Mutating,
            move |_: &ActionCaller, a: UpdateArgs| {
                if a.check == Some(true) {
                    let available = me.updates(a.id.as_deref());
                    return Ok::<_, String>(raw(&json!({ "available": available })));
                }
                let updated = me.update(a.id.as_deref());
                Ok::<_, String>(raw(&serde_json::to_value(&updated).unwrap_or(Value::Null)))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &INSTALL_FILE,
            Effect::Mutating,
            move |caller: &ActionCaller, a: InstallFileArgs| {
                let org = me.host().grants().and_then(|g| g.org());
                let grant = a
                    .grant
                    .as_deref()
                    .map(|g| Scope::parse(g, org.as_deref()))
                    .transpose()?;
                let bytes = std::fs::read(&a.path).map_err(|e| format!("{}: {e}", a.path))?;
                let installed = me
                    .install_archive(bytes, &a.path, grant, &by(caller))
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(raw(&Self::installed_json(&installed)))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &PIN,
            Effect::Mutating,
            move |_: &ActionCaller, a: PinArgs| {
                let pin = me
                    .pin(&a.id, a.version.as_deref(), a.unpin == Some(true))
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(raw(&json!({ "id": a.id, "pin": pin })))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &GRANT,
            Effect::Mutating,
            move |caller: &ActionCaller, a: GrantArgs| {
                let grants = me
                    .host()
                    .grants()
                    .ok_or("this app keeps no consent: every extension runs with what it asks")?;
                let scope = Scope::parse(
                    a.scope.as_deref().unwrap_or("device"),
                    grants.org().as_deref(),
                )?;
                let (version, caps) = match (me.host().get(&a.id), a.version.as_deref()) {
                    (Some(l), None) => (l.manifest.version.to_string(), l.manifest.capabilities),
                    (_, v) => {
                        let p = me.preview(&a.id, &req(v)?).map_err(|e| e.to_string())?;
                        (p.version, p.capabilities)
                    }
                };
                let grant = grants.grant(&a.id, &version, caps, scope, &by(caller));
                Ok::<_, String>(raw(
                    &json!({ "granted": grant, "may_reach": me.host().capabilities(&a.id) }),
                ))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &REVOKE,
            Effect::Mutating,
            move |_: &ActionCaller, a: RevokeArgs| {
                let grants = me.host().grants().ok_or("this app keeps no consent")?;
                let scope = a
                    .scope
                    .as_deref()
                    .map(|s| Scope::parse(s, grants.org().as_deref()))
                    .transpose()?;
                let revoked = grants.revoke(&a.id, scope.as_ref());
                Ok::<_, String>(raw(&json!({
                    "id": a.id,
                    "revoked": revoked,
                    "may_reach": me.host().capabilities(&a.id),
                })))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &GRANTS,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: GrantsArgs| {
                let Some(grants) = me.host().grants() else {
                    return Ok::<_, String>(raw(&json!({ "consent_kept": false, "grants": [] })));
                };
                let all: Vec<_> = grants
                    .list()
                    .into_iter()
                    .filter(|g| a.id.as_deref().is_none_or(|i| i == g.id))
                    .collect();
                let reach: Vec<Value> = me
                    .host()
                    .loaded()
                    .into_iter()
                    .filter(|l| a.id.as_deref().is_none_or(|i| i == l.manifest.id))
                    .map(|l| {
                        json!({
                            "id": l.manifest.id,
                            "origin": l.origin.name(),
                            "may_reach": me.host().capabilities_of(&l.manifest, l.origin),
                            "not_granted": if l.origin == crate::catalog::Origin::Installed {
                                grants.needs(&l.manifest.id, &l.manifest.capabilities)
                            } else {
                                Vec::new()
                            },
                        })
                    })
                    .collect();
                Ok(raw(&json!({
                    "consent_kept": true,
                    "org": grants.org(),
                    "grants": all,
                    "extensions": reach,
                })))
            },
        )?;
        let me = self.clone();
        list.register_with(
            &LOGS,
            Effect::ReadOnly,
            move |_: &ActionCaller, a: LogsArgs| {
                let limit = a
                    .limit
                    .map_or(100, |l| usize::try_from(l).unwrap_or(usize::MAX));
                let lines = me.host().logs().since(&a.id, a.since, limit);
                Ok::<_, String>(raw(&json!({
                    "id": a.id,
                    "loaded": me.host().get(&a.id).is_some(),
                    "lines": lines,
                })))
            },
        )?;
        let me = self.clone();
        let info = architect::action::ActionInfo {
            id: HOST_CALL.id.to_owned(),
            display_name: HOST_CALL.display_name.to_owned(),
            description: HOST_CALL.description.to_owned(),
            category: HOST_CALL.category.to_owned(),
            group: HOST_CALL.group.to_owned(),
            shortcut: String::new(),
            effect: Effect::Mutating,
            params_schema: Some(HOST_CALL_SCHEMA.to_owned()),
            result_schema: None,
            enabled: true,
            disabled_reason: None,
            checked: None,
        };
        let handler: architect::action::CallerAsyncHandler =
            std::sync::Arc::new(move |caller: &ActionCaller, args: &[u8]| {
                let a: HostCallArgs =
                    serde_json::from_slice(args).map_err(|e| ActionError::BadArgs {
                        id: HOST_CALL.id.to_owned(),
                        message: e.to_string(),
                        expected: Some(HOST_CALL_SCHEMA.to_owned()),
                    })?;
                let api = me.host().api(&a.id).acting_for(caller.principal().cloned());
                let run: architect::action::ActionFuture = Box::pin(async move {
                    let answer = HostAnswer::from(api.call(a.request).await);
                    Ok(serde_json::to_vec(&answer).unwrap_or_default())
                });
                Ok(run)
            });
        list.register_forwarded_with(&info, handler)?;
        Ok(())
    }
}
