---
name: ctl-grill-with-spec
description: "Align before building — the single entry to the alignment station. Grills an ambiguous, broad, multi-option, or high-risk request from first principles via a micro-decision interview: one question at a time, each with a recommended answer; facts come from the repo, direction comes from the user; nothing is built until the user confirms shared understanding. Produces a confirmed alignment note at .ctl/spec/alignment/ that feeds ctl-to-prd (and, only when the user confirms, a domain/ADR note) — never a claim of truth. Absorbs the retired ctl-brainstorm. Triggers when: the request is vague, too broad, has multiple valid approaches, is high-risk, or likely to produce the wrong thing; also on /ctl-new. Do NOT trigger for: an already well-scoped request (go to ctl-to-prd or ctl-to-tasks), a trivial single-file edit, code review (ctl-review), or debugging (ctl-diagnose)."
---


## Station contract

- **Upstream**: control-guard triage — any non-trivial request enters here first.
- **Produces**: an alignment note at `.ctl/spec/alignment/<yyyy-mm-dd>-<slug>.md`
  (status: `draft` until the user confirms → `confirmed`), registered as
  brainstorm provenance on the eventual task (record-only — never gates).
- **Downstream**: `ctl-to-prd` consumes the **confirmed** alignment note. For a
  single obvious task, control-guard may take the converged proposal straight to
  `ctl task create` — the alignment note still carries the provenance.

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

**Challenge inherited assumptions.** For each assumption ask: *are we doing this
because the domain requires it, or because the existing architecture/framework
suggests it?* Strike or downgrade anything that is convention masquerading as a
constraint.

**Outputs are artifacts, not truth.** A grill records what you currently believe
and why — it never asserts the answer is correct. `ctl-to-prd` turns confirmed
alignment into a PRD; unconfirmed items travel forward as OpenUncertainty, never
silently resolved.

### Where artifacts go

- The alignment note: `.ctl/spec/alignment/<yyyy-mm-dd>-<slug>.md` (spec tier —
  writable under the gate; mark `status: draft` until the user confirms).
- Working notes once a task exists: `.ctl/tasks/<task-id>/grill.md` (inside the
  active task's `write_allow`).
- A crystallized domain term or decision **only when the user confirms it**:
  `.ctl/spec/domain.md` or `.ctl/spec/adr/ADR-xxxx.md`. Do **not** write a
  domain/ADR doc on your own judgement — an ADR records a *confirmed* decision,
  not a draft thought.

### Quality bar

- Every "fact" cites where it came from; unconfirmed beliefs are labelled
  assumptions, not facts.
- Every repository-answerable question was answered by inspection, not asked;
  every direction-shaping decision was confirmed by the user, not assumed.
- At least one inherited assumption was challenged and resolved (kept / struck).
- Non-goals are explicit, not implied.
- Unknowns are disclosed, not buried; the riskiest one names the experiment that
  would settle it.
- write_allow in any proposal is the smallest set that lets the work happen.

### Anti-patterns

- ❌ Asking the user something the repository already answers.
- ❌ Deciding a trade-off, priority, or acceptance bar for the user because you
  were confident.
- ❌ Multiple questions in one message, or a question without a recommended
  answer.
- ❌ Building, or creating the implementation task, before the user confirms.
- ❌ Presenting an assumption as an observed fact.
- ❌ Writing a domain/ADR doc without user confirmation or outside write scope.
- ❌ Treating the grill's conclusions as proven rather than as artifacts.

<!-- integration:omp -->

The alignment station's single entry (the old `ctl-brainstorm` / `/ctl-new` is an
alias of this skill). Run the interview with the host's question UI when
available; record which cognitive artifacts the eventual task derived from with
`ctl brainstorm` provenance (record-only — it never gates create/finish and makes
no claim about thinking quality). Writing the alignment note targets
`.ctl/spec/alignment/` (spec tier — writable); `grill.md` or an ADR must fall
inside the active task's `write_allow`, or the OMP PreToolUse ctl gate records
(and for protected paths blocks) it. Hand the confirmed note to `ctl-to-prd`; a
durable lesson to `/ctl-spec-update`.
<!-- integration:opencode -->

The alignment station's single entry (absorbs `ctl-brainstorm`). Record the
cognitive artifacts the eventual task derived from with `ctl brainstorm`
provenance (record-only — never gates, no quality claim). The alignment note
targets `.ctl/spec/alignment/` (spec tier — writable); `grill.md` or an ADR is a
mutating write gated by `.opencode/plugins/ctl-gate.ts`. Hand the confirmed note
to `ctl-to-prd`; a durable lesson to `ctl-spec-update`.

**Recommended role** (autonomous dispatch — see control-guard): `explore` for the
read-only investigation and alignment; `designer` when authoring `grill.md` or an ADR
inside an active task's scope. `explore` is the only read-only role.
<!-- integration:claude -->

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
