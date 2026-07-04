# ADR: Gate Observe Mode — record-and-disclose replaces deny-by-default

Status: accepted (2026-07-03, user-confirmed direction)
Supersedes: the deny-by-default write-gate behavior shipped through 0.0.10.
Extends: `.ctl/spec/prd/attestation-layer.md` (record-and-disclose stance).

## Context

The PreToolUse write gate denied every Write/Edit outside the active task's
`write_allow` and **all** writes when no task was `in_progress` (idle). Field
experience showed the deny posture produced friction without matching
enforcement value:

- **The boundary was already only half-hard.** Bash was never path-scoped
  (`classify_bash` gates on task state, not path), so the "hard write
  boundary" existed only for the Write/Edit tools.
- **Real verification happens later.** Gate evidence binds to the committed
  tree; the completion audit, reviewer independence, and the finish interlock
  are all post-commit. Write-time is the earliest, least-informed, most
  interruption-prone enforcement point.
- **Protected paths were protected only incidentally**: an out-of-repo or
  protected path could never be in `write_allow`, so it fell into the generic
  "outside write_allow" deny. Softening that deny required making protection
  explicit.
- The attestation-layer PRD already established the project's philosophy:
  ctl records provenance and discloses; it informs, it does not gate
  cognition. Observe mode extends the same stance to write-time enforcement.

## Decision

`ctl hook gate` verdicts change for the **softened categories** — from
`allowed:false` to `allowed:true` + `record:true` + a `warning` field:

| Scenario | Before | Now |
|---|---|---|
| Write/Edit outside `write_allow` (in-repo) | deny | allow + record + warning |
| Write/Edit outside the repository | deny (incidental) | allow + record + warning |
| Write/Edit with no `in_progress` task (idle/review/completed) | deny | allow + record + warning |
| `bash_write` with no `in_progress` task | deny | allow + record + warning |
| `git commit` / `git push` outside the Review window | deny | allow + record + warning |

Observed verdicts are appended to the **non-canonical** decision log
(`.ctl/decisions.jsonl`, the gate-decision-log-v1 channel). The Claude hook
surfaces the `warning` to the model as PreToolUse `additionalContext` without
a `permissionDecision`, so the normal permission flow is untouched — the gate
never silently escalates approval.

The **hard core remains hard** (unchanged deny):

- protected paths (`.git`, `.ctl` ledgers, `.control`, `schemas/`,
  `Cargo.toml`, `Cargo.lock` — the boundary-normalizer list, now checked
  explicitly in the gate; a granted `ctl apply` exception still authorizes a
  specific path);
- traversal/UNC targets the gate refuses to classify;
- dependency changes without a granted `deps` step-up approval;
- writes while the task is **held**;
- cross-task write overlap (M-c) and multiple-active ambiguity without a
  dispatch binding;
- destructive git operations while agent runs are active (M6);
- `ctl` unavailable still **fails closed** for Write/Edit/MultiEdit: with no
  gate there is no recorder, and the protected-path check would be blind.

## Consequences

- A warning is a prompt to re-scope, not permission to ignore governance:
  the agent is expected to create/widen a task when warned, and reviewers can
  audit every ungoverned write in `.ctl/decisions.jsonl`.
- Out-of-repo writes (e.g. agent memory directories) are now possible and
  disclosed, unblocking session-end memory capture (global/project tiers).
- The finish interlock, gates, completion audit, and reviewer independence
  are unchanged and remain the actual correctness spine.
- opencode's plugin allows-and-records observed verdicts but does not yet
  surface the `warning` text to the model (follow-up: forward it via the
  plugin's output channel).
