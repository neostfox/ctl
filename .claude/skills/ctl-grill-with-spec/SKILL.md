---
name: ctl-grill-with-spec
description: "Align before building — the single entry to the alignment station. Grills an ambiguous, broad, multi-option, or high-risk request into a confirmed alignment note via a micro-decision interview. Triggers when: the request is vague, too broad, has multiple valid approaches, is high-risk, or likely to produce the wrong thing; also on /ctl-new. Do NOT trigger for: an already well-scoped request (go to ctl-to-prd or ctl-to-tasks), a trivial single-file edit, code review (ctl-review), or debugging (ctl-diagnose)."
---

# ctl-grill-with-spec (Claude Code)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-grill-with-spec/source.md` by `ctl skills sync`. Claude Code-specific mechanics live after the core.

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

- **Upstream**: control-guard triage — any non-trivial request enters here first.
- **Produces**: alignment note at `.ctl/spec/alignment/<yyyy-mm-dd>-<slug>.md` (`draft` → `confirmed`); registered as brainstorm provenance (record-only).
- **Downstream**: `ctl-to-prd` consumes the **confirmed** note; for a single obvious task, control-guard may go straight to `ctl task create`.

## The grill (alignment phase body)

**Facts from the repo, direction from the user.** Anything the repository can
answer, read — code, tests, configs, specs, task history; never ask the user for
it. But scheme trade-offs, priorities, scope boundaries, risk tolerance, and
acceptance criteria are the **user's to decide — confirm them even when you are
confident**, by proposing an answer, not by staying silent.

### The interview loop (micro-decisions)

Interview relentlessly but narrowly, one micro-decision at a time:

- Ask the **single highest-value open question**, then wait.
- Every question carries: the decision needed · why it matters · **your
  recommended answer** · the trade-off if the user chooses otherwise.
- Prefer concrete options over open-ended prompts; never ask process questions
  ("should I search the code?") — just do the work.
- Stop when both sides confirm shared understanding of goals, constraints, and
  approach.
- **Do not build until the user confirms the shared understanding.**

### Diverge first when the request is broad

When the goal is wide or multiple approaches are valid: stress-test assumptions,
sketch 2–3 candidate approaches with trade-offs and a recommendation, name what
is in and out of scope, and split a large effort into independently verifiable
child tasks with **non-overlapping write_allow** (overlap forces sequencing —
see control-guard).

### The alignment note

| Field | What it captures |
|---|---|
| Observed facts | what you actually read or ran (cite the source) |
| Declared rules | invariants the project states (specs, schemas, guides) |
| Assumptions | beliefs you are carrying that are not yet confirmed |
| Irreducible constraints | what cannot change (domain, physics, contracts) |
| User goals | the outcome that must be true when done |
| Non-goals | what is explicitly out of scope |
| Decisions | each micro-decision: question · recommendation · what the user chose |
| Unknowns | unresolved questions, ranked by how much they could change scope |
| Minimum viable experiment | the smallest probe that would confirm direction |

When converging directly to tasks, append the task proposal fields (objective ·
read scope · minimal write_allow · gates · risks) for control-guard.

**Challenge inherited assumptions.** For each assumption ask: *domain requirement, or convention from the existing architecture/framework?* Strike anything that is convention masquerading as a constraint.

### Where artifacts go

- The alignment note: `.ctl/spec/alignment/<yyyy-mm-dd>-<slug>.md` (spec tier —
  writable under the gate; mark `status: draft` until the user confirms).
- Working notes once a task exists: `.ctl/tasks/<task-id>/grill.md` (inside the
  active task's `write_allow`).
- A crystallized domain term or decision **only when the user confirms it**:
  `.ctl/spec/domain.md` or `.ctl/spec/adr/ADR-xxxx.md`. Do **not** write a
  domain/ADR doc on your own judgement — an ADR records a *confirmed* decision,
  not a draft thought.

### Anti-patterns

- ❌ Asking the user something the repository already answers.
- ❌ Multiple questions in one message, or a question without a recommended answer.
- ❌ Building, or creating the implementation task, before the user confirms.
- ❌ Writing a domain/ADR doc without user confirmation or outside write scope.

## Claude Code Integration (platform-specific)

The alignment station's single entry. Run the interview loop with
`AskUserQuestion` — one micro-decision per call, the recommended answer listed
first and marked "(Recommended)". Record which cognitive artifacts the eventual
task derived from with `ctl brainstorm` provenance (record-only — never gates
create/finish). The alignment note targets `.ctl/spec/alignment/` (spec tier —
writable under the gate); `grill.md` or an ADR must fall inside the active
task's `write_allow`. Read-only investigation can be dispatched to a subagent
(built-in `Explore`, `claude-code-guide`); keep writes inline so they carry the
task's `CTL_TASK_ID` binding. Hand the confirmed note to `ctl-to-prd`; a durable
lesson to `/ctl-spec-update`.
