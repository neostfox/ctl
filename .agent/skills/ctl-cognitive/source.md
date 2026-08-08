---
name: ctl-cognitive
description: "Orchestrates the cognitive + knowledge layer. Triggers when: a task carries brainstorm provenance (divergence/convergence artifacts), open uncertainties (proceeding on unverified assumptions), or research findings; or when a durable verified fact worth keeping is discovered. Records canonical cognitive state via ctl (brainstorm/uncertainty/research) and manages the non-canonical knowledge base via the scripts/knowledge.py companion. Do NOT trigger for: routine implementation (just write code), or governance gates (control-guard)."
---


The workflow-side orchestrator for ctl's two-tier cognitive/knowledge model. ctl's
Rust layer owns the **canonical** appends (the task ledger); this skill decides
**when and why** to record, and manages the **non-canonical** knowledge base via
the companion script. Nothing here evaluates quality or replaces review — it
records and discloses, leaving judgement to humans and the hard gates.

## The two tiers (critical distinction)

| Tier | Carrier | Writes via | What it holds |
|---|---|---|---|
| Canonical (task ledger) | `.ctl/tasks/<id>/events.jsonl` | `ctl brainstorm/uncertainty/research record` | what THIS task carries — provenance, unknowns, research |
| Non-canonical knowledge | `.ctl/facts.jsonl`, `~/.ctl/memory/` | `scripts/knowledge.py` (companion) | cross-task, agent-owned verified facts + global memory |

Only ctl appends canonical events (the core invariant — external actors cannot).
The knowledge base is evidence ctl never owns; the companion script manages it.

## When to record canonical cognitive state

- **Brainstorm provenance** — after divergence (brainstorm/PRD) or convergence
  (alignment) thinking that shaped the task's objective/scope:
  `ctl brainstorm record --id <task> --convergence-path <artifact> [--divergence-path <artifact>]`.
  Attach a critic artifact that reshaped the design with
  `ctl brainstorm attach-critic`; skip with a reason via `skip-critic` when there
  was no meaningful critic step.
- **Uncertainties** — the moment the task proceeds on an unverified assumption,
  record it: `ctl uncertainty record --id <task> --id <u-id> --question "..."`.
  Resolve with `--disposition resolved --evidence <ref>` once externally proven,
  or `accepted_as_assumption` to proceed visibly unresolved. An uncertainty may
  never be resolved by model assertion alone.
- **Research artifacts** — for a `research`-kind task, record the finding:
  `ctl research record --id <task> --artifact <path> --kind findings|experiment|recommendation`.
  ctl hashes the artifact; research tasks complete via evidence + uncertainty
  disposition, not code.

Record close to the moment the cognition happens — provenance and uncertainties
degrade if reconstructed after the fact.

## When to capture non-canonical knowledge

- **Add a fact** — when you discover a durable, VERIFIED fact with provenance (a
  fact without a source is an opinion): `scripts/knowledge.py fact add --statement
  "..." --source <file:line|command|url> --category <boundary|architecture|gotcha|...>`.
  Provenance is required.
- **Consult before work** — at task start, search for relevant prior knowledge:
  `scripts/knowledge.py fact list --search <query>` (or `--category <c>`).
- **Promote** — when a fact is durable enough to belong in curated spec, copy it
  into a spec markdown file: `scripts/knowledge.py fact promote --id F-NNN --to
  backend/<file>.md`. The raw fact stays in the store; this is the processed copy.
- **Memory hygiene** — periodically check that global memory
  (`~/.ctl/memory/*.md`) isn't leaking one repo's specifics into every session:
  `scripts/knowledge.py memory verify` (advisory, warns only).

## Discipline

- Record-and-disclose only. These commands never relax a boundary, never declare
  a task complete, and never replace `ctl-review` or the finish interlock.
- Canonical recording is per-task; the knowledge base is cross-task. Don't dump
  task-local scratch into facts — facts are reusable across tasks.
- If unsure whether something is canonical (task ledger) or knowledge (facts
  store): if it describes THIS task's reasoning → canonical; if it's a reusable
  verified truth about the codebase → knowledge.
