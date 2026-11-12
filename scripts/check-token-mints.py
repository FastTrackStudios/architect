#!/usr/bin/env python3
"""Fail when a workflow mints or reaches a GitHub App key wider than its hop needs.

The fts-forge App is installed org-wide with broad permissions, so every
token is narrowed when it is minted (actions/create-github-app-token), and
the key itself is reachable only where a mint needs it. This PARSES every
`.github/workflows/*.y*ml` and every composite action
(`.github/actions/**/action.y*ml`, its `runs.steps`), so formatting cannot
hide a mint, and fails when:

  - a mint (any `uses:` of actions/create-github-app-token, quoted or not,
    at any indentation) is not listed in ALLOWED for its workflow and job;
  - a mint names more or other than ONE listed repository, asks for a
    permission outside its list (never workflows, administration,
    organization-*, secrets, members, environments), or asks for none;
  - its private-key is not exactly `${{ secrets.<its key> }}`, or it does
    not run in its listed environment;
  - an App key secret (BOT_KEYS) appears ANYWHERE else: a `run:` script, an
    `env:`, a `with:` of another action (a curl or `gh api` mint);
  - `toJSON(secrets)` appears anywhere; a job passes `secrets: inherit`
    without being listed in INHERIT;
  - a job exports a minted token as an output, or a step writes one to
    $GITHUB_OUTPUT / $GITHUB_ENV / $GITHUB_STEP_SUMMARY / $GITHUB_PATH: the
    whole command that writes the file is read, i.e. its pipeline
    (`… | tee -a "$GITHUB_ENV"`), a `{ …; }` or `( … )` group redirected as
    one (one line or several), a heredoc, line continuations. A token is
    `outputs.token`, `secrets.*`, or a variable named *TOKEN*. (A token
    copied into a differently named variable first is beyond a lint: that
    is what the review of the workflows is for.)

A LEGACY entry is an existing mint that predates the release bots and
reads FORGE_APP_PRIVATE_KEY; it is printed on every run. `--self-test`
checks that each of these shapes fails. Needs PyYAML, or mikefarah's `yq`.
"""
from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# ---- per repo -------------------------------------------------------------
# workflow -> job -> rule (repos: the ONE-repo choices a mint may name).
# Architect mints NO App token: its release starts nothing downstream (the
# downstream bots poll; the dispatch job was dropped, owner 2026-11-12).
ALLOWED: dict[str, dict[str, dict]] = {}
# (workflow, job) pairs allowed `secrets: inherit`, with the reason
INHERIT: dict[tuple[str, str], str] = {}
# ---------------------------------------------------------------------------

BOT_KEYS = {"FORGE_APP_PRIVATE_KEY", "PIN_BUMP_APP_KEY", "RELEASE_DISPATCH_APP_KEY",
            "RELEASE_QUEUE_APP_KEY", "RELEASE_BUMP_APP_KEY"}
FORBIDDEN = re.compile(r"^(workflows|administration|organization.*|members|secrets|environments)$")
MINT = re.compile(r"^\s*['\"]?actions/create-github-app-token@")


def load(path: Path):
    text = path.read_text()
    try:
        import yaml  # type: ignore

        class Loader(yaml.SafeLoader):
            pass
        # `on:` must stay the key "on", not the boolean.
        Loader.yaml_implicit_resolvers = {k: [r for r in v if r[0] != "tag:yaml.org,2002:bool"]
                                          for k, v in Loader.yaml_implicit_resolvers.items()}
        return yaml.load(text, Loader=Loader)
    except ImportError:
        yq = shutil.which("yq")
        if not yq:
            sys.exit("check-token-mints: needs PyYAML or mikefarah's yq to parse the workflows")
        out = subprocess.run([yq, "-o=json", "eval", ".", str(path)], check=True,
                             capture_output=True, text=True).stdout
        return json.loads(out)


def strings(node, where=()):
    """Every string in a parsed document, with its key path."""
    if isinstance(node, dict):
        for k, v in node.items():
            yield from strings(v, where + (str(k),))
    elif isinstance(node, list):
        for i, v in enumerate(node):
            yield from strings(v, where + (str(i),))
    elif isinstance(node, str):
        yield where, node


