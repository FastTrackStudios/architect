# architect — recipes
# Run commands: just <recipe-name>

default:
    @just --list --unsorted

# ── Run it (docs/content/guide/local-stack.md) ───────────────────────────
# The same three recipes, with the same meaning, in Architect, Studio and
# Keyflow. With a Studio checkout beside this one (../studio, or
# LOCAL_STACK_STUDIO_PATH) they run Studio's stack, Studio's extensions
# on its node, with THIS checkout as its Architect (`architect=local`);
# `plain` runs Architect's own stack instead (`devenv up`: the demo
# seed, no Studio). Arguments go to Studio's `just dev` (keyflow=local,
# desktop=on, watch=off).

# Everything locally, rebuilt on save: Studio's node with this Architect, issuer, web app, site (`plain`: Architect's own stack, the desktop app and the web app under dx serve, and the site)
dev *ARGS:
    @bash scripts/run-it.sh dev {{ARGS}}

# See it working, quickly: issuer + a node with a share link, no watchers (`plain`: Architect's demo seed)
demo *ARGS:
    @bash scripts/run-it.sh demo {{ARGS}}

# The public website (apps/site: landing, download, changelog, extensions, docs), hot reload
website:
    devenv up site

# The running Studio stack's node on this tailnet over HTTPS for a phone (tailscale serve, never funnel); `just phone off`
phone *ARGS:
    @bash scripts/run-it.sh phone {{ARGS}}

# Recompile architect-ui's utility sheet.
#
# Run this after adding Tailwind classes to any component. The output is
# COMMITTED and embedded in the crate as `architect_ui::UTILITIES_CSS`,
# because consumers in other repos cannot scan our sources — `@source`
# resolves on the filesystem and a git dep has no stable path, so a
# downstream sheet silently omits every class used only in here.
ui-css:
    cd features/ui/architect-ui && tailwindcss -i tailwind.css -o assets/utilities.css --minify

# Fail if the committed utility sheet is not what the sources produce.
# Same rot problem as any generated-but-committed artifact: add a class,
# forget to rebuild, and a consumer renders it unstyled.
ui-css-check: ui-css
    #!/usr/bin/env bash
    set -euo pipefail
    if ! git diff --quiet -- features/ui/architect-ui/assets/utilities.css; then
        echo "utilities.css is out of date — run 'just ui-css' and commit the result" >&2
        exit 1
    fi
    echo "utilities.css is up to date"

# The auth pages' sheet. Same generated-but-committed deal as `ui-css`,
# scanning this crate AND architect-ui so a class used here but not by
# any architect-ui component still gets a rule — without it the element
# renders unstyled and nothing warns.
auth-ui-css:
    cd features/auth/auth-ui && tailwindcss -i tailwind.css -o assets/utilities.css --minify

auth-ui-css-check: auth-ui-css
    #!/usr/bin/env bash
    set -euo pipefail
    if ! git diff --quiet -- features/auth/auth-ui/assets/utilities.css; then
        echo "auth-ui's utilities.css is out of date — run 'just auth-ui-css' and commit the result" >&2
        exit 1
    fi
    echo "auth-ui utilities.css is up to date"

check:
    cargo check --workspace --all-targets

# nextest runs every test; it does not run doctests, so those follow. Then
# the tests a feature keeps out of that run (feature-tests.toml).
test:
    cargo nextest run --workspace
    cargo test --workspace --doc
    cargo run -q -p xtask -- feature-tests --run

lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features

# Release tags must order the commits the way their numbers do — six
# repos pin this one by tag, and that is the only question they ask.
tags:
    cargo run -p xtask -- tags --verbose

# Every PR that changes a crate carries a cargo-rail change file (`cargo
# rail change add <crates> --bump none|patch|minor|major -m "…"`;
# .changes/README). This is pr.yml's gate: against BASE when given (CI
# passes the PR's base sha), else the merge-base with main.
ci-changes BASE="":
    if [ -n "{{BASE}}" ]; then cargo rail change check --since "{{BASE}}"; else cargo rail change check --merge-base; fi

