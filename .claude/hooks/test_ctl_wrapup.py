#!/usr/bin/env python3
"""Unit tests for the Claude Stop wrap-up hook (.claude/hooks/ctl-wrapup.py).

Pins the contract:
  * pending=true → a top-level {"decision": "block", "reason": ...} that names
    both memory tiers and states the once-per-finish guarantee;
  * pending=false → no output (the stop proceeds);
  * every failure mode (ctl missing, non-zero exit, unparseable stdout,
    malformed stdin) FAILS OPEN — the session is never trapped.

No ctl binary and no model: `subprocess.run` is mocked, stdin is fed the Stop
payload, and stdout is asserted. Run from this directory: python -m unittest -v
"""
import importlib.util
import io
import json
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

_HOOK_PATH = Path(__file__).with_name("ctl-wrapup.py")


def _load_hook():
    spec = importlib.util.spec_from_file_location("ctl_wrapup_hook", _HOOK_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class _FakeCompleted:
    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


class WrapupHookTest(unittest.TestCase):
    def setUp(self):
        self.mod = _load_hook()

    def _invoke(self, *, verdict=None, ctl_exc=None, ctl_stdout=None,
                stdin_text=None):
        if ctl_stdout is None and verdict is not None:
            ctl_stdout = json.dumps(verdict)

        def fake_run(args, **kwargs):
            if ctl_exc is not None:
                raise ctl_exc
            return _FakeCompleted(0, ctl_stdout or "")

        out = io.StringIO()
        stdin = io.StringIO(stdin_text if stdin_text is not None else "{}")
        with mock.patch.object(self.mod.subprocess, "run", side_effect=fake_run), \
                mock.patch.object(self.mod.sys, "stdin", stdin), \
                redirect_stdout(out):
            with self.assertRaises(SystemExit) as cm:
                self.mod.main()
        self.assertEqual(cm.exception.code, 0)
        text = out.getvalue().strip()
        return json.loads(text) if text else None

    def test_pending_blocks_once_with_tiered_instructions(self):
        r = self._invoke(verdict={"pending": True, "task_id": "t-9"})
        self.assertEqual(r["decision"], "block")
        self.assertIn("t-9", r["reason"])
        self.assertIn(".ctl/spec/", r["reason"])          # project tier
        self.assertIn("~/.ctl/memory/", r["reason"])      # global tier
        self.assertIn("once per finish", r["reason"])     # once-guard promise

    def test_not_pending_is_silent(self):
        r = self._invoke(verdict={"pending": False, "reason": "no completed tasks"})
        self.assertIsNone(r, "a clear wrap-up must not block the stop")

    # ── fail-open matrix: a reminder must never trap the session ──

    def test_ctl_missing_fails_open(self):
        r = self._invoke(ctl_exc=OSError("ctl missing"))
        self.assertIsNone(r)

    def test_unparseable_ctl_output_fails_open(self):
        r = self._invoke(ctl_stdout="not json")
        self.assertIsNone(r)

    def test_malformed_stdin_fails_open(self):
        out = io.StringIO()
        with mock.patch.object(self.mod.sys, "stdin", io.StringIO("not json")), \
                redirect_stdout(out):
            with self.assertRaises(SystemExit) as cm:
                self.mod.main()
        self.assertEqual(cm.exception.code, 0)
        self.assertEqual(out.getvalue().strip(), "")


if __name__ == "__main__":
    unittest.main()
