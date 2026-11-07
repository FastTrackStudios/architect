//! Where files live (#347): a file's copies, the storage health of an
//! org or a root, and search over the registry, offline.
//!
//! - [`Copies`]: on a file's and an asset's page, each holder of the
//!   bytes, its site, its state, how sure we are and since when, and
//!   whether it is reachable now (`FILES_LOCATIONS`).
//! - the `health` view: copy counts, bytes under policy and on one
//!   holder, each holder's share, the files under policy; the policy
//!   set there, and a scrub now (`FILES_COPIES`, `FILES_POLICY_SET`,
//!   `FILES_SCRUB`).
//! - the `find` view: the registry (`FILES_SEARCH`), each hit with where
//!   its bytes are and whether one is reachable now.

use dioxus::prelude::*;
use serde_json::{Value, json};

use super::{
    BUTTON, CARD, HEADING, MUTED, PAGE, ROW, Target, bump, run, set_current, size, text, use_read,
};

const INPUT: &str = "height: 28px; padding: 0 10px; border-radius: 7px; border: 1px solid var(--border); background: var(--background); color: var(--foreground);";

fn now_us() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_micros()).ok())
        .unwrap_or(0)
}

/// `3 h ago`, `2 days ago`.
#[must_use]
pub fn ago(at_us: i64, now_us: i64) -> String {
    let secs = now_us.saturating_sub(at_us) / 1_000_000;
    match secs {
        s if s < 60 => format!("{}s ago", s.max(0)),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} days ago", s / 86_400),
    }
}

/// One copy's line: `verified 3 h ago by scrub`, `unverified since 2 days ago`.
#[must_use]
pub fn verdict(copy: &Value, now: i64) -> String {
    let at = copy
        .get("verified_at_us")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let how = text(copy, "how");
    match text(copy, "confidence") {
        "Verified" => format!("verified {} by {how}", ago(at, now)),
        "Stale" => format!("unverified since {}", ago(at, now)),
        "Failed" => format!("failed: {how}"),
        _ if at > 0 => format!("not counted ({how}), last verified {}", ago(at, now)),
        _ => format!("not counted ({how})"),
    }
}

fn colour(confidence: &str) -> &'static str {
    match confidence {
        "Verified" => "var(--success, #2e9d5b)",
        "Stale" => "var(--warning, #c98a1b)",
        "Failed" => "var(--destructive)",
        _ => "var(--muted-foreground)",
    }
}

/// `2 verified copies at 2 sites`, and what the policy lacks.
#[must_use]
pub fn headline(loc: &Value) -> String {
    let v = loc.get("verified").and_then(Value::as_u64).unwrap_or(0);
    let s = loc.get("sites").and_then(Value::as_u64).unwrap_or(0);
    let copies = if v == 1 { "copy" } else { "copies" };
    let sites = if s == 1 { "site" } else { "sites" };
    let line = format!("{v} verified {copies} at {s} {sites}");
    loc.get("lacks")
        .and_then(Value::as_str)
        .map_or_else(|| line.clone(), |l| format!("{line} · under policy: {l}"))
}

/// A file's copies (`root` and `path`, or `root` empty and `path` a hash).
#[component]
pub fn Copies(root: String, path: String) -> Element {
    let loc = use_read("FILES_LOCATIONS", &json!({ "root": root, "path": path }));
    let now = now_us();
    let body = match loc() {
        None => rsx! { span { style: "{MUTED}", "Looking…" } },
        Some(Err(e)) => rsx! { span { style: "{MUTED}", "{e}" } },
        Some(Ok(loc)) => {
            let copies = loc
                .get("copies")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let line = headline(&loc);
            let under = loc.get("lacks").is_some_and(|l| !l.is_null());
            rsx! {
                span { style: if under { "color: var(--destructive); font-weight: 600;" } else { "font-weight: 600;" }, "data-files": "copy-count", "{line}" }
                if copies.is_empty() { span { style: "{MUTED}", "No holder attests these bytes." } }
                for c in copies {
                    CopyRow { key: "{text(c.get(\"holder\").unwrap_or(&Value::Null), \"id\")}", copy: c, now }
                }
            }
        }
    };
    rsx! {
        div { style: "{CARD}", "data-files": "copies",
            span { style: "{HEADING}", "Copies" }
            {body}
        }
    }
}