# A tag's release notes (the change files added since the tag before it),
# or the draft for origin/main (or a given revision) since its newest tag.
# release.yml publishes exactly this.
release-notes *REF:
    @bash scripts/release-notes.sh notes {{REF}}

# The next semver tag for origin/main (or a given revision): the highest
# change-file bump since the newest tag picks it (.changes/README).
release-next *REF:
    @bash scripts/release-notes.sh next {{REF}}

# Film the sandbox's guide scenarios (apps/sandbox/scenarios; all of them, or
# one by id) into target/media: <id>.gif and <id>.png, and .webm/.mp4
# where a scenario asks. Needs ffmpeg and gifski (the dev shell has them).
media *IDS:
    cargo run -q -p architect-sandbox-media -- {{IDS}} --out target/media

# Play the sandbox's guide scenarios as tests, no frames.
media-test *IDS:
    cargo run -q -p architect-sandbox-media -- {{IDS}} --test

# Regenerate every scenario's media and commit them to the orphan `media`
# branch (only the generated files, its folders kept; README and docs embed them from
# raw.githubusercontent.com/FastTrackStudios/architect/media/<id>.gif).
# Works in a temporary worktree: this checkout and its branch are never
# touched. PUSH=1 pushes the branch too.
media-publish PUSH="0":
    #!/usr/bin/env bash
    set -euo pipefail
    root=$(git rev-parse --show-toplevel)
    out=$(mktemp -d)
    tree=$(mktemp -d)
    trap 'git -C "$root" worktree remove --force "$tree" 2>/dev/null || true; rm -rf "$out" "$tree"' EXIT
    cargo run -q -p architect-sandbox-media -- --out "$out"
    git -C "$root" fetch -q origin media 2>/dev/null || true
    if git -C "$root" rev-parse -q --verify origin/media >/dev/null; then
        git -C "$root" worktree add -q --detach "$tree" origin/media
        git -C "$tree" checkout -q -B media
    else
        git -C "$root" worktree add -q --detach "$tree"
        git -C "$tree" checkout -q --orphan media
        git -C "$tree" rm -rq --cached . >/dev/null
        git -C "$tree" clean -fdxq
    fi
    # The generated files only: folders on the branch (a PR's own evidence,
    # 443-navigator/, landing-page/) are not regenerated here and are kept.
    find "$tree" -mindepth 1 -maxdepth 1 -type f ! -name .git -exec rm -f {} +
    find "$out" -maxdepth 1 -type f \( -name '*.gif' -o -name '*.webm' -o -name '*.mp4' -o -name '*.png' \) -exec cp {} "$tree"/ \;
    git -C "$tree" add -A
    if git -C "$tree" diff --cached --quiet; then
        echo "media: nothing changed"
    else
        rev=$(git -C "$root" rev-parse --short HEAD)
        git -C "$tree" commit -qm "media: regenerated from $rev"
        echo "media: committed $(git -C "$tree" rev-parse --short HEAD) to media"
    fi
    if [ "{{PUSH}}" = "1" ]; then
        git -C "$tree" push -q origin media
        echo "media: pushed"
    fi

# Build architect's docs (docs/content) into target/site: a static site
# with the Board's ```kf charts drawn by its own page (#364).
docs OUT="target/site":
    cargo run -q -p architect-cli -- site build --vault docs/content --out {{OUT}} \
        --extension apps/sandbox/board \
        --url https://architect.fasttrackstudio.app

# Serve the docs on http://127.0.0.1:4321/, rebuilt and reloaded on save.
docs-serve PORT="4321":
    cargo run -q -p architect-cli -- site serve --vault docs/content --out target/site-serve \
        --extension apps/sandbox/board --port {{PORT}}

# Every path and command the docs and skills name must exist (CI runs this).
docs-check:
    cargo run -q -p xtask -- docs

