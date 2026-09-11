# architect — recipes
# Run commands: just <recipe-name>

default:
    @just --list

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

check:
    cargo check --workspace --all-targets

test:
    cargo test --workspace

lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features

# Release tags must order the commits the way their numbers do — six
# repos pin this one by tag, and that is the only question they ask.
tags:
    cargo run -p xtask -- tags --verbose

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
auth-dev:
    AUTH_DEV_SEED=1 \
    AUTH_DATABASE_URL="sqlite::memory:" \
    AUTH_SECRET="a-secret-at-least-32-bytes-long!!" \
    AUTH_BASE_URL="http://localhost:8080" \
    AUTH_SOCIAL_MOCK_URL="http://localhost:4040" \
    AUTH_GITHUB_CLIENT_ID="mock-github" \
    AUTH_GITHUB_CLIENT_SECRET="mock-github-secret" \
    AUTH_GOOGLE_CLIENT_ID="mock-google" \
    AUTH_GOOGLE_CLIENT_SECRET="mock-google-secret" \
    AUTH_TONE3000_CLIENT_ID="mock-tone3000" \
    cargo run -p auth-server

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
    AUTH_DEV_SEED=1 \
    AUTH_DATABASE_URL="sqlite://{{DB}}?mode=rwc" \
    AUTH_SECRET="a-secret-at-least-32-bytes-long!!" \
    AUTH_BASE_URL="http://localhost:8080" \
    cargo run -p auth-server

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
    AUTH_IMPORT_SNAPSHOT="{{SNAPSHOT}}" \
    AUTH_DATABASE_URL="sqlite://{{DB}}?mode=rwc" \
    AUTH_SECRET="a-secret-at-least-32-bytes-long!!" \
    AUTH_BASE_URL="http://localhost:8080" \
    cargo run -p auth-server

# Browser tests for the auth pages, against a server this starts itself.
#
# First run downloads a browser and builds the server. On NixOS the
# downloaded browser cannot find libglib, so point it at a system one:
#   nix-shell -p chromium --run 'PLAYWRIGHT_CHROMIUM_PATH=$(which chromium) just auth-e2e'
auth-e2e:
    cd apps/auth-server/e2e && npm install && npx playwright test
