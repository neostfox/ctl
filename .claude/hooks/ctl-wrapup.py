#!/usr/bin/env python3
"""Claude Code Stop hook — wrap-up knowledge-capture reminder.

Asks `ctl hook wrapup-check` whether the most recent task completion is still
missing a knowledge capture (project tier `.ctl/spec/`, global tier
`~/.ctl/memory/`). If pending, blocks the stop ONCE with instructions — ctl's
once-guard (`.ctl/wrapup-reminded.json`) marks the finish as reminded during
that same check, so the identical finish can never block a second stop, and a
capture write clears the pending state on its own.

Fails OPEN on every error (ctl missing, timeout, unparseable output): a
reminder must never trap the user in a session.

Registered in .claude/settings.json for the Stop event.
"""
import json
import os
import subprocess
import sys


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


def main() -> None:
    try:
        json.load(sys.stdin)  # payload unused; malformed input must not block
    except Exception:
        sys.exit(0)

    try:
        out = subprocess.run(
            [resolve_ctl(), "hook", "wrapup-check"],
            capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=5,
        )
        verdict = json.loads(out.stdout)
    except Exception:
        sys.exit(0)  # fail OPEN — never trap the session on a ctl error

    if verdict.get("pending") is True:
        task = verdict.get("task_id", "")
        print(json.dumps({
            "decision": "block",
            "reason": (
                f"ctl wrap-up: task '{task}' finished without a knowledge "
                "capture. Run /ctl-spec-update now — durable repo lessons go "
                "to .ctl/spec/ (project tier); stable cross-project "
                "preferences and workflows go to ~/.ctl/memory/ (global "
                "tier, one fact per file plus a MEMORY.md index line). "
                "This reminder fires once per finish; stop again afterwards."
            ),
        }))
    sys.exit(0)


if __name__ == "__main__":
    main()
