#!/usr/bin/env python3
"""Whether a PR touches Rust, and which feature builds it reaches.

Usage: pr-scope.py BASE [HEAD] [--all] [--cmds FILE]
  prints KEY=VALUE lines for $GITHUB_OUTPUT; with --cmds, writes the
  feature builds (one `cargo clippy` per line) to FILE; --all reaches
  everything whatever the diff (a push to main, warming the cache).

The PR check (.github/workflows/pr.yml) lints the whole workspace, exactly
as main does, whenever a PR changes a crate: a hosted runner with main's
cache does that in about three minutes, and a `-p` subset resolves
features differently from the workspace, so it can fail on code main
lints clean or pass code main fails. What this script narrows is the rest:

- no clippy at all when no crate is touched (docs, skills, nix, the site);
- each feature-tests.toml pass, and each EXTRA_BUILDS build, only when a
  crate it builds is reached: every member owning a changed file (or
  `include_str!`/`include_bytes!`-ing one), plus every member depending on
  one, normal, build or dev, transitively.

Not `cargo rail plan`: without recorded build evidence (`cargo rail plan
evidence`) rail widens every plan whose diff holds a file it cannot prove
no build script reads, which includes every `.changes/*.md`, so on this
repo it plans the whole workspace for every PR.

Outputs:
  rust=true|false   lint the workspace
  reached=          the members reached, space-separated, or (workspace)
  builds=N          how many feature builds --cmds wrote

Anything this script cannot read exactly reaches everything, never
narrows on a guess.
"""
import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import NoReturn

ROOT = Path(__file__).resolve().parent.parent

# Inputs that change how every crate builds or lints, or what this check
# itself does: any change here reaches every member.
WORKSPACE_INPUTS = (
    "Cargo.toml",
    "Cargo.lock",
    ".cargo/",
    "rust-toolchain.toml",
    "clippy.toml",
    ".clippy.toml",
    "rustfmt.toml",
    ".rustfmt.toml",
    "feature-tests.toml",
    ".config/hakari.toml",
    ".github/workflows/pr.yml",
    ".github/actions/",
    "scripts/pr-scope.py",
)

# Builds main runs that are neither the workspace lint nor a
# feature-tests.toml pass, so a feature combination they compile is only
# checked here. Each runs when one of `packages` is reached.
EXTRA_BUILDS = [
    # The headless node: `xtask/tests/dev_stack.rs` builds it inside the
    # dev-stack pass under main's `CARGO_BUILD_WARNINGS=deny` (#549 went red
    # on dead code that only exists without `ui`), and release.yml ships it.
    {
        "name": "headless node",
        "packages": ["architect-desktop"],
        "args": "-p architect-desktop --no-default-features --features vox --bins",
    },
    {
        "name": "dev-stack servers",
        "packages": ["auth-server", "mock-oauth"],
        "args": "-p auth-server -p mock-oauth --bins",
    },
]

INCLUDE = re.compile(r'include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)')


def emit(**kv):
    for k, v in kv.items():
        print(f"{k}={v}")


def everything(reason, members, cmds) -> NoReturn:
    print(f"scope: {reason} -> every member and feature build", file=sys.stderr)
    builds = feature_builds(set(members), cmds)
    emit(rust="true", reached="(workspace)", builds=builds)
    sys.exit(0)


def metadata():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def changed_files(base, head):
    out = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", base, head],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    return [line for line in out.splitlines() if line]


def owners(members):
    """Member directories, deepest first, so a nested package wins."""
    dirs = []
    for name, manifest in members.items():
        d = Path(manifest).parent.relative_to(ROOT).as_posix()
        dirs.append((d, name))
    dirs.sort(key=lambda t: -len(t[0]))
    return dirs


def includes(dirs):
    """Repo path -> members that `include_str!`/`include_bytes!` it."""
    out = {}
    for d, name in dirs:
        for rs in (ROOT / d).rglob("*.rs"):
            rel = rs.relative_to(ROOT).as_posix()
            if "/target/" in f"/{rel}" or "/node_modules/" in rel:
                continue
            try:
                text = rs.read_text(errors="replace")
            except OSError:
                continue
            for m in INCLUDE.finditer(text):
                p = os.path.normpath((rs.parent / m.group(1)).as_posix())
                try:
                    key = Path(p).relative_to(ROOT).as_posix()
                except ValueError:
                    continue
                out.setdefault(key, set()).add(name)
    return out


