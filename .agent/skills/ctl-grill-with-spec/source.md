---
name: ctl-grill-with-spec
description: "Align before building — the single entry to the alignment station. Grills an ambiguous, broad, multi-option, or high-risk request into a confirmed alignment note via a micro-decision interview. Triggers when: the request is vague, too broad, has multiple valid approaches, is high-risk, or likely to produce the wrong thing; also on /ctl-new. Do NOT trigger for: an already well-scoped request (go to ctl-to-prd or ctl-to-tasks), a trivial single-file edit, code review (ctl-review), or debugging (ctl-diagnose)."
---


## Station contract

- **Upstream**: control-guard triage — any non-trivial request enters here first.
- **Depth by fit**: classify on entry — `trivial` (control-guard edits directly,
  grill skipped); `single-task converged` (clear objective, ≤2 write_allow, no
  design divergence → grill degrades to a 5-line intent confirm, then straight
  to `ctl task create`); `multi-option / ambiguous / high-risk` (full interview
  below). The user can always request the full interview.
- **Produces**: alignment note at `.ctl/spec/alignment/` (`draft` → `confirmed`);
  on the single-task path the note lives in-conversation — provenance is
  optional, recorded post-create if desired (record-only).
- **Downstream**: `ctl-to-prd` consumes a **confirmed** note ONLY when multiple
  durable tasks are needed; a single converged task skips PRD and goes to
  `ctl task create` directly.

## The grill (alignment phase body)

**Facts from the repo, direction from the user.** Anything the repository can
answer, read — code, tests, configs, specs, task history; never ask the user for
it. But scheme trade-offs, priorities, scope boundaries, risk tolerance, and
acceptance criteria are the **user's to decide — confirm them even when you are
confident**, by proposing an answer, not by staying silent.

### The interview loop — design tree + frontier (per round)

Interview relentlessly, but in **rounds**, not one question at a time. Model the
effort as a **design tree**: each decision branches into the decisions that hang
off it. The **frontier** is every decision whose prerequisites are already
settled — the questions you can ask *now* without guessing at answers you have
not heard yet.

- Each round, ask the **whole frontier together**: number each question
  (Q1, Q2, …) and attach the decision needed · why it matters · **your
  recommended answer** · the trade-off if the user chooses otherwise.
- A question whose answer depends on another **still-open** question belongs to
  a **later round** — never ask upstream of an unsettled prerequisite.
  **Independent** decisions batch into one round (this is the point of the
  frontier: fewer round-trips); **dependent** ones never do.
- Each round's answers reshape the tree and push the frontier outward. Recompute
  it and ask the next round. Prefer concrete options over open-ended prompts;
  never ask process questions ("should I search the code?") — just do the work.
- The session converges when the **frontier is empty**: every branch visited,
  nothing left silently assumed. On the single-task path, take the converged
  proposal straight to `ctl task create` — do not force a PRD or a full
  alignment-note write-up.

#### Facts are your job, never the user's

When a frontier question needs a fact (code, tests, configs, specs, task
history), **find it yourself** — never ask the user for anything the repository
can answer. Dispatch a **read-only** sub-agent (`Explore` / `scout` / `explore`,
per platform) to look it up, and **don't block on it**: a running lookup is an
unsettled prerequisite, so only the questions downstream of it wait — ask the
rest of the frontier now. The *decisions* (scope, priorities, risk tolerance,
acceptance criteria) are the user's — put each to them and wait.

#### "Don't build until consensus" — discipline, NOT an enforced gate

Present the populated design tree and **stop** for the user to confirm. Be blunt
about how weak this is: ctl does **not** hard-block a write before consensus.
Observe-mode *records* an ungoverned or early write to `.ctl/decisions.jsonl`;
it does not prevent one (only protected paths, dependency step-ups, held tasks,
and cross-task overlap are hard-denied). So "wait for confirmation" is **workflow
discipline you are trusted to follow**, not a gate ctl enforces — and an early
write becomes part of the observation log the review gates audit. Treat it as a
strong norm, and know exactly how soft its enforcement is.

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
The expanded First Principles framework (restate → truths → challenge → build → validate) is in `references/first-principles.md` — load it on demand when the request is vague or a solution feels over-engineered.

### Where artifacts go

- **Single-task path**: the alignment note lives in-conversation during grill
  (no file write up front). Provenance is optional: if you want it recorded,
  write the note to `.ctl/spec/alignment/` and run
  `ctl brainstorm record --id <task-id> --brainstorm <bs-id> --divergence <note-path>`
  after `ctl task create`.
- **Multi-task path**: write `.ctl/spec/alignment/<yyyy-mm-dd>-<slug>.md`
  (spec tier — writable; `status: draft` until confirmed) — `ctl-to-prd` reads it.
- Working notes once a task exists: `.ctl/tasks/<task-id>/grill.md` (inside
  `write_allow`).
- A crystallized domain term or decision **only when the user confirms it**:
  `.ctl/spec/domain.md` or `.ctl/spec/adr/ADR-xxxx.md`.

### Anti-patterns

- ❌ Asking the user something the repository already answers.
- ❌ Multiple DEPENDENT questions in one message (independent decisions may
  batch as one multi-select), or a question without a recommended answer.
- ❌ Building, or creating the implementation task, before the user confirms.
- ❌ Writing a domain/ADR doc without user confirmation or outside write scope.

<!-- integration:omp -->

The alignment station's single entry (successor to the retired `ctl-brainstorm` / `/ctl-new` is an
alias of this skill). Run the interview with the host's question UI when
available; record which cognitive artifacts the eventual task derived from with
`ctl brainstorm` provenance (record-only — it never gates create/finish and makes
no claim about thinking quality). Writing the alignment note targets
`.ctl/spec/alignment/` (spec tier — writable); `grill.md` or an ADR must fall
inside the active task's `write_allow`, or the OMP PreToolUse ctl gate records
(and for protected paths blocks) it. Hand the confirmed note to `ctl-to-prd`; a
durable lesson to `/ctl-spec`.
<!-- integration:opencode -->

The alignment station's single entry (absorbs `ctl-brainstorm`). Record the
cognitive artifacts the eventual task derived from with `ctl brainstorm`
provenance (record-only — never gates, no quality claim). The alignment note
targets `.ctl/spec/alignment/` (spec tier — writable); `grill.md` or an ADR is a
mutating write gated by `.opencode/plugins/ctl-gate.ts`. Hand the confirmed note
to `ctl-to-prd`; a durable lesson to `ctl-spec`.

**Recommended role** (autonomous dispatch — see control-guard): `explore` for the
read-only investigation and alignment; `designer` when authoring `grill.md` or an ADR
inside an active task's scope. `explore` is the only read-only role.
<!-- integration:claude -->

The alignment station's single entry. Run the interview loop with
AskUserQuestion — ask the whole **frontier** (independent decisions only; a
question depending on a still-open one waits for a later round) as one
multi-select call (recommended answer listed
first and marked "(Recommended)". Record which cognitive artifacts the eventual
task derived from with `ctl brainstorm` provenance (record-only — never gates
create/finish). The alignment note targets `.ctl/spec/alignment/` (spec tier —
writable under the gate); `grill.md` or an ADR must fall inside the active
task's `write_allow`. Read-only investigation can be dispatched to a subagent
(built-in `Explore`, `claude-code-guide`); keep writes inline so they carry the
task's `CTL_TASK_ID` binding. Hand the confirmed note to `ctl-to-prd`; a durable
lesson to `/ctl-spec`.