#[component]
fn CopyRow(copy: Value, now: i64) -> Element {
    let holder = copy.get("holder").cloned().unwrap_or(Value::Null);
    let name = text(&holder, "name").to_owned();
    let kind = text(&holder, "kind").to_lowercase();
    let site = text(&holder, "site").to_owned();
    let state = text(&copy, "state").to_lowercase();
    let confidence = text(&copy, "confidence").to_owned();
    let here = copy.get("here").and_then(Value::as_bool).unwrap_or(false);
    let reachable = copy
        .get("reachable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let line = verdict(&copy, now);
    let dot = colour(&confidence);
    let reach = if here {
        "this device"
    } else if reachable {
        "reachable now"
    } else {
        "offline"
    };
    rsx! {
        div { style: "display: flex; gap: 8px; align-items: baseline;", "data-files-copy": "{name}",
            span { style: "color: {dot};", "●" }
            div { style: "flex: 1; display: flex; flex-direction: column;",
                span { span { style: "font-weight: 600;", "{name}" } span { style: "{MUTED}", " · {kind} · {site} · {state}" } }
                span { style: "{MUTED} font-size: 12px;", "{line}" }
            }
            span { style: "{MUTED} font-size: 11px;", "{reach}" }
        }
    }
}

/// The storage health of an org or a root.
#[allow(non_snake_case)]
pub fn HealthView() -> Element {
    let roots = use_read("FILES_ROOTS", &json!({}));
    let mut orgs: Vec<String> = roots()
        .and_then(Result::ok)
        .and_then(|r| r.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|r| text(r, "org").to_owned())
        .collect();
    orgs.sort();
    orgs.dedup();
    let current = super::current().map(|t| t.root).filter(|r| !r.is_empty());
    let scope = current
        .or_else(|| orgs.first().cloned())
        .unwrap_or_default();
    rsx! { Health { key: "{scope}", scope, orgs } }
}

#[component]
fn Health(scope: String, orgs: Vec<String>) -> Element {
    let report = use_read("FILES_COPIES", &json!({ "scope": scope }));
    let mut problem = use_signal(|| None::<String>);
    let mut scrub = use_signal(|| None::<String>);
    let mut min_verified = use_signal(|| None::<String>);
    let mut min_sites = use_signal(|| None::<String>);
    let Some(report) = report() else {
        return rsx! { div { style: "{PAGE} {MUTED}", "Counting copies of {scope}…" } };
    };
    let report = match report {
        Ok(r) => r,
        Err(e) => {
            return rsx! { div { style: "{PAGE} color: var(--destructive);", "The files agent: {e}" } };
        }
    };
    let n = |k: &str| report.get(k).and_then(Value::as_u64).unwrap_or(0);
    let policy = report.get("policy").cloned().unwrap_or(Value::Null);
    let pv = policy
        .get("min_verified")
        .and_then(Value::as_u64)
        .unwrap_or(2);
    let ps = policy.get("min_sites").and_then(Value::as_u64).unwrap_or(1);
    let holders = report
        .get("holders")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let violations = report
        .get("violations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let tiles = [
        ("Files", n("files").to_string(), size(n("bytes"))),
        (
            "Under policy",
            n("under_files").to_string(),
            size(n("under_bytes")),
        ),
        (
            "On one holder",
            String::new(),
            size(n("single_holder_bytes")),
        ),
        (
            "No verified copy",
            String::new(),
            size(n("unverified_bytes")),
        ),
    ];
    let save = {
        let scope = scope.clone();
        move |_| {
            let scope = scope.clone();
            let v = min_verified()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or_else(|| u32::try_from(pv).unwrap_or(2));
            let s = min_sites()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or_else(|| u32::try_from(ps).unwrap_or(1));
            spawn(async move {
                let done = run(
                    "FILES_POLICY_SET",
                    json!({ "scope": scope, "min_verified": v, "min_sites": s }),
                )
                .await;
                problem.set(done.err());
                bump();
            });
        }
    };
    let scrub_now = {
        let scope = scope.clone();
        move |_| {
            let scope = scope.clone();
            scrub.set(Some("Scrubbing…".to_owned()));
            spawn(async move {
                let done = run("FILES_SCRUB", json!({ "scope": scope })).await;
                scrub.set(Some(match done {
                    Ok(r) => format!(
                        "Re-hashed {} ({}), {} bad; challenged {} copies, {} passed",
                        r.get("scrubbed").and_then(Value::as_u64).unwrap_or(0),
                        size(r.get("bytes").and_then(Value::as_u64).unwrap_or(0)),
                        r.get("corrupt")
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len),
                        r.get("challenged").and_then(Value::as_u64).unwrap_or(0),
                        r.get("passed").and_then(Value::as_u64).unwrap_or(0),
                    ),
                    Err(e) => e,
                }));
                bump();
            });
        }
    };
    let count = n("violation_count");
    rsx! {
        div { style: "{PAGE}", "data-files-page": "health",
            div { style: "display: flex; flex-wrap: wrap; align-items: center; gap: 12px;",
                div { style: "display: flex; flex-direction: column; gap: 2px; flex: 1 1 240px;",
                    span { style: "{MUTED} font-size: 12px;", "Storage health" }
                    span { style: "font-size: 20px; font-weight: 700;", "{scope}" }
                    span { style: "{MUTED}", "Policy: at least {pv} verified copies at {ps} sites" }
                }
                for o in orgs {
                    button {
                        key: "{o}",
                        style: "{BUTTON}",
                        onclick: move |_| set_current(Target { root: o.clone(), ..Target::default() }),
                        "{o}"
                    }
                }
                button { style: "{BUTTON}", "data-files": "scrub", onclick: scrub_now, "Scrub now" }
            }
            if let Some(s) = scrub() { div { style: "{MUTED}", "{s}" } }
            if let Some(p) = problem() { div { style: "color: var(--destructive);", "{p}" } }
            div { style: "display: flex; flex-wrap: wrap; gap: 12px;",
                for (label, files, bytes) in tiles {
                    div { key: "{label}", style: "{CARD} flex: 1 1 150px;",
                        span { style: "{HEADING}", "{label}" }
                        span { style: "font-size: 20px; font-weight: 700;", "{bytes}" }
                        if !files.is_empty() { span { style: "{MUTED}", "{files} files" } }
                    }
                }
            }
            div { style: "display: flex; flex-wrap: wrap; gap: 16px; align-items: flex-start;",
                div { style: "{CARD} flex: 2 1 320px;",
                    span { style: "{HEADING}", "Holders" }
                    for h in holders {
                        HolderRow { key: "{text(h.get(\"holder\").unwrap_or(&Value::Null), \"id\")}", share: h }
                    }
                }
                div { style: "{CARD} flex: 1 1 220px;",
                    span { style: "{HEADING}", "Policy" }
                    div { style: "display: flex; gap: 8px; align-items: center;",
                        span { style: "width: 110px; {MUTED}", "verified copies" }
                        input { style: "{INPUT} width: 56px;", value: min_verified().unwrap_or_else(|| pv.to_string()), oninput: move |e| min_verified.set(Some(e.value())) }
                    }
                    div { style: "display: flex; gap: 8px; align-items: center;",
                        span { style: "width: 110px; {MUTED}", "sites" }
                        input { style: "{INPUT} width: 56px;", value: min_sites().unwrap_or_else(|| ps.to_string()), oninput: move |e| min_sites.set(Some(e.value())) }
                    }
                    button { style: "{BUTTON}", "data-files": "policy-save", onclick: save, "Save policy" }
                }
            }
            div { style: "{CARD}",
                span { style: "{HEADING}", "Under policy ({count})" }
                if violations.is_empty() { span { style: "{MUTED}", "Every file meets its policy." } }
                for v in violations.into_iter().take(40) {
                    ViolationRow { key: "{text(&v, \"root\")}|{text(&v, \"path\")}", v }
                }
            }
        }
    }
}

