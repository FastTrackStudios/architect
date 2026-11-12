#!/usr/bin/env python3
"""Fail when a workflow mints a GitHub App token wider than its hop needs.

The fts-forge App is installed org-wide with broad permissions, so every
token is narrowed when it is minted (actions/create-github-app-token).
For every `.github/workflows/*.yml` this checks that each mint:

  - is listed in ALLOWED for its workflow and job;
  - names exactly ONE repository (`repositories:`), the listed one;
  - asks only for the listed `permission-*` (never workflows,
    administration or organization-*), and asks for at least one;
  - reads the key from the listed secret: a bot job's own name that
    exists in its environment only, never FORGE_APP_PRIVATE_KEY (an org
    secret and a repo secret carry that name, and a job outside its
    environment would silently fall back to them);
  - runs in the listed `environment:`;

and that no job exports a token as an output or writes one to
$GITHUB_OUTPUT / $GITHUB_ENV / $GITHUB_STEP_SUMMARY.

A LEGACY entry is an existing mint that predates the release bots and
still reads FORGE_APP_PRIVATE_KEY (the owner moves those keys; see the
release-chain design, infra#3). It is reported on every run, never silently
allowed to grow. Run by PR checks (`just check-token-mints`); `--self-test`
checks that a wide mint fails.
"""
from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# workflow -> job -> rule
ALLOWED: dict[str, dict[str, dict]] = {
    "release.yml": {
        # start the downstream pin bumps: one token per repository, each
        # `actions: write` only
        "dispatch": dict(repos=("keyflow", "studio", "infra"), perms={"actions": "write"},
                         secret="RELEASE_DISPATCH_APP_KEY", environment="release-dispatch"),
    },
}

FORBIDDEN = re.compile(r"^(workflows|administration|organization.*|members|secrets|environments)$")
MINT = re.compile(r"^\s*-?\s*uses:\s*actions/create-github-app-token@")


def jobs(text: str) -> dict[str, tuple[int, int]]:
    """job id -> (first line, end line), 0-based, for the top-level `jobs:`."""
    lines = text.splitlines()
    out: dict[str, tuple[int, int]] = {}
    start = next((i for i, l in enumerate(lines) if re.match(r"^jobs:\s*$", l)), None)
    if start is None:
        return out
    cur, first = None, 0
    for i in range(start + 1, len(lines)):
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", lines[i])
        if m:
            if cur:
                out[cur] = (first, i)
            cur, first = m.group(1), i
        elif lines[i] and not lines[i].startswith(" ") and not lines[i].startswith("#"):
            break
    if cur:
        out[cur] = (first, len(lines))
    return out


def step_keys(lines: list[str], i: int) -> dict[str, str]:
    """The `with:` (and sibling) keys of the step whose `uses:` is line i."""
    step_indent = None
    for back in range(i, max(i - 8, -1), -1):
        if lines[back].lstrip().startswith("- "):
            step_indent = len(lines[back]) - len(lines[back].lstrip())
            break
    if step_indent is None:
        step_indent = len(lines[i]) - len(lines[i].lstrip()) - 2
    keys: dict[str, str] = {}
    for back in range(i, -1, -1):  # keys above `uses:` in the same step
        line = lines[back]
        kv = re.match(r"^\s*-?\s*([A-Za-z0-9_-]+):\s*(.*?)\s*$", line)
        if kv:
            keys.setdefault(kv.group(1), kv.group(2).strip("'\""))
        if line.lstrip().startswith("- "):
            break
    for nxt in lines[i + 1:]:
        if not nxt.strip():
            continue
        if len(nxt) - len(nxt.lstrip()) <= step_indent:
            break
        kv = re.match(r"^\s*([A-Za-z0-9_-]+):\s*(.*?)\s*$", nxt)
        if kv:
            keys[kv.group(1)] = kv.group(2).strip("'\"")
    return keys


