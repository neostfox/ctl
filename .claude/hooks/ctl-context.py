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
        "Enforcement (Claude Code PreToolUse gate) is per-tool, not uniform: "
        "Write/Edit/MultiEdit are path-scoped against write_allow and FAIL CLOSED "
        "when ctl is unavailable. Bash is gated but FAILS OPEN on a ctl "
        "error/timeout (the shell is never locked out) and is not path-scoped — so "
        "Bash is not a hard write boundary; prefer the Write/Edit tools for in-scope "
        "edits. The Task/subagent-spawn tool is NOT matched by PreToolUse at all "
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
