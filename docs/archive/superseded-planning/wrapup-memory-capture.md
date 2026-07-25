# PRD: Wrap-up automation + two-tier memory

status: confirmed (2026-07-04, user)
upstream: .ctl/spec/alignment/2026-07-04-wrapup-memory-capture.md (confirmed)
downstream: ctl-to-tasks

## Objective

When a governed session wraps up (a task finishes, or the session stops with a
finish behind it), knowledge capture happens by default — split into a
**global tier** (`~/.ctl/memory/`: cross-project preferences and workflows) and
a **project tier** (`.ctl/spec/`: repo facts) — with zero new hard gates.

## Context

ObservedBasis: finish emits one event + observation summary; only
SessionStart/PreToolUse hooks are wired; ctl-spec-update is advisory prose; no
global tier exists; observe mode makes out-of-repo writes governable-by-
disclosure. (Full citations in the alignment note.)
ConfirmedBasis (user, 2026-07-04): Stop-hook reminder as trigger;
`~/.ctl/memory/` + adapter references as global tier; prose classifier first.
OpenUncertainty:
- U-A: exact Claude Stop-hook block semantics (can it block once with a reason
  without looping?) — verify against docs during implementation; fallback is a
  non-blocking reminder injection.
- U-B: how OMP/opencode surface the same reminder (their hook models differ);
  v1 may ship Claude-first with the others as follow-ups.

## Tasks

1. **wrapup-stop-hook-v1** — `.claude/hooks/ctl-wrapup.py` on the `Stop` event:
   query ctl for "tasks completed this session after the last capture"; if any,
   block once with a reason instructing the model to run `/ctl-spec-update`
   (loop-safe: second stop passes). Wire in `.claude/settings.json`; unit tests
   mirror test_ctl_gate.py; a `ctl hook wrapup-check` subcommand provides the
   query (surface-guard registered).
   Write scope: .claude/hooks/, .claude/settings.json, src/cli/mod.rs, tests.
2. **memory-two-tier-v1** — define `~/.ctl/memory/` layout (MEMORY.md index +
   one-fact files, mirroring the proven auto-memory shape); ctl-spec-update
   gains the tier-classifier prose (stable preference → global; repo fact →
   project) and writes both tiers directly (observe mode discloses the
   out-of-repo writes); SessionStart context injection mentions the global
   tier's index so sessions start with it.
   Write scope: .agent/skills (spec-update source is hand-authored? verify),
   skills copies, .claude/hooks/ctl-context.py, docs.
3. **wrapup-adapters-v1** (follow-up, may defer) — OMP `agent_end` /
   opencode equivalents surface the same reminder.

## Acceptance

- Finishing a task and stopping the session yields exactly one reminder; after
  a capture write, the next stop is silent.
- A cross-project preference lands under `~/.ctl/memory/` and is visible in the
  next session's injected context; a repo fact lands under `.ctl/spec/`.
- No lifecycle command gains a new hard gate; all captures are record-only.
