#!/usr/bin/env python3
"""Claude Code PreToolUse hook — enforce ctl governance boundaries.

Calls `ctl hook gate` and translates the verdict into a Claude Code
permission decision. Fails CLOSED for mutating tools when ctl is
unavailable: an unenforceable boundary must never silently allow writes.

Registered in .claude/settings.json for matcher "Write|Edit|MultiEdit|Bash".
"""
import json
import os
import subprocess
import sys

FAIL_CLOSED_TOOLS = {"Write", "Edit", "MultiEdit"}  # Bash excluded: ctl can fail to parse complex/non-ASCII command args on Windows; do not lock out the shell on ctl errors


def deny(reason: str) -> None:
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    }))
    sys.exit(0)


def allow() -> None:
    # No decision => defer to normal permission flow.
    sys.exit(0)


def main() -> None:
    try:
        payload = json.load(sys.stdin)
    except Exception:
        allow()  # unparseable input — do not interfere

    tool = payload.get("tool_name", "")
    ti = payload.get("tool_input", {}) or {}

    args = ["ctl", "hook", "gate"]
    if tool in ("Write", "Edit", "MultiEdit"):
        args += ["--tool", "write", "--path", ti.get("file_path", "")]
    elif tool == "Bash":
        args += ["--tool", "bash", "--command", ti.get("command", "")]
    else:
        allow()  # not a gated tool

    # M-e: forward the dispatch binding so the gate governs this call by the
    # task that dispatched it (resolves multi-active ambiguity). ctl also reads
    # CTL_TASK_ID from its own env, so this is the explicit, audited seam.
    task = os.environ.get("CTL_TASK_ID", "").strip()
    if task:
        args += ["--task", task]

    try:
        out = subprocess.run(
            args, capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=5,
        )
    except Exception:
        if tool in FAIL_CLOSED_TOOLS:
            deny("ctl gate unavailable (timeout or missing binary) — failing "
                 "closed. Ensure `ctl` is on PATH.")
        allow()

    if out.returncode != 0:
        if tool in FAIL_CLOSED_TOOLS:
            deny("ctl gate error — failing closed.\n" + (out.stderr or "")[:300])
        allow()

    try:
        verdict = json.loads(out.stdout)
    except Exception:
        if tool in FAIL_CLOSED_TOOLS:
            deny("ctl gate returned unparseable output — failing closed.")
        allow()

    if verdict.get("allowed") is True:
        allow()

    state = verdict.get("state", "")
    reason = verdict.get("reason", "blocked by ctl governance")
    msg = f"ctl gate [{state}]: {reason}"
    remedy = verdict.get("remedy")
    if remedy:
        msg += f"\nRemedy: {remedy}"
    deny(msg)


if __name__ == "__main__":
    main()
