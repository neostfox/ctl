#!/usr/bin/env python3
"""Unit tests for the Claude PreToolUse gate hook (.claude/hooks/ctl-gate.py).

These pin the per-tool enforcement contract the hook must honor — the contract
the 0.0.5 honesty audit (D1 / Finding 6 / U-1) found was previously UNTESTED:

  * Write / Edit / MultiEdit FAIL CLOSED (deny) when ctl is unavailable;
  * Bash FAILS OPEN (allow) when ctl is unavailable — the shell is never locked;
  * the Task / subagent-spawn tool is NOT matched by PreToolUse at all, so the
    hook never even consults ctl for it (the U-1 platform boundary);
  * idle / out-of-scope deny verdicts surface as a deny decision;
  * deny verdicts and bash_write allows are forwarded to the decision log.

No ctl binary and no model: `subprocess.run` is mocked, stdin is fed a tool
payload, and the emitted permission decision (stdout JSON) is asserted.

Run from this directory:  python -m unittest -v
"""
import importlib.util
import io
import json
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

_HOOK_PATH = Path(__file__).with_name("ctl-gate.py")


def _load_hook():
    """Load the hyphen-named hook as an importable module (fresh per test)."""
    spec = importlib.util.spec_from_file_location("ctl_gate_hook", _HOOK_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class _FakeCompleted:
    """Minimal stand-in for subprocess.CompletedProcess."""

    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


class GateHookTest(unittest.TestCase):
    def setUp(self):
        self.mod = _load_hook()
        self.gate_calls = []
        self.record_calls = []

    def _invoke(self, payload, *, gate_exc=None, gate_rc=0, verdict=None,
                gate_stdout=None, env=None):
        """Run the hook's main() over a mocked ctl and stdin.

        Returns the parsed decision dict the hook printed, or None when the hook
        emitted nothing (i.e. it allowed / deferred to the normal permission
        flow). `gate_exc` simulates ctl being unavailable; `verdict` is the gate
        JSON ctl would return; `gate_rc` / `gate_stdout` drive the non-zero and
        unparseable paths.
        """
        if gate_stdout is None and verdict is not None:
            gate_stdout = json.dumps(verdict)

        def fake_run(args, **kwargs):
            if "record-decision" in args:
                self.record_calls.append(args)
                return _FakeCompleted(0, '{"recorded": true}')
            # the gate call
            self.gate_calls.append(args)
            if gate_exc is not None:
                raise gate_exc
            return _FakeCompleted(gate_rc, gate_stdout or "")

        out = io.StringIO()
        with mock.patch.object(self.mod.subprocess, "run", side_effect=fake_run), \
                mock.patch.dict(self.mod.os.environ, env or {}, clear=True), \
                mock.patch.object(self.mod.sys, "stdin",
                                  io.StringIO(json.dumps(payload))), \
                redirect_stdout(out):
            with self.assertRaises(SystemExit) as cm:
                self.mod.main()
        # The hook always exits 0; the decision lives in stdout, not the code.
        self.assertEqual(cm.exception.code, 0)
        text = out.getvalue().strip()
        return json.loads(text) if text else None

    @staticmethod
    def _decision(result):
        if result is None:
            return None
        return result["hookSpecificOutput"]["permissionDecision"]

    @staticmethod
    def _reason(result):
        return result["hookSpecificOutput"]["permissionDecisionReason"]

    @staticmethod
    def _record_payload(call):
        return json.loads(call[call.index("--data") + 1])

    # ── fail-closed (Write/Edit/MultiEdit) vs fail-open (Bash) on ctl down ──

    def test_write_family_fails_closed_when_ctl_unavailable(self):
        for tool in ("Write", "Edit", "MultiEdit"):
            r = self._invoke(
                {"tool_name": tool, "tool_input": {"file_path": "src/x.rs"}},
                gate_exc=OSError("ctl missing"),
            )
            self.assertEqual(self._decision(r), "deny", f"{tool} must fail closed")
        # ctl was down, so nothing could be recorded either.
        self.assertEqual(self.record_calls, [])

    def test_bash_fails_open_when_ctl_unavailable(self):
        r = self._invoke(
            {"tool_name": "Bash", "tool_input": {"command": "echo hi"}},
            gate_exc=OSError("ctl missing"),
        )
        self.assertIsNone(r, "Bash must fail OPEN (no deny) when ctl is unavailable")

    def test_write_fails_closed_on_nonzero_exit(self):
        r = self._invoke(
            {"tool_name": "Write", "tool_input": {"file_path": "src/x"}},
            gate_rc=1, gate_stdout="error text",
        )
        self.assertEqual(self._decision(r), "deny")

    def test_write_fails_closed_on_unparseable_output(self):
        r = self._invoke(
            {"tool_name": "Write", "tool_input": {"file_path": "src/x"}},
            gate_rc=0, gate_stdout="not json",
        )
        self.assertEqual(self._decision(r), "deny")

    def test_bash_fails_open_on_unparseable_output(self):
        r = self._invoke(
            {"tool_name": "Bash", "tool_input": {"command": "ls"}},
            gate_rc=0, gate_stdout="not json",
        )
        self.assertIsNone(r, "Bash must not be locked out by a ctl parse error")

    # ── verdict-driven allow / deny ──

    def test_in_scope_write_is_allowed(self):
        r = self._invoke(
            {"tool_name": "Write", "tool_input": {"file_path": ".claude/x"}},
            verdict={"allowed": True, "state": "in_progress",
                     "reason": "within write_allow"},
        )
        self.assertIsNone(r, "an allowed write emits no decision")

    def test_out_of_scope_write_is_denied_with_reason_and_remedy(self):
        r = self._invoke(
            {"tool_name": "Write", "tool_input": {"file_path": "Cargo.toml"}},
            verdict={"allowed": False, "state": "in_progress",
                     "reason": "outside write_allow",
                     "remedy": "ctl task revise --id <id>"},
        )
        self.assertEqual(self._decision(r), "deny")
        self.assertIn("outside write_allow", self._reason(r))
        self.assertIn("ctl task revise", self._reason(r))

    def test_idle_write_is_denied(self):
        # Idle = no active in_progress task; writes must be denied.
        r = self._invoke(
            {"tool_name": "Write", "tool_input": {"file_path": "src/x"}},
            verdict={"allowed": False, "state": "idle",
                     "reason": "no active in_progress task — create one first"},
        )
        self.assertEqual(self._decision(r), "deny")
        self.assertIn("idle", self._reason(r))

    # ── the U-1 platform boundary: PreToolUse does not gate Task ──

    def test_task_tool_is_ungoverned_and_never_reaches_ctl(self):
        # The matcher is Write|Edit|MultiEdit|Bash. Task/Agent is not matched, so
        # the hook allows without consulting ctl. This pins the U-1 boundary:
        # PreToolUse structurally cannot gate subagent spawn.
        r = self._invoke(
            {"tool_name": "Task", "tool_input": {"subagent_type": "designer"}},
        )
        self.assertIsNone(r, "Task must be allowed (no deny)")
        self.assertEqual(self.gate_calls, [], "Task must never reach `ctl hook gate`")

    def test_unrelated_read_tool_is_not_gated(self):
        r = self._invoke({"tool_name": "Read", "tool_input": {"file_path": "x"}})
        self.assertIsNone(r)
        self.assertEqual(self.gate_calls, [])

    # ── decision-log recording (gate-decision-log-v1 contract) ──

    def test_deny_is_recorded_to_the_decision_log(self):
        self._invoke(
            {"tool_name": "Write", "tool_input": {"file_path": "Cargo.toml"}},
            verdict={"allowed": False, "state": "in_progress",
                     "reason": "outside write_allow"},
        )
        self.assertEqual(len(self.record_calls), 1)
        data = self._record_payload(self.record_calls[0])
        self.assertFalse(data["allowed"])
        self.assertEqual(data["source"], "claude")
        self.assertEqual(data["tool"], "Write")
        self.assertEqual(data["path"], "Cargo.toml")

    def test_bash_write_allow_is_recorded_but_still_allowed(self):
        r = self._invoke(
            {"tool_name": "Bash", "tool_input": {"command": "echo hi > src/x.rs"}},
            verdict={"allowed": True, "record": True, "state": "in_progress",
                     "reason": "bash write allowed under active task — "
                               "NOT path-scope-checked against write_allow"},
        )
        self.assertIsNone(r, "a flagged bash_write is still allowed")
        self.assertEqual(len(self.record_calls), 1)
        data = self._record_payload(self.record_calls[0])
        self.assertTrue(data["allowed"])
        self.assertEqual(data["command"], "echo hi > src/x.rs")

    def test_ordinary_allow_is_not_recorded(self):
        self._invoke(
            {"tool_name": "Write", "tool_input": {"file_path": ".claude/x"}},
            verdict={"allowed": True, "state": "in_progress",
                     "reason": "within write_allow"},
        )
        self.assertEqual(self.record_calls, [], "a plain allow must not be logged")

    def test_dispatch_binding_is_forwarded_to_ctl(self):
        # CTL_TASK_ID in the environment must be forwarded as --task so the gate
        # binds this call to its dispatching task under multi-active ambiguity.
        self._invoke(
            {"tool_name": "Write", "tool_input": {"file_path": ".claude/x"}},
            verdict={"allowed": True, "state": "in_progress", "reason": "ok"},
            env={"CTL_TASK_ID": "my-task"},
        )
        self.assertEqual(len(self.gate_calls), 1)
        self.assertIn("--task", self.gate_calls[0])
        self.assertEqual(
            self.gate_calls[0][self.gate_calls[0].index("--task") + 1], "my-task"
        )

    # ── ctl-binary resolution (claude-gate-ctl-resolve-v1 contract) ──

    def test_ctl_bin_env_override_is_used_as_the_binary(self):
        # CTL_BIN is the explicit operator override (resolution priority #1).
        # On Windows a `npm i -g @velo-ai/ctl` exposes only .cmd/.ps1 shims on
        # PATH (no real ctl.exe), so a bare "ctl" execFile/subprocess fails and
        # the gate would fail closed. The hook must invoke the resolved binary,
        # not the literal string "ctl".
        override = "/custom/path/to/ctl.exe"
        self._invoke(
            {"tool_name": "Write", "tool_input": {"file_path": ".claude/x"}},
            verdict={"allowed": True, "state": "in_progress", "reason": "ok"},
            env={"CTL_BIN": override},
        )
        self.assertEqual(len(self.gate_calls), 1)
        self.assertEqual(
            self.gate_calls[0][0], override,
            "gate must invoke the CTL_BIN-resolved binary, not bare 'ctl'",
        )

    def test_bare_ctl_is_the_fallback_when_nothing_resolves(self):
        # With no override and no install present, resolution falls through to
        # bare "ctl" (PATH) — prior behavior preserved. isfile is forced False so
        # the result is independent of any real npm/cargo install on the host.
        with mock.patch.dict(self.mod.os.environ, {}, clear=True), \
                mock.patch.object(self.mod.os.path, "isfile", return_value=False):
            self.mod._CTL_BIN_CACHE = None
            self.assertEqual(self.mod.resolve_ctl(), "ctl")


if __name__ == "__main__":
    unittest.main()