# Link this checkout's architect skill into Claude Code and Codex, for a
# machine the fleet-skills sync doesn't reach. Replaces an existing
# symlink; refuses to touch a real file or directory.
skill-install:
    #!/usr/bin/env bash
    set -euo pipefail
    src="$(pwd)/skills/architect"
    dirs=("$HOME/.claude/skills" "$HOME/.codex/skills")
    for dir in "${dirs[@]}"; do
        if [ -e "$dir/architect" ] && [ ! -L "$dir/architect" ]; then
            echo "$dir/architect exists and is not a symlink; move it aside first" >&2
            exit 1
        fi
    done
    for dir in "${dirs[@]}"; do
        dest="$dir/architect"
        mkdir -p "$dir"
        ln -sfn "$src" "$dest"
        echo "$dest -> $src"
    done

# Repoint every architect-owned git dep in a consumer at ONE tag.
# Bumping them one at a time is how `signal` ended up with three
# different checkouts of this repo in one lockfile.
#   just fleet-bump v0.9.0 ../signal ../task
fleet-bump TAG +MANIFESTS:
    cargo run -p xtask -- fleet-bump {{TAG}} {{MANIFESTS}}

# ── Local auth servers ───────────────────────────────────────────────

# A fresh auth server on http://localhost:8080, seeded with a fixed cast.
#
# Three people (ada@local.test is the administrator), two organizations,
# a team, a pending invitation and a live invite link — enough that every
# section of every page has something in it. Everyone's password is
# `development-password`.
#
# The seed is deterministic: the same accounts and the same organizations
# on every machine and every run, which is what lets a browser test
# assert on `ada@local.test` owning `acme` rather than on whatever the
# fixture happened to make.
#
# Social sign-in is on, pointed at the mock provider below rather than
# at GitHub, Google and TONE3000 — so the buttons work without anyone's
# credentials. `just mock-oauth` has to be running; `just auth-demo`
# starts both.
#
# Its environment is written once, in `cargo xtask dev auth`
# (xtask/src/dev/mod.rs `auth_env`), which `devenv up`'s auth server
# shares; it execs the server.
auth-dev:
    cargo xtask dev auth --port 8080 --db memory --seed --mock-port 4040

# A stand-in for GitHub, Google and TONE3000 on http://localhost:4040.
#
# It verifies nothing — no client secret, no PKCE, no expiry — and says
# so on every page it renders. Setting AUTH_SOCIAL_MOCK_URL is what
# points a server at it; leave that unset and the real providers are
# used, which is why production needs no flag to stay safe.
mock-oauth:
    cargo run -p mock-oauth

# The full demo: mock providers and a freshly seeded auth server.
#
# The mock runs in the background and is killed when this exits, so
# Ctrl-C leaves nothing holding port 4040.
auth-demo:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p mock-oauth -p auth-server
    cargo run -p mock-oauth &
    mock=$!
    trap 'kill "$mock" 2>/dev/null || true' EXIT
    just auth-dev

# The same, but on disk, so it survives a restart.
#   just auth-dev-persistent ./auth-local.db
auth-dev-persistent DB="./auth-local.db":
    cargo xtask dev auth --port 8080 --db {{DB}} --seed

# Pull a sanitised copy of a running server's database.
#
# Needs an administrator's session token in AUTH_ADMIN_TOKEN — read from
# the environment, not passed as an argument, so it stays out of shell
# history and the process table.
#
# The scrubbing happens on the SERVER: password hashes, sessions, OAuth
# tokens, API keys, passkeys and two-factor secrets never leave the
# machine that holds them. What arrives is the organization graph.
#
#   AUTH_ADMIN_TOKEN=… just auth-mirror https://auth.fasttrackstudio.app
auth-mirror FROM OUT="auth-snapshot.json":
    cargo run -p xtask -- auth-mirror --from {{FROM}} --out {{OUT}}

