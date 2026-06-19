---
name: ctl-grill-with-spec
description: "Align before building: grill an ambiguous, broad, or high-risk request from first principles, separating observed facts from declared rules from assumptions, naming irreducible constraints and a minimum viable experiment. Produces alignment artifacts (and, only when scope allows and the user confirms, a domain/ADR note) — never a claim of truth. Triggers when: the request is vague, too broad, high-risk, or likely to produce the wrong thing, before a PRD or implementation. Do NOT trigger for: an already well-scoped request (go to ctl-to-prd or ctl-to-tasks), code review (ctl-review), or debugging (ctl-diagnose)."
---


## The grill (first-principles phase body)

Evidence before questions: anything the repository can answer, read — code,
tests, configs, specs, task history. Only ask the user for what the repo cannot
answer (intent, preference, risk tolerance). Then assemble the alignment artifact:

| Field | What it captures |
|---|---|
| Observed facts | what you actually read or ran (cite the source) |
| Declared rules | invariants the project states (specs, schemas, guides) |
| Assumptions | beliefs you are carrying that are not yet confirmed |
| Irreducible constraints | what cannot change (domain, physics, contracts) |
| User goals | the outcome that must be true when done |
| Non-goals | what is explicitly out of scope |
| Unknowns | unresolved questions, ranked by how much they could change scope |
| Minimum viable experiment | the smallest probe that would confirm direction |

**Challenge inherited assumptions.** For each assumption ask: *are we doing this
because the domain requires it, or because the existing architecture/framework
suggests it?* Strike or downgrade anything that is convention masquerading as a
constraint.

**Outputs are artifacts, not truth.** A grill records what you currently believe
and why — it never asserts the answer is correct. The next phase (`ctl-to-prd`)
turns confirmed alignment into a PRD; unconfirmed items travel forward as
OpenUncertainty, never silently resolved.

### Where artifacts go (only within scope, only when confirmed)

- Working notes: `.ctl/tasks/<task-id>/grill.md` (inside the active task's
  `write_allow`).
- A crystallized domain term or decision **only when the user confirms it**:
  `.ctl/spec/domain.md` or `.ctl/spec/adr/ADR-xxxx.md`. Do **not** write a
  domain/ADR doc on your own judgement or outside the task's write scope — an ADR
  records a *confirmed* decision, not a draft thought.

### Quality bar

- Every "fact" cites where it came from; unconfirmed beliefs are labelled
  assumptions, not facts.
- At least one inherited assumption was challenged and resolved (kept / struck).
- Non-goals are explicit, not implied.
- Unknowns are disclosed, not buried; the riskiest one names the experiment that
  would settle it.

### Anti-patterns

- ❌ Asking the user something the repository already answers.
- ❌ Presenting an assumption as an observed fact.
- ❌ Writing a domain/ADR doc without user confirmation or outside write scope.
- ❌ Treating the grill's conclusions as proven rather than as artifacts.

<!-- integration:omp -->

Invoke during scoping (often right after `/ctl-new` / `ctl-brainstorm`). Record
which cognitive artifacts the eventual task derived from with `ctl brainstorm`
provenance (record-only — it never gates create/finish and makes no claim about
thinking quality). Writing `grill.md` or an ADR is a normal mutating write: it must
fall inside the active task's `write_allow`, or the OMP PreToolUse ctl gate blocks it.
Hand confirmed alignment to `ctl-to-prd`; hand a durable lesson to `/ctl-spec-update`.
<!-- integration:opencode -->

Invoke during scoping, alongside `ctl-brainstorm`. Record the cognitive artifacts the
eventual task derived from with `ctl brainstorm` provenance (record-only — never gates,
no quality claim). Writing `grill.md` or an ADR is a mutating write gated by
`.opencode/plugins/ctl-gate.ts`: it must fall inside the active task's `write_allow` or
the plugin throws. Hand confirmed alignment to `ctl-to-prd`; a durable lesson to
`ctl-spec-update`.

**Recommended role** (autonomous dispatch — see control-guard): `explore` for the
read-only investigation and alignment; `designer` when authoring `grill.md` or an ADR
inside an active task's scope. `explore` is the only read-only role.
<!-- integration:claude -->

Invoke during scoping. Record which cognitive artifacts the eventual task derived from with `ctl brainstorm` provenance (record-only — never gates create/finish). Writing `grill.md` or an ADR is a mutating write: it must fall inside the active task's `write_allow`, or the Claude Code PreToolUse ctl gate (`.claude/hooks/ctl-gate.py`) blocks it. Read-only investigation can be dispatched to a subagent (built-in `Explore`, `claude-code-guide`); keep writes inline so they carry the task's `CTL_TASK_ID` binding. Hand confirmed alignment to `ctl-to-prd`; a durable lesson to `/ctl-spec-update`.