#[component]
fn HolderRow(share: Value) -> Element {
    let holder = share.get("holder").cloned().unwrap_or(Value::Null);
    let n = |k: &str| share.get(k).and_then(Value::as_u64).unwrap_or(0);
    let reachable = share
        .get("reachable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    rsx! {
        div { style: "display: flex; gap: 8px; align-items: baseline;",
            span { style: if reachable { "color: var(--success, #2e9d5b);" } else { "{MUTED}" }, "●" }
            span { style: "flex: 1;",
                span { style: "font-weight: 600;", "{text(&holder, \"name\")}" }
                span { style: "{MUTED}", " · {text(&holder, \"kind\").to_lowercase()} · {text(&holder, \"site\")}" }
            }
            span { "{size(n(\"verified_bytes\"))} verified" }
            span { style: "{MUTED}", "of {size(n(\"bytes\"))} · {n(\"files\")} blobs" }
        }
    }
}

#[component]
fn ViolationRow(v: Value) -> Element {
    let root = text(&v, "root").to_owned();
    let path = text(&v, "path").to_owned();
    rsx! {
        div {
            style: "{ROW}",
            onclick: {
                move |_| {
                    let (root, path) = (root.clone(), path.clone());
                    spawn(async move { let _ = run("FILES_OPEN", json!({ "root": root, "path": path })).await; });
                }
            },
            span { style: "flex: 1;", "{path}" span { style: "{MUTED}", " · {root}" } }
            span { style: "color: var(--destructive); font-size: 12px;", "{text(&v, \"lacks\")}" }
            span { style: "{MUTED} font-size: 12px;", "{size(v.get(\"size\").and_then(Value::as_u64).unwrap_or(0))}" }
        }
    }
}

