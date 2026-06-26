#!/usr/bin/env python3
"""Claude Code PreToolUse hook — enforce ctl governance boundaries.

Calls `ctl hook gate` and translates the verdict into a Claude Code
permission decision. Fails CLOSED for mutating tools when ctl is
unavailable: an unenforceable boundary must never silently allow writes.

Registered in .claude/settings.json for matcher "Write|Edit|MultiEdit|Bash".
"""
import json
import os
import platform
import subprocess
import sys

FAIL_CLOSED_TOOLS = {"Write", "Edit", "MultiEdit"}  # Bash excluded: ctl can fail to parse complex/non-ASCII command args on Windows; do not lock out the shell on ctl errors

# ── ctl binary resolution ────────────────────────────────────────────────
# subprocess.run (no shell) needs a REAL executable. On Windows a global
# `npm i -g @velo-ai/ctl` exposes only .cmd/.ps1 shims on PATH (never a real
# ctl.exe), so a bare ["ctl", ...] call raises FileNotFoundError, the gate
# becomes "unavailable", and Write/Edit/MultiEdit fail closed — locking out the
# session. Resolve a real binary first; CTL_BIN stays the explicit override.
_CTL_BIN_CACHE = None


def _platform_binary():
    return "ctl.exe" if sys.platform == "win32" else "ctl"


def _platform_dir():
    """The napi platform tuple matching npm/bin/ctl.js (platforms/<dir>/)."""
    machine = platform.machine().lower()
    arch = "x64" if machine in ("amd64", "x86_64") else (
        "arm64" if machine in ("arm64", "aarch64") else machine)
    plat = ("win32" if sys.platform == "win32"
            else "darwin" if sys.platform == "darwin" else "linux")
    tuples = {
        "win32-x64": "win32-x64-msvc",
        "darwin-x64": "darwin-x64",
        "darwin-arm64": "darwin-arm64",
        "linux-x64": "linux-x64-gnu",
        "linux-arm64": "linux-arm64-gnu",
    }
    key = f"{plat}-{arch}"
    return tuples.get(key, key)


def _resolve_ctl_uncached():
    # 1. explicit operator override
    override = os.environ.get("CTL_BIN", "").strip()
    if override:
        return override
    binname = _platform_binary()
    rel = os.path.join("node_modules", "@velo-ai", "ctl",
                       "platforms", _platform_dir(), binname)
    # 2. local npm: walk up node_modules from this hook's directory
    d = os.path.dirname(os.path.abspath(__file__))
    while True:
        cand = os.path.join(d, rel)
        if os.path.isfile(cand):
            return cand
        parent = os.path.dirname(d)
        if parent == d:
            break
        d = parent
    # 3. global npm: probe the known global roots (invisible to a local walk;
    #    on Windows only shims sit on PATH, so the real exe must be found here)
    roots = []
    prefix = os.environ.get("npm_config_prefix", "").strip()
    if prefix:
        roots.append(prefix)             # Windows: <prefix>\node_modules
        roots.append(os.path.join(prefix, "lib"))  # unix: <prefix>/lib/node_modules
    if sys.platform == "win32":
        appdata = os.environ.get("APPDATA", "").strip()
        if appdata:
            roots.append(os.path.join(appdata, "npm"))  # default global root
    else:
        roots += ["/usr/local", "/usr/local/lib", "/usr", "/usr/lib"]
        home = os.path.expanduser("~")
        roots += [os.path.join(home, ".npm-global"),
                  os.path.join(home, ".npm-global", "lib")]
    for r in roots:
        cand = os.path.join(r, rel)
        if os.path.isfile(cand):
            return cand
    # 4. cargo install
    cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", binname)
    if os.path.isfile(cargo):
        return cargo
    # 5. bare name — PATH resolution (prior behavior)
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
