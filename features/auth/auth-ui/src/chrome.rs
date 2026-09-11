//! The chrome every hosted auth page sits in.
//!
//! One stylesheet and one frame, owned here rather than in the server
//! binary, so a page added by any consumer looks like the rest of the
//! studio without copying 230 lines of CSS to get there.
//!
//! The whole sheet is inlined into each response on purpose. These are
//! the pages that must work before anything else does — a sign-in
//! screen whose stylesheet 404s is a sign-in screen nobody can read —
//! and one document with no subresources has no second request to fail.

use dioxus::prelude::*;

/// The apps one account opens, each with the colour the studio gives it
/// on fasttrackstudio.app. The brand panel draws them as a short
/// spectrum — the site's own motif — with the names beneath.
pub(crate) const APPS: [(&str, &str); 5] = [
    ("Task", "#ededf1"),
    ("Keyflow", "#a78bfa"),
    ("Signal", "#2fd673"),
    ("Session", "#2e9bff"),
    ("Ignition", "#ff8a2b"),
];

/// The page frame every hosted screen sits in: the brand panel that says
/// what this account is for, and the working panel beside it. On a
/// narrow screen the brand panel folds into a short header so the form
/// is the first thing in reach.
#[component]
pub fn Shell(children: Element) -> Element {
    rsx! {
        div { class: "console",
            aside { class: "brand",
                a { class: "wordmark", href: "https://fasttrackstudio.app", "FastTrackStudio" }
                div { class: "pitch",
                    p { class: "tagline", "One account." }
                    p { class: "tagline dim", "Every app in the studio." }
                }
                ul { class: "apps", aria_label: "Apps this account signs in to",
                    for (name, color) in APPS {
                        li { style: "--app: {color}",
                            i { class: "bar" }
                            span { "{name}" }
                        }
                    }
                }
            }
            main { class: "panel", {children} }
        }
    }
}
pub const STYLE: &str = r#"
@import url("https://fonts.googleapis.com/css2?family=Archivo:wdth,wght@87.5..112.5,400..700&family=JetBrains+Mono:wght@400;700&display=swap");
:root {
  color-scheme: dark;
  /* fasttrackstudio.app's own tokens */
  --void: #08080a;
  --bg: #0a0a0c;
  --deck: #131318;
  --surface: #16161c;
  --raised: #1d1d25;
  --line: #26262f;
  --line-strong: #353541;
  --fg: #ededf1;
  --muted: #9c9ca8;
  --subtle: #63636f;
  --error: #ff8a7a;
  --ok: #2fd673;
  --sans: "Archivo", system-ui, -apple-system, "Segoe UI", sans-serif;
  --mono: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
}
* { box-sizing: border-box; }
/* This sheet is served to the settings pages as well, which are drawn
   with the design system — so everything below is scoped to the centred
   card shell rather than to the document. An unscoped `body` rule here
   would centre the settings app and repaint it in these colours instead
   of the themed ones. */
