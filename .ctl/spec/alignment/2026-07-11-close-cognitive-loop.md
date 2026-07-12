# Alignment — Close the Cognitive Loop (prd-plan-v1)

> status: confirmed — user accepted the converged proposal; implemented as prd-plan-v1.
> provenance: ctl-grill-with-spec micro-decision interview (2026-07-11)

## Observed facts (read from the repo)

- **M0–M6 + M-a…M-g + V1 cognitive layer all shipped.** The V1 layer is
  explicitly record-and-disclose: never gates, never scores, never renders an
  epistemic verdict (ROADMAP §"V1 认知层"; EPISTEMIC_CONTROL.md).
- **The cognitive loop has exactly one mechanical break: `prd plan`.** The
  pipeline is grill(alignment note) → PRD(`ctl prd init` template) → **tasks**
  → TDD → handoff. grill/PRD/TDD/handoff all work; the PRD `## Tasks` section
  cannot become governed tasks today except by manual `ctl task create` per item.
- **`prd plan` is a designed-but-unimplemented gap, not a new idea.** The PRD
  template (`src/cli/mod.rs` `PRD_TEMPLATE`, L3016) carries an HTML comment:
  "Conventions a future `prd plan` parser will rely on." A test
  (`prd_template_holds_the_parseable_convention`, L7822) pins the format.
  ROADMAP L860/L874 list `prd plan` under "已知缺口."
- **The decomposition intelligence already happened upstream.** The PRD
  `## Tasks` section is authored during the ctl-to-prd/ctl-to-tasks step and
  confirmed by the human. `prd plan` mechanically executes the confirmed plan —
  no model judgement at plan time. This fits "确定性优先于智能程度."
- **All primitives exist**: `create_task(id, CreateTaskInput{...depends_on...})`
  (application L299); `record_brainstorm_artifacts(task, brainstorm, divergence,
  convergence)` (L999) — PRD = convergence, alignment note = divergence;
  `detect_write_scope_overlap` (reused by schedule run / board) for cross-task
  overlap; `ctl task create` already nudges "record alignment provenance" (L2300)
  — `prd plan` would *do* it instead of hinting.
- **No markdown dependency; format is rigid** (`- id:` starts an item, indented
  `key: value`). A minimal line parser needs no new crate — respects DEP rules.

## Declared rules (inviolable)

- `ctl` never spawns an executor, never writes code. `prd plan` only calls
  existing `create_task` + `record_brainstorm_artifacts` — both append events
  via the normal validated path.
- Every `ctl task create` from `prd plan` still goes through the PreToolUse gate
  (boundary / scope / protected-path). No bypass.
- All write ops support `--dry-run` (ROADMAP "体验标准").
- A `draft` PRD must not generate durable tasks unless `--dry-run`
  (ctl-to-prd skill hard rule).