TARGET = r"\"?\$\{?GITHUB_(?:OUTPUT|ENV|STEP_SUMMARY|PATH)\b"
WRITE = re.compile(r"(?:>>?|\btee\b(?:\s+-\w+)*)\s*" + TARGET)
TOKEN = re.compile(r"outputs\.token|secrets\.|\$\{?\w*TOKEN\w*")


def writes(script: str) -> list[str]:
    """The text of every command in `script` that writes a GITHUB_* file."""
    text = script.replace("\\\n", " ")
    found = []
    for m in WRITE.finditer(text):
        start, end = m.start(), m.end()
        # Back over a redirected group: `{ …; } >> F` or `( … ) >> F`.
        j = start - 1
        while j >= 0 and text[j] in " \t":
            j -= 1
        if j >= 0 and text[j] in "})":
            close, open_ = text[j], "{" if text[j] == "}" else "("
            depth, k = 0, j
            while k >= 0:
                if text[k] == close:
                    depth += 1
                elif text[k] == open_:
                    depth -= 1
                    if depth == 0:
                        break
                k -= 1
            start = max(k, 0)
        # Back to the start of the pipeline: the previous ; && || or newline
        # (not a `|`: `echo $T | tee -a F` is one command).
        k = start
        while k > 0 and text[k - 1] != "\n" and text[k - 1] != ";" \
                and text[k - 2:k] not in ("&&", "||"):
            k -= 1
        start = k
        # Forward to the end of the command, and through a heredoc's body.
        nl = text.find("\n", end)
        end = len(text) if nl < 0 else nl
        cmd = text[start:end]
        h = re.search(r"<<-?\s*['\"]?(\w+)", cmd)
        if h:
            term = re.compile(r"^\s*" + re.escape(h.group(1)) + r"\s*$", re.M)
            t = term.search(text, end)
            cmd = text[start:t.end() if t else len(text)]
        found.append(cmd)
    return found


def files(root: Path):
    """(name, parsed, steps per job) for every workflow and composite action."""
    for path in sorted((root / ".github" / "workflows").glob("*.y*ml")):
        doc = load(path) or {}
        yield path.name, doc, doc.get("jobs") or {}
    for path in sorted((root / ".github" / "actions").glob("**/action.y*ml")):
        doc = load(path) or {}
        runs = doc.get("runs") or {}
        yield str(path.relative_to(root)), doc, {"(composite)": {"steps": runs.get("steps") or []}}


