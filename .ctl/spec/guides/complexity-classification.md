# Complexity Classification

Read this guide when estimating task scope before proposing boundaries.

## Classification Table

Classification decides **which pipeline station a request enters at** (triage →
align → PRD → tasks → execute → wrap-up). Everything above Trivial goes through
a proposal the user confirms before code is written.

| Complexity | Criteria | Action |
|---|---|---|
| **Trivial** | Single-line fix, typo, config change | Skip the pipeline, implement directly (the gate records ungoverned writes) |
| **Simple** | Clear goal, 1-2 files, no ambiguity | Short proposal with a recommendation → user confirms → `ctl task quick` |
| **Moderate** | Multi-file, some ambiguity, cross-layer | Enter the align station light: read specs, 2-3 micro-decisions (each with a recommended answer) → confirmed proposal → task |
| **Complex** | Vague goal, architectural choices, new subsystem | Full `ctl-grill-with-spec` (alignment note → PRD → tasks); do not build until the user confirms |

## Heuristics by task type

| Type | Typical complexity | Key risk |
|---|---|---|
| Bug fix | Simple | Root cause is elsewhere (cross-layer) |
| New feature | Moderate | Scope creep, missing test coverage |
| Refactoring | Moderate | Dependent files missed |
| Config/build | Simple → Complex | High blast radius even for small changes |
| Architecture change | Complex | Milestone scope violation |

## Expansion sweep (for Moderate+)

Before converging on scope:

1. **Future evolution**: What might this become in 1-3 months? Extension points worth preserving?
2. **Related scenarios**: Adjacent flows that should stay consistent?
3. **Failure modes**: Conflicts, offline failure, retries, idempotency, rollback, security boundaries.

What's in MVP → `write_allow`. What's excluded → note in risks.
