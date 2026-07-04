---
name: ctl-to-tasks
description: "Convert a confirmed PRD or plan into ctl task proposals — vertical slices, each independently verifiable, each declaring objective, read scope, write scope, gates, acceptance evidence, an AFK/HITL label, and blocking uncertainties. Triggers when: a PRD or plan exists and you need to break it into governed tasks. Do NOT trigger for: scoping a single fuzzy request (ctl-grill-with-spec), or fabricating task state without a plan. Never creates tasks that bypass protected-path controls and never synthesizes completed ctl events from a plan."
---


## Station contract

- **Upstream**: a **confirmed** PRD from `ctl-to-prd` (`.ctl/spec/prd/<prd-id>.md`),
  or — for a single obvious task — a confirmed alignment note straight from
  `ctl-grill-with-spec`.
- **Produces**: task proposals handed to control-guard for `ctl task create`;
  each created task records `ctl brainstorm` provenance back to the alignment
  note / PRD it derived from (record-only — never gates).
- **Downstream**: execution under control-guard (`ctl-tdd-loop` for behavior
  changes), then wrap-up (finish → `ctl-spec-update`).

## Decompose into tasks (phase body)

Prefer **vertical slices** over horizontal layers. Each proposal must declare:

| Field | Rule |
|---|---|
| objective | one testable sentence |
| read scope | what the task may read |
| write scope | the **minimal** `write_allow` (start narrow, widen only on approval) |
| gates | required gate templates (`cargo_fmt_check`, `cargo_check`, `cargo_clippy`, `cargo_test`) |
| acceptance evidence | the artifact that proves done (test output, run output) |
| AFK / HITL | can it run unattended, or does it need a human in the loop? |
| blocking uncertainties | the OpenUncertainty items that must resolve first |

**AFK vs HITL.** Label each task: **AFK** (away-from-keyboard — deterministic,
low-risk, fully gated, safe to run unattended) or **HITL** (human-in-the-loop —
needs a decision, touches a protected path via exception, or carries unresolved
risk). When unsure, label HITL.

### Hard rules

- Prefer vertical slices; split anything too big to finish inside one boundary.
- Each task is independently verifiable on its own evidence.
- Non-overlapping `write_allow` across sibling tasks (overlap forces sequencing).
- **Do not create tasks that bypass protected-path controls** (`.git/`,
  `events.jsonl`, `schemas/`, `Cargo.toml`).
- **Do not synthesize completed ctl events from a PRD/plan.** A plan describes
  intended work; only real execution + evidence may close a task.

### Output: proposals, then governed creation

Write proposals to `.ctl/tasks/<task-id>/proposal.md` (within scope), or dry-run
the boundary before committing to it:

```
ctl task create --dry-run --id <id> --objective "..." \
  --read-scope <p> --write-allow <p> --gates cargo_check --gates cargo_test
```

`--dry-run` validates and shows what would happen without persisting — a safe way
to check a proposed boundary before the real create. Hand approved proposals to
control-guard for the actual `ctl task create`.

### Anti-patterns

- ❌ Horizontal layer tasks that can't be verified alone.
- ❌ A broad `write_allow` "to be safe".
- ❌ Overlapping write scopes across sibling tasks.
- ❌ Marking a risky task AFK to avoid asking.
- ❌ Emitting events that claim work is done before it ran.

<!-- integration:omp -->

Proposals are notes; the real `ctl task create` is dispatched by control-guard and gated
by the OMP PreToolUse hook. Use `ctl task create --dry-run` to preview a boundary, and
`ctl board` to check sibling tasks for write-scope overlap before creating. Record PRD
provenance on the created task with `ctl brainstorm` (record-only).
<!-- integration:opencode -->

Proposals are notes; the real `ctl task create` is gated by `.opencode/plugins/ctl-gate.ts`.
Use `ctl task create --dry-run` to preview a boundary and `ctl board` to check sibling
tasks for write-scope overlap before creating. Record PRD provenance with `ctl brainstorm`.

**Recommended role** (autonomous dispatch — see control-guard): `designer` — shaping
vertical task proposals from a PRD is design work; the proposals are notes until the
gated `ctl task create`. Writable role, so it needs an active in_progress task.
<!-- integration:claude -->

Proposals are notes; the real `ctl task create` is gated by the Claude Code PreToolUse hook (`.claude/hooks/ctl-gate.py`). Use `ctl task create --dry-run` to preview a boundary, and `ctl board` to check sibling tasks for write-scope overlap before creating. Record PRD provenance on the created task with `ctl brainstorm` (record-only).