def check(root: Path) -> tuple[list[str], list[str]]:
    bad, legacy = [], []
    for name, doc, jobs in files(root):
        mint_keys: set[tuple] = set()
        for job_id, job in jobs.items():
            if not isinstance(job, dict):
                continue
            env = job.get("environment")
            env = env.get("name") if isinstance(env, dict) else env
            if job.get("secrets") == "inherit" and (name, job_id) not in INHERIT:
                bad.append(f"{name} (job {job_id}): `secrets: inherit` not listed in INHERIT")
            for out_k, out_v in (job.get("outputs") or {}).items():
                if isinstance(out_v, str) and ".outputs.token" in out_v:
                    bad.append(f"{name} (job {job_id}): exports a minted token as output {out_k}")
            for i, step in enumerate(job.get("steps") or []):
                if not isinstance(step, dict):
                    continue
                run = step.get("run") or ""
                # The token inside the command that writes the file (a
                # `[ -n "$TOKEN" ]; then echo present=true >> …` is fine).
                for cmd in writes(run if isinstance(run, str) else ""):
                    if TOKEN.search(cmd):
                        bad.append(f"{name} (job {job_id}, step {i}): a token written to a GITHUB_* file: {cmd.strip()[:80]}")
                uses = step.get("uses") or ""
                if not (isinstance(uses, str) and MINT.match(uses)):
                    continue
                where = f"{name} (job {job_id}, step {i})"
                mint_keys.add(("jobs", job_id, "steps", str(i), "with", "private-key"))
                rule = ALLOWED.get(name, {}).get(job_id)
                if rule is None:
                    bad.append(f"{where}: a token mint not in ALLOWED")
                    continue
                w = {k: str(v) for k, v in (step.get("with") or {}).items()}
                repos = tuple(r for r in re.split(r"[,\s]+", w.get("repositories", "")) if r)
                if rule.get("legacy"):
                    if repos != tuple(rule["repos"]):
                        bad.append(f"{where}: repositories must be {','.join(rule['repos'])}, is {','.join(repos) or 'missing'}")
                    legacy.append(f"{where}: LEGACY, {rule['legacy']}")
                elif len(repos) != 1:
                    bad.append(f"{where}: a bot token is minted for exactly one repository, is {','.join(repos) or 'missing'}")
                elif repos[0] not in rule["repos"]:
                    bad.append(f"{where}: repository {repos[0]} is not one of {','.join(rule['repos'])}")
                asked = {k[len("permission-"):]: v for k, v in w.items() if k.startswith("permission-")}
                if not asked:
                    bad.append(f"{where}: no permission-* (the token would get every permission the App has)")
                for perm, level in sorted(asked.items()):
                    if FORBIDDEN.match(perm) or perm not in rule["perms"]:
                        bad.append(f"{where}: permission-{perm} is not allowed here")
                    elif level != rule["perms"][perm] and not (rule["perms"][perm] == "write" and level == "read"):
                        bad.append(f"{where}: permission-{perm}: {level}, allowed {rule['perms'][perm]}")
                if w.get("owner", "FastTrackStudios") != "FastTrackStudios":
                    bad.append(f"{where}: owner must be FastTrackStudios")
                if not re.fullmatch(r"\$\{\{\s*secrets\." + re.escape(rule["secret"]) + r"\s*\}\}", w.get("private-key", "").strip()):
                    bad.append(f"{where}: private-key must be exactly ${{{{ secrets.{rule['secret']} }}}}")
                if env != rule["environment"]:
                    bad.append(f"{where}: must run in environment {rule['environment']}, runs in {env}")
        # The keys anywhere else, and the whole secrets context.
        for keypath, s in strings(doc):
            if re.search(r"toJSON\(\s*secrets\s*\)", s):
                bad.append(f"{name} ({'.'.join(keypath)}): toJSON(secrets)")
            # Allowed besides a mint's private-key: exactly `${{ secrets.K != '' }}`
            # (the environment guard: a boolean, never the value).
            if re.fullmatch(r"\$\{\{\s*secrets\.[A-Za-z0-9_]+\s*!=\s*''\s*\}\}", s.strip()):
                continue
            for m in re.finditer(r"secrets\.([A-Za-z0-9_]+)", s):
                if m.group(1) in BOT_KEYS and keypath not in mint_keys:
                    bad.append(f"{name} ({'.'.join(keypath)}): secrets.{m.group(1)} outside a mint's private-key")
    return bad, legacy


def main(root: Path = ROOT) -> int:
    bad, legacy = check(root)
    for line in legacy:
        print(f"  {line}")
    if bad:
        print("App keys or tokens reach further than their hop needs:")
        print("\n".join(f"  {b}" for b in dict.fromkeys(bad)))
        return 1
    print("ok: every bot token is minted for one repository, with its listed permissions, in its environment, "
          "and no App key is reachable anywhere else")
    return 0


