---
name: ctl-to-tasks
description: "Convert a confirmed PRD or plan into governed ctl task proposals — vertical slices broken down for control-guard to create. Triggers when: a PRD or plan exists and you need to break it into governed tasks. Do NOT trigger for: scoping a single fuzzy request (ctl-grill-with-spec), or fabricating task state without a plan."
---

# ctl-to-tasks (OMP)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-to-tasks/source.md` by `ctl skills sync`. OMP-specific mechanics live after the core.

<!-- ctl:workflow-core:start version=1 -->
# ctl Workflow Skills — Core Protocol

WORKFLOW_PROTOCOL_VERSION = 1

This is the platform-neutral workflow-skills core. It is split into an
**embedded** part (division of labor + invariants), carried verbatim inside
every workflow skill's managed-core block, and a **reference** part (phase map +
frameworks + provenance) that lives only in this file — the auto-loaded
control-guard carries the pipeline routing, and each skill's body covers its own
phase. The canonical copy lives at `.agent/protocols/workflow-skills.md`; a CI
drift check fails if any embedded copy diverges. Edit this file and re-sync
every workflow skill together — never one in isolation. Nothing platform-specific
(tool names, hook mechanics, plugin paths) and nothing phase-specific belongs in
the embedded part; that lives in each skill outside the managed core.

## Division of labor (non-negotiable)

Skills and agents manage **semantic workflow** — what to think about, in what
order, and which artifact each phase produces. ctl manages **facts, scope,
evidence, gates, ledgers, and honest disclosure**. A workflow skill never relaxes
a boundary, never declares a task complete, and never substitutes its own
judgement for ctl evidence. Workflow discipline is not proof: it does not replace
gates, audits, reviewer independence, or tamper evidence, and it never creates a
verdict.

## Invariants every phase honors

- Produce **artifacts, not claims**. "Done" is an evidence artifact ctl can see,
  never an assertion — "where is the evidence?"
- Keep **draft separate from confirmed basis**; disclose open uncertainty rather
  than hiding it.
- **Red before green**: no green claim without prior red evidence for the same
  behavior.
- **No fix before a reproduction loop.**
- **Architecture review is read-only**; a refactor needs a fresh governed task.
- External workflow inspiration is **L0 reference material** (see Provenance) —
  never an authority, never vendored as an active control.
<!-- ctl:workflow-core:end -->

*The phase map, frameworks, and provenance are reference material in `.agent/protocols/workflow-skills.md` — not embedded here. The auto-loaded control-guard carries the pipeline routing; this skill's body covers its own phase.*

## Station contract

- **Upstream**: a **confirmed** PRD from `ctl-to-prd`, or — for a single obvious task — a confirmed alignment note from `ctl-grill-with-spec`.
- **Produces**: task proposals for control-guard to `ctl task create` (provenance recorded, never gates).
- **Downstream**: execution under control-guard (`ctl-tdd-loop`), then wrap-up (`ctl-spec-update`).

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

**AFK vs HITL.** **AFK** = deterministic, low-risk, fully gated, safe unattended; **HITL** = needs a decision, touches a protected path via exception, or carries unresolved risk. When unsure, label HITL.

### Rules

- Vertical slices only; split anything too big to finish inside one boundary.
- Each task is independently verifiable on its own evidence.
- Non-overlapping `write_allow` across siblings — overlap forces sequencing; keep it minimal, never broad "to be safe".
- Label honestly: don't mark a risky task AFK to avoid asking.
- **Do not create tasks that bypass protected-path controls** (`.git/`, `events.jsonl`, `schemas/`, `Cargo.toml`).
- **Do not synthesize completed ctl events from a PRD/plan** — only real execution + evidence may close a task.

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

## OMP Integration (platform-specific)

Proposals are notes; the real `ctl task create` is dispatched by control-guard and gated
by the OMP PreToolUse hook. Use `ctl task create --dry-run` to preview a boundary, and
`ctl board` to check sibling tasks for write-scope overlap before creating. Record PRD
provenance on the created task with `ctl brainstorm` (record-only).