# As above, with every address replaced by user-<id>@local.invalid.
auth-mirror-redacted FROM OUT="auth-snapshot.json":
    cargo run -p xtask -- auth-mirror --from {{FROM}} --out {{OUT}} --redact-emails

# Run a local server on a snapshot taken by `auth-mirror`.
#
# Every imported account gets the published development password, so you
# can sign in as anybody — which is also why the import refuses to run
# against anything but a local database.
#
#   just auth-replay auth-snapshot.json
auth-replay SNAPSHOT="auth-snapshot.json" DB="./auth-replay.db":
    #!/usr/bin/env bash
    set -euo pipefail
    # A snapshot import wants an empty database; starting from a stale
    # one fails with "already holds N users", which is correct but
    # unhelpful as the answer to "run this again".
    rm -f "{{DB}}"
    exec cargo xtask dev auth --port 8080 --db "{{DB}}" --import "{{SNAPSHOT}}"

# Browser tests for the auth pages, against a server this starts itself.
#
# First run downloads a browser and builds the server. On NixOS the
# downloaded browser cannot find libglib, so point it at a system one:
#   nix-shell -p chromium --run 'PLAYWRIGHT_CHROMIUM_PATH=$(which chromium) just auth-e2e'
auth-e2e:
    cd apps/auth-server/e2e && npm install && npx playwright test

# ── The whole stack on your machine (`devenv up`, #541) ─────────────
# docs/content/guide/local-stack.md. Every recipe here is `cargo xtask dev`.

# What is running, which fill and why, the snapshot's age, the URLs to
# open and the development password.
dev-status:
    cargo xtask dev status

# Remove all of the stack's state (the mirror, the databases, the node's
# data, the app's folder and scratch vault). Stop `devenv up` first.
dev-reset:
    cargo xtask dev reset

# Mirror production again (needs AUTH_ADMIN_TOKEN or AUTH_ADMIN_TOKEN_FILE);
# the next start loads it. `--real-emails` keeps real addresses;
# `--vault <name>` is not available yet (the docs say why).
#   just dev-refresh
#   just dev-refresh --real-emails
dev-refresh *ARGS:
    cargo xtask dev refresh {{ARGS}}

# The demo seed from the next `devenv up` on, even on a credentialed
# machine (`just dev-refresh` undoes it).
dev-seed:
    cargo xtask dev seed

# The stack's guard alone: what every process would be given, checked.
dev-guard:
    cargo xtask dev guard

# Its own folder (~/.config/architect); its first run asks for a vault.
# ARGS go to the app: `just app actions list` asks the running app. Runs
# in the dev shell (its libraries) unless you are already in it.
# The Architect app on the desktop (apps/desktop), as a person runs it.
app *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "${DEVENV_ROOT:-}" ]; then
        exec cargo run -q -p architect-desktop -- {{ARGS}}
    fi
    exec devenv shell -- cargo run -q -p architect-desktop -- {{ARGS}}

# The lab's old name for `just app`, for now.
alias lab := app

# Inbox, Tasks, Schedule, the Board and the Clicker, with a folder of its
# own (~/.config/architect-sandbox). Its vault is a scratch copy of
# apps/ui/demo-vault under devenv's state, seeded when missing
# (ARCHITECT_VAULT names another folder). ARGS go to the sandbox:
# `just sandbox actions list`.
# The dev sandbox (apps/sandbox): the app on made-up data.
sandbox *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    export ARCHITECT_VAULT="${ARCHITECT_VAULT:-${DEVENV_STATE:-$PWD/.devenv/state}/demo-vault}"
    export ARCHITECT_VAULT_SEED="${ARCHITECT_VAULT_SEED:-$PWD/apps/ui/demo-vault}"
    if [ -n "${DEVENV_ROOT:-}" ]; then
        exec cargo run -q -p architect-sandbox -- {{ARGS}}
    fi
    exec devenv shell -- cargo run -q -p architect-sandbox -- {{ARGS}}