# The shapes the security review found the line-based check missed, and the
# ones it caught; each must fail.
CASES = {
    "unscoped mint in a 4-space-indented job": """
jobs:
    open:
        runs-on: ubuntu-latest
        steps:
            - uses: actions/create-github-app-token@0000000000000000000000000000000000000000
              with:
                  private-key: ${{ secrets.PIN_BUMP_APP_KEY }}
""",
    "quoted uses": """
jobs:
  open:
    steps:
      - uses: "actions/create-github-app-token@0000000000000000000000000000000000000000"
        with: {private-key: "${{ secrets.FORGE_APP_PRIVATE_KEY }}", repositories: "a,b", permission-workflows: write}
""",
    "a curl mint with the bot key in run": """
jobs:
  other:
    steps:
      - run: |
          curl -H "x: ${{ secrets.PIN_BUMP_APP_KEY }}" https://api.github.com/app/installations
""",
    "the bot key in env": """
jobs:
  other:
    env:
      K: ${{ secrets.RELEASE_QUEUE_APP_KEY }}
    steps:
      - run: gh api app/installations
""",
    "the key inside a longer expression": """
jobs:
  other:
    env:
      K: ${{ secrets.PIN_BUMP_APP_KEY != '' && secrets.PIN_BUMP_APP_KEY }}
    steps:
      - run: "true"
""",
    "toJSON(secrets)": """
jobs:
  other:
    steps:
      - run: echo '${{ toJSON(secrets) }}' > /dev/null
""",
    "secrets: inherit": """
jobs:
  other:
    uses: ./.github/workflows/x.yml
    secrets: inherit
""",
    "a brace group on one line": """
jobs:
  other:
    steps:
      - run: '{ echo "X=$GH_TOKEN"; } >> "$GITHUB_ENV"'
""",
    "a brace group over several lines": """
jobs:
  other:
    steps:
      - run: |
          {
            echo "A=1"
            echo "X=$GH_TOKEN"
          } >> "$GITHUB_ENV"
""",
    "a subshell": """
jobs:
  other:
    steps:
      - run: |
          ( echo "A=1"; printf 'X=%s\\n' "${{ steps.forge.outputs.token }}" ) >>$GITHUB_OUTPUT
""",
    "tee -a": """
jobs:
  other:
    steps:
      - run: echo "X=$GH_TOKEN" | tee -a "$GITHUB_ENV"
""",
    "through a pipe": """
jobs:
  other:
    steps:
      - run: printf %s "$FORGE_TOKEN" | base64 >> "${GITHUB_ENV}"
""",
    "a heredoc": """
jobs:
  other:
    steps:
      - run: |
          cat >> "$GITHUB_ENV" <<EOF
          X=${{ secrets.NPM_TOKEN }}
          EOF
""",
    "a line continuation": """
jobs:
  other:
    steps:
      - run: |
          echo "X=$GH_TOKEN" \\
            >> "$GITHUB_ENV"
""",
    "a composite action's mint": {".github/actions/x/action.yml": """
name: x
runs:
  using: composite
  steps:
    - uses: actions/create-github-app-token@0000000000000000000000000000000000000000
      with: {private-key: "${{ inputs.k }}", app-id: "1"}
"""},
    "a composite action writing a token": {".github/actions/y/action.yaml": """
name: y
runs:
  using: composite
  steps:
    - shell: bash
      run: '{ echo "T=$GH_TOKEN"; } >> "$GITHUB_ENV"'
"""},
    "an exported token": """
jobs:
  open:
    outputs:
      t: ${{ steps.forge.outputs.token }}
    steps:
      - run: echo "$GH_TOKEN" >> "$GITHUB_OUTPUT"
""",
}


# And shapes that must PASS (no token reaches the file).
GOOD = {
    "a presence check, then a write": """
jobs:
  other:
    steps:
      - run: |
          if [ -n "$REGISTRY_TOKEN" ]; then echo "present=true" >> "$GITHUB_OUTPUT"; fi
          [ -n "$GH_TOKEN" ] && echo ok=1 >> "$GITHUB_OUTPUT"
          { echo "a=1"; echo "b=2"; } >> "$GITHUB_OUTPUT"
          echo "### done" | tee -a "$GITHUB_STEP_SUMMARY"
          gh api x --jq .sha > "$RUNNER_TEMP/sha"
""",
    "a composite action with no token": {".github/actions/z/action.yml": """
name: z
runs:
  using: composite
  steps:
    - shell: bash
      run: echo "dir=$RUNNER_TEMP" >> "$GITHUB_OUTPUT"
"""},
}


def run_case(text) -> list[str]:
    with tempfile.TemporaryDirectory() as d:
        (Path(d) / ".github" / "workflows").mkdir(parents=True)
        if isinstance(text, dict):
            for rel, body in text.items():
                f = Path(d) / rel
                f.parent.mkdir(parents=True, exist_ok=True)
                f.write_text(body)
        else:
            (Path(d) / ".github" / "workflows" / "case.yml").write_text("name: case\non: push\n" + text)
        return check(Path(d))[0]


def self_test() -> int:
    missed = [case for case, text in CASES.items() if not run_case(text)]
    wrong = {case: bad for case, text in GOOD.items() if (bad := run_case(text))}
    if missed or wrong:
        print(f"self-test FAILED: not caught: {missed}; flagged though fine: {wrong}")
        return 1
    print(f"self-test ok: {len(CASES)} wide or leaking shapes all fail, {len(GOOD)} fine ones pass")
    return 0


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv else main())