- PRD stays an agent-readable artifact: no PRD events, no PRD subsystem, never
  gates a task (ctl-to-prd skill: "V1 is an agent-readable artifact workflow
  only").
- Dependencies: no new crates beyond the allowed set (clap/serde/anyhow/sha2/libc).

## Assumptions (not yet confirmed)

- A simple `> Status: confirmed` blockquote line in the PRD is sufficient to
  encode status (matches the template's existing `>` style; no frontmatter dep).
- `depends-on` as a new *optional* PRD field is the right place to wire M-d
  dependency edges (rather than inferring from ordering or from write-overlap).
- `prd plan` creating tasks only to `Planning` state (no auto-ready/start) is
  the right automation ceiling.

## Irreducible constraints

- The PRD `## Tasks` format is pinned by a test; extending it requires updating
  that test in lockstep.
- `create_task` already rejects duplicate ids — re-running `prd plan` on the
  same PRD fails fast on the first existing task (correct; `--dry-run` previews).
- Provenance is record-only — it never gates create/finish and never claims
  thinking quality or reviewer independence.

## User goals

- A confirmed PRD's `## Tasks` section becomes governed ctl tasks in one
  command, each still gated, with provenance wired back to the PRD + alignment.
- The full alignment→PRD→task→progress chain is observable at a glance.

## Non-goals (V1)

- PRD event subsystem / PRD ledger (PRD never enters events.jsonl, never gates).
- Model-based decomposition at plan time (decomposition is a PRD-authoring step).
- Auto ready/start/finish (plan stops at Planning).
- PRD→task strong-binding gate (provenance is record-only).
- A PRD parser for arbitrary markdown — only the rigid `## Tasks` convention.

## Decisions (micro-decisions, confirmed by user)

1. **Direction**: "close the cognitive loop" (chosen over: expand deterministic
   brain / feed-smarter-to-model / model-as-advisory). Rationale: it is the
   designed-but-unimplemented seam; fully deterministic; fits first principle.
2. **Scope**: plan + observable loop (chosen over: plan-only / + feedback wiring).
   Adds `prd validate` + `prd status` alongside `prd plan`.

## Design (converged)

### PRD template extension (`PRD_TEMPLATE` + pinned test)
- Add optional `read-scope:` (defaults to write-allow) and `depends-on:`
  (comma-separated task ids) to each task item.
- Add a `> Status: draft` line near the top; human flips to `confirmed`.

### `ctl prd plan --file <prd>.md [--dry-run]`
1. Parse `## Tasks` (id/objective/write-allow/gates/read-scope?/depends-on?).
2. **Status gate**: refuse unless `confirmed` (`draft` → `--dry-run` only;
   `superseded` → refuse outright).
3. **Implicit validate** (see below); fail fast, create nothing on any problem.
4. Per task: `create_task` (gated) + `record_brainstorm_artifacts` (PRD =
   convergence, alignment note = divergence).
5. `--dry-run`: print each would-be task (id/boundary/gates/deps), persist nothing.
6. Output: N planned + suggested next (`ctl task ready --id <first>` / `ctl board`).

### `ctl prd validate --file <prd>.md [--json]` (read-only)
Format convention + per-task boundary normalization + protected-path check +
cross-task write-allow overlap (`detect_write_scope_overlap`) + status. The
validation core that `prd plan` runs implicitly, exposed standalone.

### `ctl prd status --file <prd>.md [--json]` (read-only, observable loop)
PRD title/status + per task: id → current phase → provenance (brainstorm chain
back to PRD/alignment) + one summary line (X/N completed).

### Layering
- Parse + validate: pure functions (no IO) — application layer (or a small
  helper module). File read is the only IO; orchestration calls existing
  `create_task` / `record_brainstorm_artifacts`.
- No domain-reducer change, no schema change, no new event types.

## Files expected to change

- `src/cli/mod.rs`: `PrdCommands` gains `Plan`/`Validate`/`Status`; extend
  `PRD_TEMPLATE`; update `prd_template_holds_the_parseable_convention` test;
  new parse/validate helpers + command handlers + tests.
- `src/application/mod.rs`: pure parse + validate helpers if they belong above
  the CLI (decision deferred to implementation; parser may stay in CLI).

## Minimum viable experiment

Write `prd plan --dry-run` against a hand-filled PRD fixture (3 tasks, one with
`depends-on`, one overlapping write-allow that validate must catch). Proves:
parse correctness, status gate, overlap detection, dry-run safety — before any
task is ever created.

## Unknowns (ranked)

1. **(highest)** Should `prd plan` auto-resolve `depends-on` to ids *within this
   PRD* only, or also allow external already-existing task ids? Proposal:
   allow both (external deps assumed satisfied, mirroring schedule-run's existing
   warning at L5491). Low risk.
2. Status encoding (`>` line vs frontmatter). Proposal: `>` line (no dep).
   Trivially reversible.
3. Whether `prd status` should also surface drift/next-action per task. Proposal:
   no — keep it a provenance/phase view; drift already has its own surface.
