# Alignment: wrap-up automation + two-tier memory (task 3)

status: confirmed (2026-07-04, user)
station: align (ctl-grill-with-spec)
downstream: ctl-to-prd → ctl-to-tasks

## Observed facts (cited)

- `ctl task finish` emits one `task_completed` event and rebuilds the view;
  since 0.0.10 it also prints an observation-log summary. No knowledge-capture
  side effect (`src/application/mod.rs` finish_task; `src/cli/mod.rs` Finish arm).
- Hook wiring today: `SessionStart` (context) + `PreToolUse` (gate) only;
  `Stop` / `SessionEnd` / `PostToolUse` are unwired (`.claude/settings.json`).
- `ctl-spec-update` exists but is an advisory routing note in control-guard —
  the model may or may not run it after finish.
- No global/user-level memory tier exists anywhere in ctl; all knowledge targets
  are repo-scoped (`.ctl/spec/`, `.ctl/tasks/<id>/`, repo CLAUDE.md).
- Gate observe mode (0.0.10) allows out-of-repo writes with record+warning, so
  a global tier is now writable under governance-by-disclosure.
- Prototype lineage: trellis Phase 4 runs `trellis-update-spec` automatically at
  finish and keeps session journals in `.trellis/workspace/` — repo-tier only.

## Declared rules

- ctl records and disclosures; cognition is never gated (attestation-layer PRD).
- No new hard interlocks — user has explicitly rejected more mandatory process.
- Platform-specific mechanics live in adapters, not in the ctl core.

## Assumptions (unconfirmed)

- A Claude `Stop` hook can block-with-reason once to make the model perform the
  capture step before the session ends (needs a doc check during build).
- The user wants memory capture on *task finish* primarily; *session end with no
  finish* is the secondary case.

## User goals

Session/task wrap-up updates memory automatically — split into a global tier
(cross-project preferences/workflows) and a project tier (repo facts) — with
less user intervention, not more ceremony.

## Non-goals

- No canonical memory events in the ledger (record-only artifacts suffice).
- No hard gate on finish (capture is prompted/automated, never a finish blocker).
- Not a general RAG/knowledge system — files an agent reads at session start.

## Micro-decisions (CONFIRMED by user, 2026-07-04)

1. Trigger mechanism → **Stop-hook reminder**: on session stop, detect "a task
   finished this session with no knowledge capture afterwards"; if hit, block
   once with a reason so the model performs the capture while still present.
   Recommended over SessionEnd (model already gone) and prose-only (relies on
   model discipline).
2. Global-tier location → **`~/.ctl/memory/` + adapter references**: a
   platform-neutral user-level directory owned by ctl; Claude/OMP/opencode
   adapters inject/reference it at session start. One truth across platforms.
3. Tier classifier → **prose rules in ctl-spec-update** (stable preferences →
   global; repo facts → project), writing files directly under observe mode;
   a `ctl memory capture` CLI carrier is deferred until the prose loop proves
   the shape.

## Minimum viable experiment

Wire a `Stop` hook that checks "any task finished this session without a
spec-update/memory write afterwards" and injects a one-line reminder; measure
whether capture happens without user prompting in the next real session.