def check(root: Path) -> tuple[list[str], list[str]]:
    bad, legacy = [], []
    for path in sorted((root / ".github" / "workflows").glob("*.y*ml")):
        text = path.read_text()
        name = path.name
        lines = text.splitlines()
        for m in re.finditer(r"^    outputs:\s*\n((?:      .*\n)+)", text, re.M):
            if ".outputs.token" in m.group(1):
                bad.append(f"{name}: a job exports a minted token as an output")
        for n, line in enumerate(lines, 1):
            if re.search(r"(outputs\.token|\$\{?GH_TOKEN|\$\{?TOKEN)\b.*>>\s*\"?\$\{?GITHUB_(OUTPUT|ENV|STEP_SUMMARY)", line):
                bad.append(f"{name}:{n}: a token written to $GITHUB_OUTPUT/ENV/STEP_SUMMARY")
        spans = jobs(text)
        for job, (first, end) in spans.items():
            env_m = next((re.match(r"^    environment:\s*(\S+)\s*$", lines[k]) for k in range(first, end)
                          if re.match(r"^    environment:", lines[k])), None)
            environment = env_m.group(1) if env_m else None
            for i in range(first, end):
                if not MINT.match(lines[i]):
                    continue
                where = f"{name}:{i + 1} (job {job})"
                rule = ALLOWED.get(name, {}).get(job)
                if rule is None:
                    bad.append(f"{where}: a token mint not in scripts/check-token-mints.py's ALLOWED")
                    continue
                keys = step_keys(lines, i)
                repos = tuple(r for r in re.split(r"[,\s]+", keys.get("repositories", "")) if r)
                # A bot mint names exactly one of its job's repositories (a job
                # that wakes three repos mints three tokens); a LEGACY one
                # names exactly its listed set.
                if rule.get("legacy"):
                    if repos != tuple(rule["repos"]):
                        bad.append(f"{where}: repositories must be {','.join(rule['repos'])}, is {','.join(repos) or 'missing'}")
                elif len(repos) != 1:
                    bad.append(f"{where}: a bot token is minted for exactly one repository, is {','.join(repos) or 'missing'}")
                elif repos[0] not in rule["repos"]:
                    bad.append(f"{where}: repository {repos[0]} is not one of {','.join(rule['repos'])}")
                asked = {k[len("permission-"):]: v for k, v in keys.items() if k.startswith("permission-")}
                if not asked:
                    bad.append(f"{where}: no permission-* (the token would get every permission the App has)")
                for perm, level in sorted(asked.items()):
                    if FORBIDDEN.match(perm) or perm not in rule["perms"]:
                        bad.append(f"{where}: permission-{perm} is not allowed here")
                    elif level != rule["perms"][perm] and not (rule["perms"][perm] == "write" and level == "read"):
                        bad.append(f"{where}: permission-{perm}: {level}, allowed {rule['perms'][perm]}")
                if keys.get("owner") not in ("FastTrackStudios", None):
                    bad.append(f"{where}: owner must be FastTrackStudios")
                key = re.search(r"secrets\.([A-Za-z0-9_]+)", keys.get("private-key", ""))
                if not key or key.group(1) != rule["secret"]:
                    bad.append(f"{where}: private-key must be secrets.{rule['secret']}")
                if environment != rule["environment"]:
                    bad.append(f"{where}: must run in environment {rule['environment']}, runs in {environment}")
                if rule.get("legacy"):
                    legacy.append(f"{where}: LEGACY, {rule['legacy']}")
        if "secrets.FORGE_APP_PRIVATE_KEY" in text:
            legit = {j for j, r in ALLOWED.get(name, {}).items() if r["secret"] == "FORGE_APP_PRIVATE_KEY"}
            for job, (first, end) in spans.items():
                if job not in legit and any("secrets.FORGE_APP_PRIVATE_KEY" in l for l in lines[first:end]):
                    bad.append(f"{name} (job {job}): reads FORGE_APP_PRIVATE_KEY (bots read their environment's own key)")
    return bad, legacy


def main(root: Path = ROOT) -> int:
    bad, legacy = check(root)
    for line in legacy:
        print(f"  {line}")
    if bad:
        print("token mints wider than their hop needs:")
        print("\n".join(f"  {b}" for b in bad))
        return 1
    print("ok: every bot token is minted for one repository, with its listed permissions, in its environment")
    return 0


WIDE = """jobs:
  {job}:
    outputs:
      t: ${{{{ steps.forge.outputs.token }}}}
    steps:
      - uses: actions/create-github-app-token@0000000000000000000000000000000000000000
        id: forge
        with:
          private-key: ${{{{ secrets.FORGE_APP_PRIVATE_KEY }}}}
          repositories: a,b
          permission-workflows: write
      - run: echo "$GH_TOKEN" >> "$GITHUB_OUTPUT"
"""


def self_test() -> int:
    """A wide mint, two repos, the shared key, no environment must all fail."""
    workflow, jobs_ = next((w, j) for w, j in ALLOWED.items() if any(not r.get("legacy") for r in j.values()))
    job = next(j for j, r in jobs_.items() if not r.get("legacy"))
    with tempfile.TemporaryDirectory() as d:
        wf = Path(d) / ".github" / "workflows"
        wf.mkdir(parents=True)
        (wf / workflow).write_text(WIDE.format(job=job))
        bad, _ = check(Path(d))
        need = ("exactly one repository", "permission-workflows", "private-key must be",
                "must run in environment", "as an output", "written to")
        missing = [n for n in need if not any(n in b for b in bad)]
        if missing:
            print(f"self-test FAILED: a wide mint was not caught for: {missing}")
            return 1
    print("self-test ok: a wide mint fails on every rule")
    return 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv else main())