# Every edit a run made is thrown away.
# The scratch vault (`just sandbox`, `devenv up`), seeded again from apps/ui/demo-vault.
vault-reset:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="${ARCHITECT_VAULT:-${DEVENV_STATE:-$PWD/.devenv/state}/demo-vault}"
    rm -rf "$dir"
    mkdir -p "$(dirname "$dir")"
    cp -r apps/ui/demo-vault "$dir"
    rm -f "$dir/.empty-folders"
    while IFS= read -r folder; do
        [ -n "$folder" ] && mkdir -p "$dir/$folder"
    done < apps/ui/demo-vault/.empty-folders
    echo "vault-reset: $dir"

alias lab-vault-reset := vault-reset


# The web front end (#431): the site (apps/site) at /, the docs at /docs/
# and the web app at /app/, one static bundle (scripts/web-bundle.sh) into
# target/web-bundle, served by static-web-server with the image's own
# config (apps/web/sws.toml) on PORT (0: build only). 8080 is a redirect
# the issuer knows (`http://localhost:8080/auth/callback`), so sign-in
# works locally against the real issuer.
web PORT="8080":
    #!/usr/bin/env bash
    set -euo pipefail
    bash scripts/web-bundle.sh target/web-bundle
    if [ "{{PORT}}" != "0" ]; then
        echo "serving http://localhost:{{PORT}}"
        exec nix run nixpkgs#static-web-server -- --config-file apps/web/sws.toml --root target/web-bundle --host 127.0.0.1 --port {{PORT}}
    fi

# The web front end in a real browser (apps/web/test/web.test.mjs): the
# site's pages, the docs, the app's sign-in redirect (caught before it
# leaves) and its callbacks, and the node's paths answering 404 here; in
# a headless Firefox (FIREFOX=… names one).
web-test PORT="18080":
    #!/usr/bin/env bash
    set -euo pipefail
    just web 0
    sws="$(nix build --no-link --print-out-paths nixpkgs#static-web-server)/bin/static-web-server"
    "$sws" --config-file apps/web/sws.toml --root target/web-bundle --host 127.0.0.1 --port {{PORT}} >/dev/null 2>&1 &
    pid=$!
    trap 'kill $pid' EXIT
    for _ in $(seq 1 50); do curl -s -o /dev/null "http://127.0.0.1:{{PORT}}/" && break; sleep 0.1; done
    ARCHITECT_WEB_ORIGIN="http://127.0.0.1:{{PORT}}" node --test apps/web/test/web.test.mjs

# The app on the org's vaults, signed in, in a real browser
# (apps/web/test/org-vaults.test.mjs): read-only until a note is live, the
# navigator changing nothing, a new note never emptying another, the lane
# dialled again with a renewed token, Sign out revoking the sign-in.
# Against the running local stack (`just dev`); WEB is its web app.
web-org-test WEB="http://127.0.0.1:7413":
    ARCHITECT_ORG_WEB="{{WEB}}" ARCHITECT_ORG_VAULT="${DEVENV_STATE:-$PWD/.devenv/state}/stack/node/data/vaults/demo" env -u LD_LIBRARY_PATH node --test --test-concurrency=1 apps/web/test/org-vaults.test.mjs

# A share link's player in a real browser (#511): the headless node
# (`architect-desktop serve`, built without the window) on a vault of its
# own with a link share of a two-song gig, in a headless Firefox
# (FIREFOX=… names one): the lyric line follows playback, a click seeks,
# and the queue goes on to the next song's page.
# An island on a share's now-playing view in a real browser (studio#445): the
# sandbox's WASI counter built for wasm32-wasip2 and by build-web, a node with it
# linked in (apps/desktop/examples/island_node.rs), Firefox headless.
share-island-test:
    devenv --profile wasi shell -- bash -c 'cargo build -q -p architect-sandbox-wasi-counter --target wasm32-wasip2 && (cd features/extensions/web && npm ci --no-audit --no-fund >/dev/null) && node features/extensions/web/cli/cli.mjs build-web target/wasm32-wasip2/debug/architect_sandbox_wasi_counter.wasm target/island-counter --manifest apps/sandbox/wasi-counter/extension.toml >/dev/null'
    devenv shell -- cargo build -q -p architect-desktop --no-default-features --features vox --example island_node
    ARCHITECT_ISLAND_NODE="$PWD/target/debug/examples/island_node" ARCHITECT_ISLAND_FOLDER="$PWD/target/island-counter" env -u LD_LIBRARY_PATH node --test apps/web/test/share-island.test.mjs