html:has(body.auth-card) { background: var(--bg); }
body.auth-card {
  margin: 0;
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 1.25rem;
  color: var(--fg);
  font: 15px/1.5 var(--sans);
  font-variation-settings: "wdth" 100;
  -webkit-font-smoothing: antialiased;
}
.console {
  width: 100%;
  max-width: 56rem;
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 6fr);
  background: var(--deck);
  border: 1px solid var(--line);
  border-radius: 14px;
  overflow: hidden;
}
/* ── Brand panel ─────────────────────────────────────── */
.brand {
  display: flex;
  flex-direction: column;
  gap: 2.5rem;
  padding: 2.25rem 2rem;
  background: var(--void);
  border-right: 1px solid var(--line);
}
.wordmark {
  align-self: flex-start;
  margin: 0;
  color: var(--fg);
  text-decoration: none;
  font-weight: 700;
  font-size: .95rem;
  letter-spacing: .02em;
  text-transform: uppercase;
  font-variation-settings: "wdth" 95;
}
.pitch { margin-top: auto; }
.tagline {
  margin: 0;
  font-size: clamp(1.75rem, 3.2vw, 2.25rem);
  line-height: 1.05;
  font-weight: 600;
  letter-spacing: -.02em;
  font-variation-settings: "wdth" 92;
}
.tagline.dim { color: var(--muted); }
/* the site's spectrum motif: one bar per app, in the app's colour */
.apps {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  grid-template-columns: repeat(5, 1fr);
  gap: .6rem;
}
.apps li {
  display: grid;
  gap: .55rem;
  font: 700 .6rem/1 var(--mono);
  letter-spacing: .18em;
  text-transform: uppercase;
  color: var(--subtle);
}
.apps .bar {
  display: block;
  height: 3px;
  border-radius: 2px;
  background: var(--app);
  opacity: .9;
}
/* ── Working panel ───────────────────────────────────── */
.panel {
  padding: 2.25rem 2.25rem 2rem;
  background: var(--deck);
}
main h1 {
  margin: 0 0 .3rem;
  font-size: 1.6rem;
  line-height: 1.15;
  font-weight: 600;
  letter-spacing: -.015em;
  font-variation-settings: "wdth" 95;
}
main h2 {
  margin: 1.75rem 0 .35rem;
  font-size: 1rem;
  font-weight: 600;
}
.sub { margin: 0 0 1.5rem; color: var(--muted); }
.hint { margin: 0 0 1rem; color: var(--muted); font-size: .9rem; }
main label {
  display: block;
  margin: 0 0 .35rem;
  color: var(--muted);
  font-size: .85rem;
  font-weight: 500;
}
main input {
  width: 100%;
  margin: 0 0 .9rem;
  padding: .7rem .8rem;
  font: inherit;
  color: var(--fg);
  background: var(--surface);
  border: 1px solid var(--line-strong);
  border-radius: 8px;
}
main input:hover { border-color: var(--subtle); }
main input:focus-visible { outline: 2px solid var(--fg); outline-offset: 1px; border-color: var(--fg); }
main button {
  width: 100%;
  margin-top: .25rem;
  padding: .75rem;
  font: inherit;
  font-weight: 600;
  color: var(--void);
  background: var(--fg);
  border: 0;
  border-radius: 8px;
  cursor: pointer;
}
main button:hover { background: #fff; }
main button:focus-visible { outline: 2px solid var(--fg); outline-offset: 2px; }
/* provider buttons: the mark, then the words, centred as one unit */
.social { display: grid; gap: .6rem; }
main a.button {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: .65rem;
  padding: .72rem .9rem;
  text-decoration: none;
  font-weight: 600;
  color: var(--fg);
  background: var(--surface);
  border: 1px solid var(--line-strong);
  border-radius: 8px;
}
main a.button:hover { background: var(--raised); border-color: var(--subtle); }
main a.button:focus-visible { outline: 2px solid var(--fg); outline-offset: 2px; }
main a.button.small { display: inline-flex; padding: .45rem .8rem; font-size: .875rem; }
.mark { flex: none; }
.or {
  display: flex;
  align-items: center;
  gap: .9rem;
  margin: 1.25rem 0 1.1rem;
  color: var(--subtle);
  font-size: .8rem;
}
.or::before, .or::after { content: ""; flex: 1; height: 1px; background: var(--line); }
.error, .ok {
  margin: 0 0 1rem;
  padding: .65rem .8rem;
  font-size: .9rem;
  border-radius: 8px;
  border: 1px solid;
}
.error { color: var(--error); border-color: color-mix(in srgb, var(--error) 45%, transparent); }
.ok { color: var(--ok); border-color: color-mix(in srgb, var(--ok) 45%, transparent); }
.alt { display: flex; justify-content: space-between; gap: 1rem; flex-wrap: wrap; margin: 1.5rem 0 0; color: var(--muted); font-size: .9rem; }
main a.quiet { color: var(--muted); }
main a.quiet:hover { color: var(--fg); }
main a { color: var(--fg); text-underline-offset: .15em; }
main a:hover { color: #fff; }
/* account: linked providers */
.providers { list-style: none; margin: 0; padding: 0; display: grid; }
.provider {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  padding: .9rem 0;
  border-top: 1px solid var(--line);
}
.provider:last-child { border-bottom: 1px solid var(--line); }
.provider-name { display: flex; align-items: center; gap: .8rem; }
.provider-name strong { display: block; font-weight: 600; }
.handle { display: block; color: var(--muted); font-size: .85rem; }
main form.inline { display: inline; margin: 0; }
main button.link {
  width: auto;
  margin: 0;
  padding: 0;
  font-weight: 500;
  color: var(--muted);
  background: none;
  border: 0;
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: .15em;
}
main button.link:hover { color: var(--fg); background: none; }
@media (max-width: 52rem) {
  body.auth-card { padding: 0; align-items: start; }
  .console { max-width: none; min-height: 100vh; grid-template-columns: 1fr; border: 0; border-radius: 0; }
  .brand { gap: 1.25rem; padding: 1.25rem 1.5rem; border-right: 0; border-bottom: 1px solid var(--line); }
  .pitch { display: none; }
  .apps { gap: .4rem; }
  .panel { padding: 1.75rem 1.5rem 2rem; }
}
@media (prefers-reduced-motion: no-preference) {
  a.button, button, input { transition: background-color .15s ease, border-color .15s ease; }
}
/* ══ Settings ══════════════════════════════════════════════════════
   The chassis — rail, frame and working surface — is the design
   system's now (see `settings.rs`). What is left below is the module
   styling the page bodies still use, until each page is converted in
   turn. */
/* A module. Bolted down, not floating: no shadow, small radius. */
.panel {
  padding: 1.4rem 1.5rem;
  background: var(--deck);
  border: 1px solid var(--line);
  border-radius: 10px;
}
.panel > h2:first-child { margin-top: 0; }
.panel > .sub:last-child, .panel > .hint:last-child { margin-bottom: 0; }
/* Forms want a readable measure; a text field the width of a table
   is a field nobody can scan. */
.panel form.stack { max-width: 26rem; }
.panel form.stack input:last-of-type { margin-bottom: 1.1rem; }
.panel form.stack button { width: auto; min-width: 11rem; }

/* ── Tables ──────────────────────────────────────────── */
main table.grid {
  width: 100%;
  margin: 0 0 .25rem;
  /* A table that follows a form needs air, or the header row reads as
     part of the button above it. */
  border-collapse: collapse;
  font-size: .92rem;
}
main table.grid th {
  padding: 0 .75rem .55rem 0;
  border-bottom: 1px solid var(--line);
  color: var(--subtle);
  font-size: .8rem;
  font-weight: 500;
  text-align: left;
}
main table.grid td {
  padding: .7rem .75rem .7rem 0;
  border-bottom: 1px solid var(--line);
  vertical-align: top;
}
main table.grid tr:last-child td { border-bottom: 0; }
main table.grid td:last-child, main table.grid th:last-child { padding-right: 0; text-align: right; }
/* Several inline forms in one cell. Without this they run together as
   one word — "BanSign in asDelete" — which is how the admin table
   read before there was any CSS for it at all. */
main table.grid td:last-child form.inline { display: inline-flex; }
main table.grid td:last-child > * + * { margin-left: .9rem; }
main table.grid td input { width: auto; margin: 0 .5rem 0 0; padding: .35rem .5rem; font-size: .85rem; }
main table.grid select { margin-right: .5rem; }
.wide { overflow-x: auto; }
main form + .wide, main form + table.grid { margin-top: 1.75rem; }
.wide + form.stack, table.grid + form.stack { margin-top: 1.5rem; }

/* ── Controls ────────────────────────────────────────── */
main select {
  max-width: 100%;
  padding: .4rem .5rem;
  font: inherit;
  font-size: .85rem;
  color: var(--fg);
  background: var(--surface);
  border: 1px solid var(--line-strong);
  border-radius: 7px;
}
main select:focus-visible { outline: 2px solid var(--fg); outline-offset: 1px; }
main form.stack select { width: 100%; margin-bottom: .9rem; padding: .7rem .8rem; font-size: 1rem; }
main button.danger { color: var(--error); }
main button.link.danger:hover { color: var(--error); }
.mono { font-family: var(--mono); font-size: .85rem; font-variant-numeric: tabular-nums; }
main input.mono { font-size: .85rem; }
.tag {
  display: inline-block;
  margin-left: .5rem;
  padding: .08rem .42rem;
  border: 1px solid var(--line-strong);
  border-radius: 999px;
  color: var(--muted);
  font-size: .72rem;
  font-variation-settings: "wdth" 92;
  vertical-align: 1px;
}

/* ── Lists ───────────────────────────────────────────── */
main ul.orgs, main ul.teams, main ul.codes, main ul.providers { margin: 0; padding: 0; list-style: none; }
main ul.orgs, main ul.teams { display: flex; flex-direction: column; }
main li.org, main li.team {
  display: flex;
  align-items: center;
  gap: .75rem;
  padding: .8rem 0;
  border-bottom: 1px solid var(--line);
}
main li.org > a, main li.team > .team-head { flex: 1; min-width: 0; text-decoration: none; color: var(--fg); }
main li.org:last-child, main li.team:last-child { border-bottom: 0; }
main li.org > a strong, main li.team strong { font-weight: 600; }
/* `li.team` beats `.team` on specificity, so the override has to
   match the same way — otherwise `align-items: center` from the shared
   rule above survives and centres the whole block. */
main li.team { flex-direction: column; align-items: stretch; gap: .45rem; }
.team-head { display: flex; align-items: center; justify-content: space-between; gap: .75rem; }
/* Backup codes and scopes: read down a column, not across a line. */
main ul.codes {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
  gap: .35rem .9rem;
  margin: 0 0 1rem;
  font-family: var(--mono);
  font-size: .9rem;
}
main li.provider { display: flex; align-items: center; justify-content: space-between; gap: .9rem; padding: .8rem 0; border-bottom: 1px solid var(--line); }
main li.provider:last-child { border-bottom: 0; }
.provider-name { display: flex; align-items: center; gap: .7rem; }

/* ── Shown once ──────────────────────────────────────── */
/* A key or an invite URL that cannot be fetched again. Raised off the
   deck so it reads as something to act on now, not a row to skim. */
.minted {
  margin: 0 0 1.1rem;
  padding: 1rem 1.1rem;
  background: var(--raised);
  border: 1px solid var(--line-strong);
  border-radius: 9px;
}
.minted p { margin-top: 0; }
.minted input { margin: 0; }
.qr { margin: 0 0 1rem; }
.qr svg { display: block; }

.alt { margin: 1.5rem 0 0; color: var(--muted); font-size: .9rem; }

/* ── Narrow ──────────────────────────────────────────── */
@media (max-width: 60rem) {
  main table.grid td:last-child > * + * { margin-left: .6rem; }
}
@media (prefers-reduced-motion: reduce) {
  * { transition: none !important; animation: none !important; }
}

"#;
