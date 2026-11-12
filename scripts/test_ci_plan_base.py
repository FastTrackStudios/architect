"""Tests for ci_plan_base.py: python3 -m unittest discover -s scripts"""
import contextlib
import io
import os
import unittest
from unittest import mock

import ci_plan_base
from ci_plan_base import pick_base


def run(sha, conclusion="success", branch="main", event="push"):
    return {"head_sha": sha, "conclusion": conclusion, "head_branch": branch, "event": event}


def anything(_sha):
    return True


class PickBase(unittest.TestCase):
    def test_newest_green_ancestor(self):
        # Newest first: c is green but not in HEAD's history, b is the
        # newest green ancestor, a is older.
        runs = [run("c"), run("b"), run("a")]
        self.assertEqual(pick_base(runs, {"a", "b"}.__contains__, "head"), "b")

    def test_skips_red_cancelled_other_branches_and_events(self):
        runs = [
            run("d", conclusion="failure"),
            run("c", conclusion="cancelled"),
            run("t", branch="v1.2.3"),
            run("s", event="schedule"),
            run("w", event="workflow_dispatch"),
            run("b"),
        ]
        self.assertEqual(pick_base(runs, anything, "head"), "b")

    def test_none_when_no_runs(self):
        self.assertIsNone(pick_base([], anything, "head"))

    def test_green_run_on_a_non_ancestor(self):
        # After a force-push: green runs exist, none in HEAD's history.
        self.assertIsNone(pick_base([run("x"), run("y")], lambda _s: False, "head"))

    def test_head_itself_is_not_a_base(self):
        # A re-run of a commit that already went green tests its diff again.
        self.assertEqual(pick_base([run("head"), run("a")], anything, "head"), "a")

    def test_malformed_runs_are_skipped(self):
        runs = [{}, {"conclusion": "success", "head_branch": "main", "event": "push"}, run("a")]
        self.assertEqual(pick_base(runs, anything, "head"), "a")


class Main(unittest.TestCase):
    def stdout_of_main(self):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            ci_plan_base.main(["ci_plan_base.py", "head"])
        return out.getvalue(), err.getvalue()

    def test_api_error_prints_no_base(self):
        with mock.patch.object(ci_plan_base, "green_runs", side_effect=OSError("boom")):
            out, err = self.stdout_of_main()
        self.assertEqual(out, "")
        self.assertIn("runs API failed", err)

    def test_missing_token_is_an_api_error(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            out, err = self.stdout_of_main()
        self.assertEqual(out, "")
        self.assertIn("runs API failed", err)

    def test_prints_the_base(self):
        with mock.patch.object(ci_plan_base, "green_runs", return_value=[run("a")]), \
             mock.patch.object(ci_plan_base, "git_is_ancestor", return_value=anything):
            out, _ = self.stdout_of_main()
        self.assertEqual(out, "a\n")


if __name__ == "__main__":
    unittest.main()