# An island's web view against a hostile bundle in a real browser
# (architect#608): nothing reaches off the node, a frame that leaves its
# document is taken down, WebRTC only when declared and granted.
# DRIVER=safari (on a Mac: `safaridriver --enable` once, Safari quit) runs
# it in Safari through safaridriver, started and stopped by the suite on a
# free port; DRIVER=gecko runs that same classic-WebDriver path (no script
# evaluated in the page) in Firefox through geckodriver
# (`just share-webview-test-classic`); the default is Firefox over BiDi.
# Each test prints PASS or FAIL with the page state it last read.
share-webview-test:
    devenv shell -- cargo build -q -p architect-desktop --no-default-features --features vox --example island_node
    ARCHITECT_ISLAND_NODE="$PWD/target/debug/examples/island_node" env -u LD_LIBRARY_PATH node --test apps/web/test/share-webview.test.mjs

# The web app's extension frames (#495) against hostile bundles in a real
# browser: the release bundle (scripts/web-bundle.sh) behind a front that
# sends `/_ext/` to a node with an island (island_node), Firefox headless
# over BiDi, kept off the audio graph (`unshare -r -m`).
app-ext-host-test:
    devenv shell -- cargo build -q -p architect-desktop --no-default-features --features vox --example island_node
    devenv shell -- bash scripts/web-bundle.sh target/web-bundle
    ARCHITECT_ISLAND_NODE="$PWD/target/debug/examples/island_node" ARCHITECT_WEB_BUNDLE="$PWD/target/web-bundle" env -u LD_LIBRARY_PATH node --test --test-concurrency=1 apps/web/test/app-ext-host.test.mjs

# What a share link's guest's view reads in the web app (#648, #669): the
# release bundle behind a front sending `/_ext/` and `/s/` to a node that
# shares a gig with an island reading setlists, Firefox headless over
# BiDi off the audio graph, at 390 px; `SHOTS=<dir>` keeps screenshots.
app-guest-content-test:
    devenv shell -- cargo build -q -p architect-desktop --no-default-features --features vox --example island_node
    devenv shell -- bash scripts/web-bundle.sh target/web-bundle
    ARCHITECT_ISLAND_NODE="$PWD/target/debug/examples/island_node" ARCHITECT_WEB_BUNDLE="$PWD/target/web-bundle" env -u LD_LIBRARY_PATH node --test --test-concurrency=1 apps/web/test/app-guest-content.test.mjs

# The web app's bottom slots as favourites (#607) at 390 px: the default
# set, a note type's declared set from the node's `/_ext/favourites.json`,
# a tab opening its island view; `ARCHITECT_SHOTS=<dir>` keeps screenshots.
app-favourites-test:
    devenv shell -- cargo build -q -p architect-desktop --no-default-features --features vox --example island_node
    devenv shell -- bash scripts/web-bundle.sh target/web-bundle
    ARCHITECT_ISLAND_NODE="$PWD/target/debug/examples/island_node" ARCHITECT_WEB_BUNDLE="$PWD/target/web-bundle" env -u LD_LIBRARY_PATH node --test --test-concurrency=1 apps/web/test/app-favourites.test.mjs

# The suite on the classic-WebDriver path Safari takes, in Firefox
# (geckodriver from nixpkgs): what keeps that path from rotting without a Mac.
share-webview-test-classic:
    DRIVER=gecko GECKODRIVER="$(nix build --no-link --print-out-paths "$(sed -n 's/^ *url: \(github:NixOS\/nixpkgs\/[0-9a-f]*\)$/\1/p' devenv.yaml | head -1)#geckodriver")/bin/geckodriver" just share-webview-test

