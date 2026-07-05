#!/usr/bin/env python3
"""Unit tests for the Claude SessionStart context hook (.claude/hooks/ctl-context.py).

The hook injects the active ctl task boundaries into the session and states how
the PreToolUse gate enforces them. These tests pin two things:

  * it injects nothing when there is no active task or ctl is unavailable (it
    must never fabricate task context out of an empty / failed ledger);
  * the enforcement notice is the HONEST per-tool wording from the 0.0.5 audit
    (D1): Write/Edit fail closed, Bash fails open, Task is not gated by
    PreToolUse — pinned here so the message cannot silently regress to the old
    "all mutating tools fail closed" overclaim.

No ctl binary: `subprocess.run` is mocked and stdout is asserted.

Run from this directory:  python -m unittest -v
"""
import importlib.util
import io
import json
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

_HOOK_PATH = Path(__file__).with_name("ctl-context.py")


def _load_hook():
    spec = importlib.util.spec_from_file_location("ctl_context_hook", _HOOK_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class _FakeCompleted:
    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def _ctx(*tasks):
    return {"active_tasks": list(tasks)}


def _task(task_id="t-1", objective="do the thing", write_allow=("src/foo",),
          write_deny=None, gates=None):
    boundary = {"write_allow": list(write_allow)}
    if write_deny is not None:
        boundary["write_deny"] = list(write_deny)
    if gates is not None:
        boundary["gates"] = list(gates)
    return {"id": task_id, "objective": objective, "boundary": boundary}


class ContextHookTest(unittest.TestCase):
    def setUp(self):
        self.mod = _load_hook()

    def _invoke(self, *, ctx=None, ctx_exc=None, ctx_stdout=None,
                memory_index=None):
        """Run main() over a mocked `ctl hook context`; return the injected
        additionalContext string, or None when the hook injected nothing.
        The global memory index defaults to a nonexistent path so tests stay
        independent of the developer machine's real ~/.ctl/memory."""
        if ctx_stdout is None and ctx is not None:
            ctx_stdout = json.dumps(ctx)

        def fake_run(args, **kwargs):
            if ctx_exc is not None:
                raise ctx_exc
            return _FakeCompleted(0, ctx_stdout or "")

        index = memory_index or str(
            Path(__file__).with_name("no-such-memory-index.md")
        )
        out = io.StringIO()
        with mock.patch.object(self.mod.subprocess, "run", side_effect=fake_run), \
                mock.patch.object(self.mod, "GLOBAL_MEMORY_INDEX", index), \
                redirect_stdout(out):
            try:
                self.mod.main()
            except SystemExit as exc:  # the "inject nothing" paths exit(0)
                self.assertEqual(exc.code, 0)
        text = out.getvalue().strip()
        if not text:
            return None
        payload = json.loads(text)
        self.assertEqual(
            payload["hookSpecificOutput"]["hookEventName"], "SessionStart"
        )
        return payload["hookSpecificOutput"]["additionalContext"]

    # ── never fabricate context ──

    def test_no_active_task_and_no_memory_injects_nothing(self):
        self.assertIsNone(self._invoke(ctx=_ctx()))

    def test_ctl_unavailable_and_no_memory_injects_nothing(self):
        self.assertIsNone(self._invoke(ctx_exc=OSError("ctl missing")))

    def test_unparseable_context_and_no_memory_injects_nothing(self):
        self.assertIsNone(self._invoke(ctx_stdout="not json"))

    # ── version visibility (B-lite: which binary answered?) ──

    def test_ctl_version_is_shown_in_the_header(self):
        ctx = self._invoke(ctx={"ctl_version": "9.9.9",
                                "active_tasks": [_task("t-v")]})
        self.assertIn("ctl 9.9.9", ctx)

    def test_missing_version_keeps_plain_header(self):
        ctx = self._invoke(ctx=_ctx(_task("t-v")))
        self.assertIn("Active ctl task boundaries — stay within", ctx)

    # ── global memory tier (memory-two-tier-v1) ──

    def _write_index(self, tmp, *entries):
        p = Path(tmp) / "MEMORY.md"
        p.write_text("\n".join(entries) + "\n", encoding="utf-8")
        return str(p)

    def test_global_memory_injects_even_when_idle(self):
        # Real state, not fabrication: an idle session still gets the index.
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            idx = self._write_index(
                tmp, "- [confirm intent](confirm-intent.md) -- propose then confirm"
            )
            ctx = self._invoke(ctx=_ctx(), memory_index=idx)
        self.assertIsNotNone(ctx)
        self.assertIn("Global memory index", ctx)
        self.assertIn("confirm-intent.md", ctx)

    def test_global_memory_appends_after_task_context(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            idx = self._write_index(tmp, "- [x](x.md) -- y")
            ctx = self._invoke(ctx=_ctx(_task("t-7")), memory_index=idx)
        self.assertIn("t-7", ctx)
        self.assertIn("Global memory index", ctx)
        self.assertLess(ctx.index("t-7"), ctx.index("Global memory index"))

    def test_empty_or_missing_index_is_silent(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            idx = self._write_index(tmp, "")  # whitespace-only
            self.assertIsNone(self._invoke(ctx=_ctx(), memory_index=idx))
        self.assertEqual(self.mod.global_memory_lines("no/such/path.md"), [])

    # ── boundary rendering ──

    def test_active_task_injects_id_objective_and_scope(self):
        ctx = self._invoke(ctx=_ctx(_task("task-42", "ship it", ["src/a", "src/b"])))
        self.assertIsNotNone(ctx)
        self.assertIn("task-42", ctx)
        self.assertIn("ship it", ctx)
        self.assertIn("src/a, src/b", ctx)

    def test_missing_write_scope_shows_placeholder(self):
        ctx = self._invoke(ctx=_ctx(_task(write_allow=[])))
        self.assertIn("(no write scope)", ctx)

    def test_deny_and_gates_rendered_when_present(self):
        ctx = self._invoke(
            ctx=_ctx(_task(write_deny=["src/secret"], gates=["cargo_check", "cargo_test"]))
        )
        self.assertIn("Deny: src/secret", ctx)
        self.assertIn("Gates: cargo_check, cargo_test", ctx)

    def test_multiple_active_tasks_are_all_listed(self):
        ctx = self._invoke(ctx=_ctx(_task("a", "first"), _task("b", "second")))
        self.assertIn("a", ctx)
        self.assertIn("b", ctx)
        self.assertIn("first", ctx)
        self.assertIn("second", ctx)

    # ── the honest enforcement notice (D1 regression guard, observe mode) ──

    def test_enforcement_notice_is_observe_mode_and_honest(self):
        ctx = self._invoke(ctx=_ctx(_task()))
        # Observe posture is stated, with the recording channel and hard core.
        self.assertIn("OBSERVE MODE", ctx)
        self.assertIn(".ctl/decisions.jsonl", ctx)
        self.assertIn("hard core still denies", ctx)
        self.assertIn("FAIL CLOSED", ctx)        # Write/Edit/MultiEdit on ctl-down
        self.assertIn("FAILS OPEN", ctx)         # Bash
        self.assertIn("not path-scoped", ctx)    # Bash is not a hard boundary
        # The U-1 platform boundary: Task is not gated by PreToolUse.
        self.assertIn("Task/subagent-spawn tool is NOT matched by PreToolUse", ctx)

    def test_enforcement_notice_avoids_the_old_overclaim(self):
        # Guard against regressing to a blanket "mutating tools ... fail closed
        # if ctl is unavailable" that silently covers Bash and Task.
        ctx = self._invoke(ctx=_ctx(_task()))
        self.assertNotIn("Mutating tools outside scope are blocked", ctx)


if __name__ == "__main__":
    unittest.main()
