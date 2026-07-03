#!/usr/bin/env python3
"""Claude Code SessionStart hook — inject active ctl task boundaries.

Calls `ctl hook context` and surfaces the active task's write scope so the
model knows the boundaries up front. Enforcement itself is done by
ctl-gate.py (PreToolUse); this hook is informational only.
"""
import json
import subprocess
import sys


def main() -> None:
    try:
        out = subprocess.run(
            ["ctl", "hook", "context"],
            capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=5,
        )
        ctx = json.loads(out.stdout)
    except Exception:
        sys.exit(0)  # ctl unavailable — nothing to inject

    active = ctx.get("active_tasks") or []
    if not active:
        sys.exit(0)

    lines = ["Active ctl task boundaries — stay within write scope:"]
    for t in active:
        b = t.get("boundary") or {}
        scope = ", ".join(b.get("write_allow") or []) or "(no write scope)"
        lines.append(f"  - {t.get('id', '')}: {t.get('objective', '')}")
        lines.append(f"    Write: {scope}")
        if b.get("write_deny"):
            lines.append(f"    Deny: {', '.join(b['write_deny'])}")
        if b.get("gates"):
            lines.append(f"    Gates: {', '.join(b['gates'])}")
    lines.append(
        "The PreToolUse gate runs in OBSERVE MODE: out-of-scope or task-less "
        "writes, and commits outside the Review window, are allowed but recorded "
        "to .ctl/decisions.jsonl with a model-visible warning — a warning is a "
        "prompt to create/widen a task, not permission to ignore governance. "
        "The hard core still denies: protected paths (.git, .ctl ledgers, "
        "schemas/, Cargo.toml/lock), dependency changes without a deps approval, "
        "held tasks, and cross-task write overlap. Write/Edit/MultiEdit FAIL "
        "CLOSED when ctl is unavailable; Bash FAILS OPEN and is not path-scoped. "
        "The Task/subagent-spawn tool is NOT matched by PreToolUse at all "
        "(a Claude platform boundary, not a TODO — see .claude/subagent-dispatch.md): "
        "dispatch only read-only subagents and keep writes inline in the main agent."
    )

    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": "\n".join(lines),
        }
    }))


if __name__ == "__main__":
    main()