# A share page on a phone, measured in Firefox (one title at the top, the
# list's gutters, the card above the tab bar): apps/web/test/share-layout.test.mjs.
share-layout-test:
    devenv shell -- cargo build -q -p architect-desktop --no-default-features --features vox --example island_node
    ARCHITECT_ISLAND_NODE="$PWD/target/debug/examples/island_node" env -u LD_LIBRARY_PATH node --test apps/web/test/share-layout.test.mjs

# The site's /download (apps/site): its link logic (bun test), then the
# built page in a headless Firefox with GitHub's API stubbed: every file
# there, the Mac build missing, the API down, rate limited
# (apps/site/test/download.browser.mjs; SCREENSHOTS=<dir> keeps the shots).
site-download-test:
    cd apps/site && bun install --frozen-lockfile && bun run test && bun run build
    cd apps/site && env -u LD_LIBRARY_PATH node --test test/download.browser.mjs

share-player-test:
    devenv shell -- cargo build -q -p architect-desktop --no-default-features --features vox --bin architect-desktop
    devenv shell -- cargo build -q -p architect-cli --bin architect
    ARCHITECT_DESKTOP="$PWD/target/debug/architect-desktop" ARCHITECT_CLI="$PWD/target/debug/architect" env -u LD_LIBRARY_PATH node --test apps/web/test/share-player.test.mjs apps/web/test/share-playlist.test.mjs apps/web/test/site-now-playing.test.mjs

# The sandbox's browser demo (apps/sandbox/web): the WASI Counter component built for
# wasm32-wasip2, transpiled by jco (features/extensions/web), and the page
# built for wasm32-unknown-unknown, into target/sandbox-web; then served on
# PORT (0: build only). Needs node (the `wasi` dev profile has it) and
# wasm-bindgen-cli at Cargo.lock's version (installed into target/tools
# when missing).
sandbox-web PORT="8090":
    #!/usr/bin/env bash
    set -euo pipefail
    out=target/sandbox-web
    mkdir -p "$out"
    devenv --profile wasi shell -- cargo build -q -p architect-sandbox-wasi-counter --target wasm32-wasip2
    (cd features/extensions/web && [ -d node_modules ] || npm ci --silent)
    node features/extensions/web/transpile.mjs target/wasm32-wasip2/debug/architect_sandbox_wasi_counter.wasm "$out/wasi-counter" >/dev/null
    cargo build -q -p architect-sandbox-web --target wasm32-unknown-unknown
    want=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/[^0-9.]/,""); print; exit}' Cargo.lock)
    bindgen=target/tools/bin/wasm-bindgen
    if ! "$bindgen" --version 2>/dev/null | grep -q "$want"; then
        cargo install -q wasm-bindgen-cli --version "$want" --locked --root target/tools
    fi
    "$bindgen" --target web --no-typescript --out-dir "$out" target/wasm32-unknown-unknown/debug/architect_sandbox_web.wasm
    cp apps/sandbox/web/index.html apps/sandbox/web/start.js apps/sandbox/web/hello.txt "$out/"
    echo "sandbox-web: $out"
    if [ "{{PORT}}" != "0" ]; then
        echo "serving http://localhost:{{PORT}}"
        python3 -m http.server -d "$out" -b 127.0.0.1 {{PORT}}
    fi

# The sandbox's browser demo in a real browser (apps/sandbox/web/test): the Board's page file,
# a runaway page and a page's reach, in a headless Chrome of its own
# (CHROME=… names one; screenshots in target/web-shots).
sandbox-web-test:
    just sandbox-web 0
    node --test apps/sandbox/web/test/pages.test.mjs

alias lab-web := sandbox-web
alias lab-web-test := sandbox-web-test