/// Search the registry: every root this device may see, offline.
#[allow(non_snake_case)]
pub fn FindView() -> Element {
    let initial = super::current().and_then(|t| t.file).unwrap_or_default();
    let mut query = use_signal(|| initial);
    let mut hits = use_signal(|| None::<Result<Value, String>>);
    let search = move || {
        let q = query();
        spawn(async move {
            hits.set(Some(run("FILES_SEARCH", json!({ "query": q })).await));
        });
    };
    use_hook(move || {
        if !query.peek().is_empty() {
            search();
        }
    });
    let rows = match hits() {
        None => {
            rsx! { span { style: "{MUTED}", "Names, paths, properties; sha:<hex> by content; org:, root:, kind: to narrow. Answered from this device, online or not." } }
        }
        Some(Err(e)) => rsx! { span { style: "color: var(--destructive);", "{e}" } },
        Some(Ok(v)) => {
            let list = v.as_array().cloned().unwrap_or_default();
            rsx! {
                span { style: "{MUTED}", "{list.len()} results" }
                for h in list {
                    HitRow { key: "{text(&h, \"root\")}|{text(&h, \"path\")}", hit: h }
                }
            }
        }
    };
    rsx! {
        div { style: "{PAGE}", "data-files-page": "find",
            span { style: "font-size: 20px; font-weight: 700;", "Find files" }
            div { style: "display: flex; gap: 8px;",
                input {
                    style: "{INPUT} flex: 1;",
                    "data-files": "query",
                    value: query(),
                    placeholder: "lead vocal, sha:9f2c…, kind:song",
                    oninput: move |e| query.set(e.value()),
                    onkeydown: move |e| if e.key() == Key::Named(NamedKey::Enter) { search(); },
                }
                button { style: "{BUTTON}", onclick: move |_| search(), "Search" }
            }
            {rows}
        }
    }
}

/// Where a hit's bytes are, in one line.
#[must_use]
pub fn where_line(hit: &Value) -> String {
    let holders: Vec<String> = hit
        .get("holders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|h| h.as_str().map(str::to_owned))
        .collect();
    if hit.get("content").is_none_or(Value::is_null) {
        return text(hit, "detail").to_owned();
    }
    let v = hit.get("verified").and_then(Value::as_u64).unwrap_or(0);
    let reach = if hit
        .get("reachable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "reachable now"
    } else {
        "not reachable now"
    };
    if holders.is_empty() {
        format!("no holder attests it · {reach}")
    } else {
        format!("{v} verified · {} · {reach}", holders.join(", "))
    }
}

#[component]
fn HitRow(hit: Value) -> Element {
    let root = text(&hit, "root").to_owned();
    let path = text(&hit, "path").to_owned();
    let name = text(&hit, "name").to_owned();
    let what = text(&hit, "what").to_owned();
    let line = where_line(&hit);
    let reachable = hit
        .get("reachable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let file = hit.get("content").is_some_and(|c| !c.is_null());
    let dot = if !file {
        MUTED
    } else if reachable {
        "color: var(--success, #2e9d5b);"
    } else {
        "color: var(--warning, #c98a1b);"
    };
    let bytes = size(hit.get("size").and_then(Value::as_u64).unwrap_or(0));
    rsx! {
        div {
            style: "{ROW} align-items: flex-start;",
            "data-files-hit": "{path}",
            onclick: {
                move |_| {
                    let (root, path) = (root.clone(), path.clone());
                    spawn(async move { let _ = run("FILES_OPEN", json!({ "root": root, "path": path })).await; });
                }
            },
            span { style: "{dot}", "●" }
            div { style: "flex: 1; display: flex; flex-direction: column;",
                span { span { style: "font-weight: 600;", "{name}" } span { style: "{MUTED}", " · {what} · {root}" } }
                if !path.is_empty() { span { style: "{MUTED} font-size: 12px;", "{path}" } }
                span { style: "font-size: 12px;", "{line}" }
            }
            if file { span { style: "{MUTED} font-size: 12px;", "{bytes}" } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_says_how_sure_and_since_when() {
        let now = 10 * 86_400 * 1_000_000;
        let c = json!({ "confidence": "Verified", "verified_at_us": now - 3 * 3600 * 1_000_000, "how": "scrub" });
        assert_eq!(verdict(&c, now), "verified 3 h ago by scrub");
        let c = json!({ "confidence": "Stale", "verified_at_us": now - 2 * 86_400 * 1_000_000, "how": "scrub" });
        assert_eq!(verdict(&c, now), "unverified since 2 days ago");
    }

    #[test]
    fn a_hit_says_where_it_lives_and_whether_it_is_reachable() {
        let h = json!({ "content": "ab", "verified": 1, "holders": ["node (verified)", "laptop (stale)"], "reachable": false });
        assert_eq!(
            where_line(&h),
            "1 verified · node (verified), laptop (stale) · not reachable now"
        );
        let l = json!({ "verified": 1, "sites": 1, "lacks": "1 of 2 verified copies" });
        assert_eq!(
            headline(&l),
            "1 verified copy at 1 site · under policy: 1 of 2 verified copies"
        );
    }
}
