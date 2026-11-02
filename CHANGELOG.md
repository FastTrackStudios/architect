# Changelog

This file ends at v0.10.0 and is no longer written. Later releases take
their notes from the cargo-rail change files in `.changes/` (see
`.changes/README`); read them on each tag's GitHub Release, or run
`just release-notes` for what is unreleased.

## [0.10.0] - 2026-11-02

### 🚀 Features

- *(workspace)* The workspace model — tab groups, splits, docks, floats, windows, screensets (#88)
- *(shell)* Architect-shell, the app shell for phone, tablet and desk, plus the lab (#85)
- *(workspace)* The host on Blitz: tabs, splits, docks and ribbons, floats, drag and drop (#86)
- *(commands)* ⌘K, ⌘O, the action list window and the menu bar, over the Action List (#87)
- *(settings)* Settings declared by the app or by an extension, and T3 Code's settings page (#89)
- *(orgs)* The person's organizations as everything's scope (#90)
- *(documents)* Kinds of document and named undo over transactions (#91)
- *(rpc)* Query-string arguments on routes with no JSON body (#110)
- *(orgs)* Everything scoped by the current org, fetched live from fts-auth, confined server-side (#111)
- *(files)* An org's files on architect's tenancy and permissions (#117)
- *(crdt)* A document's file — durable steps, fold on save, crash recovery, branches, blobs (#118)
- *(crdt)* One sync driver per org, kept connected; the org's permission on every call (#122)
- *(blob)* Documents' blobs by hash on demand — vox, iroh, HTTP, pinning, collection (#124)
- *(crdt)* Documents peer to peer — share links, LAN discovery, device registry (#125)
- *(crdt)* Presence in view spaces, a collaborative text binding and a pane presence layer (#120)
- *(settings)* #[derive(Settings)], synced and project scopes, org defaults and locks, search to the row (#115)
- *(scripting)* Luau scripts as actions, run by people and by agents over MCP (#114)
- *(menus)* Ticked items, live sections, Linux, iPad and in-window menu bars (#116)
- *(commands)* Key bindings through architect-input and the binding editor (#121)
- *(extensions)* The runtime, a registry, hot reload and install-on-open (#123)
- *(canvas)* Layers over a view and a ReaImGui-shaped UI for scripts (#126)
- *(extensions)* Process extensions and their Dioxus views in workspace panes (#127)
- *(workspace)* Pop-out OS windows, tab reorder, touch menus, sliding stacked tabs, screensets in the host (#119)
- *(examples)* A scripting API example (#136)
- *(examples)* An extension, end to end (#137)
- *(workspace)* Pop a dock (the sidebar) out into its own window, and back (#140)
- *(lab,shell,commands)* Keyboard triage, named undo, a completion moment; critique 31/40 (#141)
- *(guide)* Guide media — scenarios as headless films and tests, multi-window (#142)
- *(lab)* Guide scenarios for the docs' pictures, and their embeds (#144)
- *(shell)* A phone action bar that becomes the keyboard bar (#149)
- *(workspace)* Motion on one clock — docks, tab moves, stacked-tab scrolling, splits (#151)
- *(workspace)* Obsidian's tab switcher on a phone (#153)
- *(guide)* In-app tours, and scenarios carried by stories (#158)
- *(workspace)* Pull up for the phone tab switcher, on a Blitz fix for the collapsing pane (#159)
- *(motion)* Measure Obsidian's animations and time the springs to them (#160)
- *(presence)* Dock badge and menu, tray item, background mode, reopen, login item, one instance (#202)
- *(spaces)* Rooms below the org — model, enforcement, storage, service (#206)
- *(devices)* One devices feature — account pass, per-org enrolment, pairing, revoking (#208)
- *(files)* An HTTP byte face, ranged reads, read tickets and renditions (#210)
- *(attachments)* Files attached to documents and records (#213)

### 🐛 Bug Fixes

- *(ui-snapshot)* Find the crate at run time, not by baking in its path (#113)
- *(pubsub)* A full sink's backlog moves on without another publish (#112)
- *(shell,lab)* Title-row drags, menus, full screen; Inter bundled; lab polish; Blitz quirks recorded (#128)
- *(host)* The permissions gate on every vox lane EngineHost owns (#156)
- *(shell)* The lab on a phone — title clear of its buttons, no key hints on touch, the drawer slide in points (#157)
- *(workspace)* Tabs slide in points; tests that a note keeps its lines while the layout moves (#189)
- *(ci)* Provide python for native stylo builds (#203)
- *(extensions)* Never-says-hello test runs the test binary, not /usr/bin/true (#204)
- *(host)* A vox_at lane's org is entered outside the host's gate (#209)

### 📚 Documentation

- *(architecture)* The app shell, orgs, workspace, menus, settings, actions, extensions, documents (specs) (#83)
- *(guide)* Build an app with architect, part 1 (the backend half) (#135)
- *(guide)* Build an app, part 2 — the app half (#134)
- *(features)* One page per major feature, each walking through its code (#139)
- Final docs pass and the README (#143)

### 🧪 Testing

- *(iroh)* Skip the LAN-discovery tests on macOS unless opted in (#205)

### ⚙️ Miscellaneous Tasks

- A PR runs the fast checks; the full suite runs on main (#129)
- Rust 1.99.1 (#109)
- Use shared battleship slots and reserved releases (#201)
## [0.9.31] - 2026-10-29

### 🚀 Features

- *(telemetry)* Trace context over vox, calls as metrics, forwarded actions, chunked doc sync (0.9.31) (#82)
## [0.9.30] - 2026-10-25

### 🚀 Features

- *(auth)* One device identity for every app on a device (0.9.30) (#81)
## [0.9.29] - 2026-10-25

### 🚀 Features

- *(crdt)* App docs peer to peer, Lineage, sync events; device relay_url (0.9.29) (#80)
## [0.9.28] - 2026-10-24

### 🚀 Features

- *(crdt)* Crdt::docs — app document catalog, host and device driver (0.9.28) (#79)
## [0.9.27] - 2026-10-23

### 🚀 Features

- *(rpc)* Verbs, path params and body limits for service methods (#52) (#69)
- *(xtask)* Hand-routes counts axum route calls, not every .route( (#52) (#65)
- *(host)* Vox_at, dav and document protocol mounts on EngineHost (#52) (#67)
- *(host)* Otlp_forward, and MCP on EngineHost via architect_mcp::host::McpHost (#52) (#68)
- *(rpc)* Raw-HTTP service methods: streamed bodies and byte ranges (#52, 2/2) (#70)

### 🐛 Bug Fixes

- *(auth-client)* Persist the refresh token and add refresh helpers (#20) (#64)
- *(iroh_link)* Redial a restarted peer by id (#56) (#66)
- *(crdt)* SyncedDoc::run reports a refused attach every time (#77)

### 📚 Documentation

- *(changelog)* 0.9.27 (#76)

### ⚙️ Miscellaneous Tasks

- *(boundaries)* Drop the hand-routes check; fix inert RequestHead clippy (#74)
- Run checks and release on GitHub-hosted runners (#75)
## [0.9.26] - 2026-10-23

### 🚀 Features

- *(input)* Key releases on the hook, timer outside the runtime, MacroKeys::Off (#71) (#72)

### 📚 Documentation

- *(changelog)* 0.9.26 (#73)
## [0.9.25] - 2026-10-22

### 📚 Documentation

- *(skill)* The architect skill lives in the repo, checked by xtask docs (#62)
- *(changelog)* 0.9.25 (#63)
## [0.9.24] - 2026-10-22

### 🚀 Features

- *(actions)* Handlers see the caller; per-tool architect-mcp API (#58) (#59)
- *(actions)* Flat schemas, plain-text MCP errors, RawJson results (#58) (#60)

### 🐛 Bug Fixes

- *(rpc)* An argless SSE subscription delivers its intro first (#47)

### 📚 Documentation

- *(changelog)* 0.9.24 (#61)
## [0.9.23] - 2026-10-22

### 🚀 Features

- *(input)* Architect-input keymap engine, moved from studio (#50) (#54)

### 📚 Documentation

- *(changelog)* 0.9.23 (#57)
## [0.9.22] - 2026-10-22

### 🚀 Features

- *(release)* Architect::release signed self-update core (STA-387) (#53)

### 📚 Documentation

- *(layout)* Backends and extensibility in the one-crate shape (STA-322) (#51)
- *(changelog)* 0.9.22 (#55)
## [0.9.21] - 2026-10-22

### 🚀 Features

- *(actions)* Async actions, service-to-action projection, named MCP tools (STA-261) (#46)
## [0.9.20] - 2026-10-22

### 🚀 Features

- *(crdt)* Crdt(text) LoroText fields + crdt::schema migration helper (STA-271) (#45)
## [0.9.19] - 2026-10-22

### 🚀 Features

- *(architect)* Snapshot — WriteGate, GatedLayer and bounded WAL-checkpoint quiesce (STA-399) (#43)
## [0.9.18] - 2026-10-22

### 🚀 Features

- *(architect)* Testing — sandbox boot and counting allocator (STA-400) (#44)
## [0.9.17] - 2026-10-22

### 🚀 Features

- *(host)* /version.json on EngineHost (STA-391) (#41)
## [0.9.16] - 2026-10-22

### 🚀 Features

- *(architect)* Blob — resumable verified fetch and content-addressed BlobStore (STA-388) (#42)
## [0.9.15] - 2026-10-22

### 🚀 Features

- *(architect)* Capability — Ed25519 expiring purpose|expiry|subject tokens (STA-389) (#40)
## [0.9.14] - 2026-10-22

### 🚀 Features

- *(mcp)* Discovery file, .mcp.json and loopback serve (STA-385) (#39)
## [0.9.13] - 2026-10-22

### 🚀 Features

- *(telemetry)* Crash flight recorder — architect_telemetry::flight (STA-384) (#38)
## [0.9.12] - 2026-10-22

### 🚀 Features

- *(xtask)* Feature new scaffolds the one-crate layout (STA-322) (#29)
- *(settings)* Architect::settings::Store<T> and FileKv (STA-390) (#36)
- *(http)* Architect::http::ranged — Range/206 serving (STA-383) (#37)

### 📚 Documentation

- Repair dead paths and commands; peers.md; cargo xtask docs checker (STA-269) (#35)
## [0.9.11] - 2026-10-22

### 🚀 Features

- *(architect)* Harden the Action List — enabled reason, authorize, panic guard, validate (STA-260) (#34)
## [0.9.10] - 2026-10-22

### 🚀 Features

- *(xtask)* Boundaries — enforce the crate-layout ADR, ratchet idioms (STA-268) (#30)

### 📚 Documentation

- *(changelog)* 0.9.10 — xtask boundaries (STA-268) (#33)
## [0.9.9] - 2026-10-22

### 🚀 Features

- *(crdt)* Crdt::peer pairwise document exchange + DocRegistry::with_authorizer (#28)
## [0.9.8] - 2026-10-22

### 🚀 Features

- *(iroh)* LAN peer discovery via mDNS address lookup (STA-265) (#31)

### 🐛 Bug Fixes

- *(atom)* Clippy::map_entry in Store::put (main CI red since 00ac667) (#32)
## [0.9.7] - 2026-10-22

### 🚀 Features

- *(architect)* The Action List, architect-mcp and telemetry timing on main (#26)

### 🐛 Bug Fixes

- *(telemetry)* Timing::DIRECTIVES doc's first paragraph fits clippy::too_long_first_doc_paragraph

### 📚 Documentation

- *(layout)* ADR — one crate per feature with cargo features (STA-257) (#27)
- *(architecture)* Commands and the Action List; CHANGELOG for v0.9.7

### 💼 Other

- An app's Action List as an MCP server
- Serve(listener, server, token), for an app to spawn
- A flattened struct is its fields
- :timing: stage timings kept in-process; a span per action
- :DIRECTIVES: the Action List's span through a quiet filter too
## [0.9.6] - 2026-10-19

### 💼 Other

- An account's devices and the org they share (#25)
## [0.9.5] - 2026-10-14

### 💼 Other

- A create's live event no longer lists the row twice (#22)
- Toasts show their action button (#23)
## [0.9.4] - 2026-09-25

### 💼 Other

- The device sign-in protocol, for any client to drive
## [0.9.3] - 2026-09-24

### 💼 Other

- A device with no browser signs in over the AuthService — device/code, device/token
## [0.9.2] - 2026-09-24

### 💼 Other

- :send: a producer that streams on its own waits for its reader
- :send: Sync events, so the send is Send
- The presence refresh takes the poison-tolerant lock
## [0.9.1] - 2026-09-24

### 💼 Other

- *(Entity)* The HTTP handlers are built with vox
## [0.9.0] - 2026-09-24

### 💼 Other

- An argless subscription can introduce itself
- Facet/vox/phon to rc.7, caret ranges
- A presence host's own writes reach its peers
- Presence keeps a still peer and drops a gone one
## [0.8.11] - 2026-09-22

### 🐛 Bug Fixes

- *(auth-server)* Linked tokens refresh instead of going stale (#19)
## [0.8.10] - 2026-09-16

### 🐛 Bug Fixes

- *(telemetry)* The OTLP trace layer obeys the log filter (#17)
## [0.8.9] - 2026-09-14

### 🐛 Bug Fixes

- *(auth-db)* A migration for auth_agent_links on databases that predate it (#16)
## [0.8.8] - 2026-09-14

### 🚀 Features

- *(auth)* A linked agent inherits its owner's organizations, up to a cap (#15)
## [0.8.7] - 2026-09-14

### 🚀 Features

- *(auth)* An OIDC access token can list the organizations it belongs to (#14)

### 🐛 Bug Fixes

- *(auth-server)* A dead cookie is judged on the session, not on the engine's error (#13)
## [0.8.6] - 2026-09-14

### 🐛 Bug Fixes

- *(auth-server)* A stale session cookie on /oauth2/authorize is a sign-in, not a 401 (#12)
## [0.8.5] - 2026-09-14

### 🚀 Features

- *(auth)* The issuer is a portal you can actually walk into (#11)
## [0.8.4] - 2026-09-14

### 🚀 Features

- *(auth)* Leave an organization, and remove a member (#10)
## [0.8.3] - 2026-09-14

### 🚀 Features

- *(auth)* The organization management the engine was missing
- *(auth-ui)* The pages, in a crate anything can mount
- *(auth)* A fresh local server, and a safe copy of a real one
- *(auth-ui)* Two-factor, and the two bugs under it
- *(auth-ui)* API keys and the operator screen
- *(auth)* [**breaking**] Real WebAuthn — the assertion is verified now
- *(auth-ui)* Passkey pages, and the one script in this UI
- *(auth-ui)* Sign in with a code, a link, or a username
- *(auth-ui)* Connect a device, and hold several accounts at once
- *(auth-ui)* Start as a guest, and attach a phone number
- *(auth)* [**breaking**] Real Sign-In With Ethereum, and the OIDC consent screen
- *(auth-ui)* A settings application, not ten more cards
- *(mock-oauth)* A stand-in for GitHub, Google and TONE3000
- *(auth-ui)* Serve the design system's stylesheet
- *(auth-ui)* The settings chassis is the design system's now
- *(architect)* A method may declare its HTTP path (#9)

### 🐛 Bug Fixes

- *(auth)* [**breaking**] Passkey sign-in was an authentication bypass; fail it closed
- *(auth)* [**breaking**] Two-factor guarded the password form and nothing else
- *(auth-server)* `sqlite::memory:` gave every connection its own database
- *(mock-oauth)* `drop` on a Copy value does nothing
- *(auth)* The organization surface belongs under /auth (#8)

### 🧪 Testing

- *(auth)* Browser tests, and the message they found
- *(auth-ui)* Browser cover for the new pages, and two shared-state bugs

### ⚙️ Miscellaneous Tasks

- Adopt the no-panic clippy config, and fix what it found
- *(auth)* Bring the auth server under the no-panic lints

### 💼 Other

- :error]: a service error with nothing left to write
- :error]: the status attribute says it is HTTP (http_status = 404)
## [0.8.2] - 2026-09-05

### ⚙️ Miscellaneous Tasks

- Checks on architect's own self-hosted runner, warm target dir (#3)
- Run on the repo's own self-hosted runner (#4)
## [0.8.1] - 2026-09-04

### 🚀 Features

- *(auth-server)* GitHub + Google sign-in, account linking, linked-token RP endpoint (#2)
## [0.8.0] - 2026-09-04

### 🚀 Features

- *(auth-server)* Outgoing mail, email verification, password reset
## [0.7.4] - 2026-09-04

### 🐛 Bug Fixes

- *(auth-server)* Unindent an error string that rustdoc read as a doctest
## [0.7.3] - 2026-09-04

### 🐛 Bug Fixes

- *(auth-server)* Encode return_to into the sign-up/sign-in links
## [0.7.2] - 2026-09-03

### 🚀 Features

- *(auth)* Confidential OIDC clients, and PKCE only where it protects
## [0.7.1] - 2026-09-01

### 🚀 Features

- *(auth-client)* The redirect sign-in, shared
## [0.7.0] - 2026-09-01

### 🚀 Features

- *(auth-server)* The sign-in and sign-up pages
## [0.6.0] - 2026-08-31

### 🚀 Features

- *(auth)* Standalone auth server (apps/auth-server) (#1)
## [0.0.2] - 2026-08-26

### 🚀 Features

- *(action)* A display hint on ActionMeta
- *(host)* Cross-origin isolation, and fix chrono/serde in a narrow graph

### 🐛 Bug Fixes

- *(rig)* Re-attach MIDI after a transient-empty port scan
## [0.5.0] - 2026-08-17

### 🚀 Features

- *(gate)* A handler can see the caller the gate resolved
## [0.4.0] - 2026-08-17

### 🚀 Features

- *(permissions)* Principal::Host — a peer server is not a person
## [0.3.0] - 2026-08-16

### 🚀 Features

- *(architect-ui)* Ship compiled utilities as UTILITIES_CSS

### 🐛 Bug Fixes

- *(launcher)* Allow the infallible CommandMode::from_str

### 🚜 Refactor

- Adopt the crates/ + features/ layout
- The derive macros belong in crates/, not features/

### 🎨 Styling

- Cargo fmt after the extraction renames
## [0.2.0] - 2026-08-15

### 🚀 Features

- *(launcher)* Vendor the launcher engine as architect-launcher-*
## [0.1.1] - 2026-08-15

### 🚀 Features

- *(architect-ui)* Embed the theme stylesheet as THEME_CSS
## [0.1.0] - 2026-08-15

### 💼 Other

- Architect, extracted from the FastTrackStudio monorepo
