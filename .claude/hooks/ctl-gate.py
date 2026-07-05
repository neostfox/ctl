#!/usr/bin/env python3
"""Claude Code PreToolUse hook — ctl governance gate (observe mode).

Calls `ctl hook gate` and translates the verdict into a Claude Code
permission decision. The gate's hard core (protected paths, deps step-up,
held tasks, cross-task overlap) still returns deny verdicts, which surface
as a deny decision. Observed verdicts (out-of-scope / task-less writes,
commits outside the Review window) come back `allowed: true` with a
`warning`: the hook forwards the warning to the model as PreToolUse
`additionalContext` WITHOUT a permissionDecision, so the write proceeds
under the normal permission flow and the model sees the governance nudge.
Fails CLOSED for Write/Edit/MultiEdit when ctl is unavailable: with no gate
there is no recorder, and the protected-path check would be blind.

Registered in .claude/settings.json for matcher "Write|Edit|MultiEdit|Bash".
"""
import json
import os
import subprocess
import sys

FAIL_CLOSED_TOOLS = {"Write", "Edit", "MultiEdit"}  # Bash excluded: ctl can fail to parse complex/non-ASCII command args on Windows; do not lock out the shell on ctl errors

# ── ctl binary resolution (B-lite: one chain everywhere) ─────────────────
# subprocess.run (no shell) needs a REAL executable. The single blessed chain
# is CTL_BIN → ~/.cargo/bin → PATH; npm probing was retired with the npm
# binary distribution (see .ctl/spec/alignment/2026-07-04-binary-distribution-
# shrink.md) so exactly one install location can shadow another no more.
_CTL_BIN_CACHE = None


def _platform_binary():
    return "ctl.exe" if sys.platform == "win32" else "ctl"


def _resolve_ctl_uncached():
    # 1. explicit operator override
    override = os.environ.get("CTL_BIN", "").strip()
    if override:
        return override
    # 2. cargo install target — the blessed install location
    cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", _platform_binary())
    if os.path.isfile(cargo):
        return cargo
    # 3. bare name — PATH resolution
    return "ctl"


def resolve_ctl():
    """Resolve a real `ctl` executable, memoized. See the module note above."""
    global _CTL_BIN_CACHE
    if _CTL_BIN_CACHE is None:
        _CTL_BIN_CACHE = _resolve_ctl_uncached()
    return _CTL_BIN_CACHE


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


def allow_with_context(context: str) -> None:
    """Defer to the normal permission flow, but inject model-visible context.

    additionalContext WITHOUT a permissionDecision: the tool call proceeds
    under the user's normal permission settings (the hook never silently
    auto-approves), while the model sees the observe-mode warning and can
    self-correct (create/widen a task) instead of being blocked.
    """
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": context,
        }
    }))
    sys.exit(0)


def record_decision(tool: str, ti: dict, verdict: dict) -> None:
    """Append blocked/flagged tool calls to the NON-CANONICAL .ctl/decisions.jsonl.

    Records every DENY (allowed != true) and any verdict the gate flags with
    record=true (e.g. a bash_write ALLOW, which is never path-scoped against
    write_allow). This turns "what the gate blocked/flagged" into auditable
    evidence. Best-effort: a logging failure must NEVER block or delay the
    tool call, so every error here is swallowed.
    """
    allowed = verdict.get("allowed") is True
    if allowed and verdict.get("record") is not True:
        return
    record = {
        "source": "claude",
        "tool": tool,
        "allowed": allowed,
        "state": verdict.get("state", ""),
        "reason": verdict.get("reason", ""),
    }
    if verdict.get("warning"):
        record["warning"] = verdict["warning"]
    if tool == "Bash":
        record["command"] = ti.get("command", "")
    else:
        record["path"] = ti.get("file_path", "")
    task = verdict.get("task_id") or os.environ.get("CTL_TASK_ID", "").strip()
    if task:
        record["task_id"] = task
    try:
        subprocess.run(
            [resolve_ctl(), "hook", "record-decision", "--data", json.dumps(record)],
            capture_output=True, text=True, timeout=5,
        )
    except Exception:
        pass  # advisory log only — never fail the tool on a logging error


def main() -> None:
    try:
        payload = json.load(sys.stdin)
    except Exception:
        allow()  # unparseable input — do not interfere

    tool = payload.get("tool_name", "")
    ti = payload.get("tool_input", {}) or {}

    args = [resolve_ctl(), "hook", "gate"]
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

    # Record denies + flagged allows before acting (a bash_write ALLOW must be
    # logged before the allow() exit below).
    record_decision(tool, ti, verdict)

    if verdict.get("allowed") is True:
        warning = verdict.get("warning", "")
        if warning:
            msg = f"ctl observe [{verdict.get('state', '')}]: {warning}"
            remedy = verdict.get("remedy")
            if remedy:
                msg += f"\nSuggested: {remedy}"
            allow_with_context(msg)
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
