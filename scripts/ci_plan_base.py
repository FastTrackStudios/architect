#!/usr/bin/env python3
"""Pick the commit a push to main's test plan diffs from.

Usage: ci_plan_base.py HEAD_SHA  -> prints the base SHA, or nothing

The base is the newest strict ancestor of HEAD whose `checks` run, for a
push to main, completed successfully; not the previous push
(`github.event.before`). Main's runs cancel each other (checks.yml's
concurrency), so the commit before this one may never have run, or may
have gone red. Diffing from the last green commit makes this run cover
every commit since, cancelled and red ones included.

Prints nothing on stdout (and says why on stderr) when no such run is
among the newest MAX_RUNS green ones, when the API fails, or when none of
them is an ancestor (after a force-push): the caller then runs every test.
Always exits 0.

Reads GITHUB_REPOSITORY, GITHUB_TOKEN and GITHUB_API_URL; the token needs
`actions: read`. Tests: `python3 -m unittest discover -s scripts`.
"""
import json
import os
import subprocess
import sys
import urllib.request
from typing import Callable, Iterable, Optional

WORKFLOW = "checks.yml"
BRANCH = "main"
EVENT = "push"
# Green runs looked at, newest first: one page of the API, more than a day
# of merges. Past it a full run is cheaper than guessing.
MAX_RUNS = 100


def pick_base(
    runs: Iterable[dict], is_ancestor: Callable[[str], bool], head: str
) -> Optional[str]:
    """The `head_sha` of the newest successful push-to-main run that is a
    strict ancestor of `head`, or None.

    `runs` is the Actions API's `workflow_runs`, newest first. Runs that
    are not a successful push to main are skipped (a tag's, a dispatch's, a
    red or cancelled one), and so is `head` itself: a re-run of this commit
    tests its whole diff again. `is_ancestor(sha)` answers for `head`; a
    commit it doesn't know is not an ancestor.
    """
    seen = set()
    for run in runs:
        if (
            run.get("conclusion") != "success"
            or run.get("head_branch") != BRANCH
            or run.get("event") != EVENT
        ):
            continue
        sha = run.get("head_sha")
        if not isinstance(sha, str) or not sha or sha == head or sha in seen:
            continue
        seen.add(sha)
        if is_ancestor(sha):
            return sha
    return None


def green_runs() -> list:
    """The newest MAX_RUNS successful `checks` push runs on main. Raises on
    any API failure."""
    api = os.environ.get("GITHUB_API_URL", "https://api.github.com")
    repo = os.environ["GITHUB_REPOSITORY"]
    url = (
        f"{api}/repos/{repo}/actions/workflows/{WORKFLOW}/runs"
        f"?branch={BRANCH}&event={EVENT}&status=success&per_page={MAX_RUNS}"
    )
    req = urllib.request.Request(
        url,
        headers={
            "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        body = json.load(resp)
    runs = body.get("workflow_runs")
    if not isinstance(runs, list):
        raise ValueError("response has no workflow_runs")
    return runs


def git_is_ancestor(head: str) -> Callable[[str], bool]:
    def check(sha: str) -> bool:
        r = subprocess.run(
            ["git", "merge-base", "--is-ancestor", sha, head],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return r.returncode == 0

    return check


def main(argv=None) -> None:
    head = (argv or sys.argv)[1]
    try:
        runs = green_runs()
    except Exception as e:  # noqa: BLE001 - any API failure fails open
        print(f"base: runs API failed ({e})", file=sys.stderr)
        return
    base = pick_base(runs, git_is_ancestor(head), head)
    if base is None:
        print(
            f"base: none of the newest {len(runs)} green {WORKFLOW} pushes to "
            f"{BRANCH} is an ancestor of {head}",
            file=sys.stderr,
        )
        return
    print(f"base: {base}, the newest green ancestor on {BRANCH}", file=sys.stderr)
    print(base)


if __name__ == "__main__":
    main()
