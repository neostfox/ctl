---
name: ctl-grill-with-spec
description: "Align before building — the single entry to the alignment station. Grills an ambiguous, broad, multi-option, or high-risk request into a confirmed alignment note via a micro-decision interview. Triggers when: the request is vague, too broad, has multiple valid approaches, is high-risk, or likely to produce the wrong thing; also on /ctl-new. Do NOT trigger for: an already well-scoped request (go to ctl-to-prd or ctl-to-tasks), a trivial single-file edit, code review (ctl-review), or debugging (ctl-diagnose)."
---

# ctl-grill-with-spec (OMP)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-grill-with-spec/source.md` by `ctl skills sync`. OMP-specific mechanics live after the core.

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
- **Depth by fit**: classify on entry — `trivial` (control-guard edits directly,
  grill skipped); `single-task converged` (clear objective, ≤2 write_allow, no
  design divergence → grill degrades to a 5-line intent confirm — objective
  restated · write_allow · gates · the one risk · your recommendation — then
  straight to `ctl task create`); `multi-option / ambiguous / high-risk` (full interview
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

**Reach for First Principles at entry, not at note-time.** When the request
arrives as a *solution* ("add X", "make Y a hard gate"), your first move is to
restate it as the *problem* (FP Step 1) and challenge every inherited assumption
(FP Step 3 — *domain requirement, or convention?*). The full framework (restate
→ truths → challenge → build → validate) is in `references/first-principles.md`
— load it when the request is vague, solution-framed, or feels over-engineered.
It outputs artifacts (a restated problem, ranked assumptions, a minimum viable
experiment), never a verdict.

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
- The session converges when the **frontier is empty** — operationally: no
  remaining decision would change the task proposal you're about to hand to
  control-guard. On the single-task path, the 5-line intent confirm IS the
  converged proposal — take it straight to `ctl task create`; do not force a PRD
  or a full alignment-note write-up.

#### Facts are your job, never the user's

When a frontier question needs a fact (code, tests, configs, specs, task
history), **find it yourself** — never ask the user for anything the repository
can answer. Dispatch a **read-only** sub-agent (`Explore` / `scout` / `explore`,
per platform) to look it up, and **don't block on it**: a running lookup is an
unsettled prerequisite, so only the questions downstream of it wait — ask the
rest of the frontier now. **Distinguish dispatch latency**: for a *slow* lookup
(a multi-file sub-agent scan), fire it and ask the independent frontier in the
same turn — don't block; for an *instant* local lookup (a single file or config
read), do it inline *before* asking, so a fast fact reshapes the question
rather than asking-then-correcting. The *decisions* (scope, priorities, risk tolerance,
acceptance criteria) are the user's — put each to them and wait.

#### "Don't build until consensus" — discipline, NOT an enforced gate

Present the populated design tree and **stop** for the user to confirm. Be blunt
about how weak this is: ctl does **not** hard-block a write before consensus.
Observe-mode *records* an ungoverned or early write to `.ctl/decisions.jsonl`;
it does not prevent one — only the **hard core** is denied (protected paths,
unclassifiable targets, dependency step-ups, held tasks, cross-task overlap,
multi-active ambiguity, destructive git during runs, ctl-unavailable; see
`.ctl/spec/prd/gate-observe-mode.md` for the authoritative, non-exhaustive
list). So "wait for confirmation" is **workflow
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
The framework's note-feeding mapping (truths → constraints, challenge → assumptions, build → approaches, validate → unknowns + experiment) is in `references/first-principles.md`.

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
- ❌ Multiple DEPENDENT questions in one round (independent decisions batch as
  separate single-choice questions in one call — reserve multi-select for the
  rare decision where several options apply at once), or a question without a
  recommended answer.
- ❌ Building, or creating the implementation task, before the user confirms.
- ❌ Writing a domain/ADR doc without user confirmation or outside write scope.

## OMP Integration (platform-specific)

The alignment station's single entry (successor to the retired `ctl-brainstorm` / `/ctl-new` is an
alias of this skill). Run the interview with the host's question UI when
available; record which cognitive artifacts the eventual task derived from with
`ctl brainstorm` provenance (record-only — it never gates create/finish and makes
no claim about thinking quality). Writing the alignment note targets
`.ctl/spec/alignment/` (spec tier — writable); `grill.md` or an ADR must fall
inside the active task's `write_allow`, or the OMP PreToolUse ctl gate records
(and for protected paths blocks) it. Hand the confirmed note to `ctl-to-prd`; a
durable lesson to `/ctl-spec`.