def owner_of(path, dirs):
    for d, name in dirs:
        if path == d or path.startswith(d + "/") or d == ".":
            return name
    return None


def reverse_closure(seeds, meta, members):
    """Seeds plus every member depending on one of them, transitively."""
    name = {p["id"]: p["name"] for p in meta["packages"]}
    rdeps = {m: set() for m in members}
    for node in meta["resolve"]["nodes"]:
        if name[node["id"]] not in members:
            continue
        for dep in node["deps"]:
            if name[dep["pkg"]] in members:
                rdeps[name[dep["pkg"]]].add(name[node["id"]])
    out, todo = set(seeds), list(seeds)
    while todo:
        for r in rdeps.get(todo.pop(), ()):
            if r not in out:
                out.add(r)
                todo.append(r)
    return out


def feature_builds(reached, cmds):
    """The feature-tests.toml passes and EXTRA_BUILDS a reached member is in.

    Main only compiles these (nextest, `cargo build` in a test), so a lint
    is stricter than it; but clippy shares the workspace lint's
    fingerprints, so a pass re-lints only the members its features change,
    where `cargo check` would re-check every member (96s against 2s for
    the keyflow-lab pass, measured), and every pass is clippy-clean.
    """
    lines = []
    passes = tomllib.loads((ROOT / "feature-tests.toml").read_text()).get("pass", [])
    for p in passes:
        if not reached & set(p["packages"]):
            continue
        # As main builds it (xtask/src/feature_tests.rs).
        sel = "--workspace" if p.get("workspace") else " ".join(f"-p {x}" for x in p["packages"])
        feats = ",".join(p.get("features", []))
        args = f"{sel} --features {feats}" if feats else sel
        # nextest builds every test target of the pass: lib, bins, tests.
        lines.append(f"# pass {p['name']}\ncargo clippy {args} --tests")
    for b in EXTRA_BUILDS:
        if reached & set(b["packages"]):
            lines.append(f"# {b['name']}\ncargo clippy {b['args']}")
    if cmds:
        Path(cmds).write_text("set -euo pipefail\n" + "\n".join(lines) + "\n")
    return len(lines)


def main():
    args = sys.argv[1:]
    cmds = None
    take_all = "--all" in args
    args = [a for a in args if a != "--all"]
    if "--cmds" in args:
        i = args.index("--cmds")
        cmds = args[i + 1]
        del args[i:i + 2]
    if not args:
        sys.exit(__doc__)
    base, head = args[0], (args[1] if len(args) > 1 else "HEAD")

    meta = metadata()
    members = {
        p["name"]: p["manifest_path"]
        for p in meta["packages"] if p["id"] in set(meta["workspace_members"])
    }
    if take_all:
        everything("--all", members, cmds)
    try:
        files = changed_files(base, head)
    except subprocess.CalledProcessError as e:
        everything(f"git diff failed ({e.stderr.strip()})", members, cmds)

    for f in files:
        if any(f == w or (w.endswith("/") and f.startswith(w)) for w in WORKSPACE_INPUTS):
            everything(f"workspace input changed ({f})", members, cmds)

    dirs = owners(members)
    included = includes(dirs)
    seeds = set()
    for f in files:
        o = owner_of(f, dirs)
        if o:
            seeds.add(o)
        seeds |= included.get(f, set())
    if "architect-workspace-hack" in seeds:
        everything("the workspace-hack changed", members, cmds)
    if not seeds:
        print(f"scope: {len(files)} changed file(s), none in a crate", file=sys.stderr)
        emit(rust="false", reached="", builds=feature_builds(set(), cmds))
        return

    reached = reverse_closure(seeds, meta, members)
    print(
        f"scope: changed {', '.join(sorted(seeds))}; with dependents "
        f"{len(reached)} of {len(members)} members",
        file=sys.stderr,
    )
    builds = feature_builds(reached, cmds)
    emit(rust="true", reached=" ".join(sorted(reached)), builds=builds)


if __name__ == "__main__":
    main()
