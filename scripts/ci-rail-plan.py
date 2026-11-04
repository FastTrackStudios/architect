#!/usr/bin/env python3
"""Turn a `cargo rail plan --json` (contract v9) into the test job's filter.

Usage: ci-rail-plan.py PLAN.json  -> prints KEY=VALUE lines for $GITHUB_OUTPUT

Outputs:
  full=true|false   run every test (workspace scope, CI inputs changed, or
                    anything this script can't read exactly)
  tests=true|false  the test step has work
  test_filter=EXPR  nextest filterset for the affected packages ("" = all)

A filterset, not rail's `-p` list: building a subset of packages changes
Cargo's feature unification and leaves a second artifact set in the target
dir. The test build stays `--workspace`; only which tests RUN narrows.

Anything unexpected fails open (full=true), never narrow on a guess.
"""
import json
import sys
from typing import NoReturn

# Repository work declared in .config/rail.toml whose change means "run
# the full set".
FULL_RUN_WORK = {"ci-infra"}

# Every work id this script knows the meaning of: rail 0.32's built-ins and
# FULL_RUN_WORK. A required id outside it (a newer rail, a new rail.toml
# item) fails open rather than being silently ignored.
KNOWN_WORK = {
    "cargo.build", "cargo.clippy", "cargo.test", "cargo.doc", "cargo.doctest",
    "cargo.fmt", "cargo.package", "dependency-policy", "release.semver", "surface",
} | FULL_RUN_WORK

# The plan JSON shape this script reads (rail 0.32).
CONTRACT = 9


def emit(**kv):
    for k, v in kv.items():
        if isinstance(v, bool):
            v = str(v).lower()
        print(f"{k}={v}")


def full(reason) -> NoReturn:
    print(f"plan: {reason} -> full run", file=sys.stderr)
    emit(full=True, tests=True, test_filter="")
    sys.exit(0)


def packages(work):
    """Package names of a required Cargo work item, or None for workspace scope.

    From the typed `selection.packages[].name`, not `cargo_args`: when two
    packages share a name rail's `-p` spec becomes `name@version`, which a
    nextest `package(name@1.0.0)` never matches.
    """
    scope = work.get("scope") or {}
    sel = scope.get("selection") or {}
    if scope.get("kind") != "cargo" or sel.get("kind") == "workspace":
        return None
    pkgs = sel.get("packages")
    if not isinstance(pkgs, list) or not pkgs:
        return None
    names = {n for p in pkgs if isinstance(p, dict) and isinstance(n := p.get("name"), str) and n}
    return names or None


def main():
    try:
        plan = json.load(open(sys.argv[1]))
    except Exception as e:  # noqa: BLE001 - any read failure fails open
        full(f"unreadable plan ({e})")
    if plan.get("plan_contract_version") != CONTRACT:
        full(f"plan contract {plan.get('plan_contract_version')!r}, this script reads {CONTRACT}")
    if "work" not in plan or "required" not in plan:
        full("plan has no work/required")

    required = set(plan["required"])
    unknown = required - KNOWN_WORK
    if unknown:
        full(f"unknown required work ({', '.join(sorted(unknown))})")
    hit = required & FULL_RUN_WORK
    if hit:
        full(f"CI inputs changed ({', '.join(sorted(hit))})")

    if "cargo.test" not in required:
        print("plan: no tests affected", file=sys.stderr)
        emit(full=False, tests=False, test_filter="")
        return

    pk = packages(plan["work"]["cargo.test"])
    if pk is None:
        full("cargo.test has workspace scope")

    # `=`: an exact package name, never a glob or a substring.
    expr = " | ".join(f"package(={p})" for p in sorted(pk))
    emit(full=False, tests=True, test_filter=expr)
    print(f"plan: tests for {len(pk)} package(s): {', '.join(sorted(pk))}", file=sys.stderr)


if __name__ == "__main__":
    main()
