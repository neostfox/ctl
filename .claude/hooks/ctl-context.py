#!/usr/bin/env python3
"""Claude Code SessionStart hook — inject active ctl task boundaries.

Calls `ctl hook context` and surfaces the active task's write scope so the
model knows the boundaries up front, plus the GLOBAL memory index
(`~/.ctl/memory/MEMORY.md` — the cross-project tier ctl-spec-update writes;
platform adapters reference it so every session starts with it). Enforcement
itself is done by ctl-gate.py (PreToolUse); this hook is informational only.
"""
import json
import os
import subprocess
import sys

GLOBAL_MEMORY_INDEX = os.path.join(
    os.path.expanduser("~"), ".ctl", "memory", "MEMORY.md"
)
MAX_INDEX_LINES = 30


def resolve_ctl():
    """The one blessed resolution chain: CTL_BIN → ~/.cargo/bin → PATH.
    Identical across all three .claude hooks so they can never run
    different binaries in the same session."""
    override = os.environ.get("CTL_BIN", "").strip()
    if override:
        return override
    binname = "ctl.exe" if sys.platform == "win32" else "ctl"
    cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", binname)
    if os.path.isfile(cargo):
        return cargo
    return "ctl"


def global_memory_lines(index_path=None):
    """Render the global memory index as context lines, or [] when absent.

    Absent/empty index is normal (the tier is created by the first capture);
    read errors must never break session start.
    """
    path = index_path or GLOBAL_MEMORY_INDEX
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            entries = [line.rstrip() for line in f if line.strip()]
    except OSError:
        return []
    if not entries:
        return []
    lines = [
        "Global memory index (~/.ctl/memory/MEMORY.md — cross-project "
        "preferences captured by ctl-spec-update; read a referenced file "
        "before applying it):"
    ]
    lines.extend(f"  {entry}" for entry in entries[:MAX_INDEX_LINES])
    return lines


def main() -> None:
    try:
        out = subprocess.run(
            [resolve_ctl(), "hook", "context"],
            capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=5,
        )
        ctx = json.loads(out.stdout)
    except Exception:
        ctx = {}  # ctl unavailable — no task context, but memory can still inject

    active = ctx.get("active_tasks") or []
    if not active:
        mem = global_memory_lines()
        if not mem:
            sys.exit(0)  # nothing real to inject — never fabricate context
        print(json.dumps({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": "\n".join(mem),
            }
        }))
        sys.exit(0)

    version = ctx.get("ctl_version", "")
    header = (
        f"Active ctl task boundaries (ctl {version}) — stay within write scope:"
        if version
        else "Active ctl task boundaries — stay within write scope:"
    )
    lines = [header]
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

    mem = global_memory_lines()
    if mem:
        lines.append("")
        lines.extend(mem)

    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": "\n".join(lines),
        }
    }))


if __name__ == "__main__":
    main()
