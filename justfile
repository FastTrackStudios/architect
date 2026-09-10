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
