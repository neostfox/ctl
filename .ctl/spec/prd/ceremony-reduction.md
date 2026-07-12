# PRD — Ceremony Reduction (token / latency diet)

Status: confirmed (2026-07-11) — direction approved; scheme 6 resolved (see
Resolved uncertainties).
Provenance: first-principles analysis of the skill/protocol/gate surface, confirmed
against source (`src/infrastructure/skill_sync.rs`, `src/infrastructure/skills.rs`,
`src/application/mod.rs`, `.agent/protocols/*.md`, `.omp/skills/*/SKILL.md`).

## ObservedBasis (read from the repo)

- **Managed core is embedded verbatim N times.** `skill_sync.rs::compose` (L127-161)
  wraps the full 93-line canonical `workflow-skills.md` core into every generated
  skill × every platform. 7 skills × 3 platforms = 21 copies; each `ctl-grill-with-spec`
  SKILL.md is 218 lines, of which ~104 are the verbatim core. `control-guard.md`
  (170 lines) is embedded in its skill. Drift checks (`workflow_protocol_sync`,
  `control_guard_protocol_sync` in `skills.rs`) enforce byte-identity.
- **Three-way description overlap.** The pipeline map is restated in:
  `control-guard.md` Pipeline Routing (L142-160), `workflow-skills.md` Phase Map
  (L28-57), and each skill's Station Contract + `control-guard/SKILL.md` OMP
  Integration (L186-213).
- **Everything-non-trivial is pushed through grill.** `control-guard.md` Pipeline
  Routing routes "everything else" to the align station; grill runs a one-question-
  at-a-time micro-decision interview (`grill SKILL.md` L116-122) — 3-5 decisions =
  3-5 model+user round-trips.
- **Each phase produces a persisted artifact the next station reads back.** grill →
  `.ctl/spec/alignment/*.md`; PRD; task proposals. On the single-task path the
  alignment note is provenance (record-only, never gates — `grill SKILL.md` L111)
  yet still pays write + downstream read.
- **Evidence binds to the committed tree, not the working tree.**
  `application/mod.rs` finish interlock (L555) uses `head_tree_hash()` = `HEAD^{tree}`;
  gate evidence (L570) and the completion audit (L710) both compare their recorded
  `tree_hash` to the current HEAD tree. So **any commit after an audit invalidates
  it.** Canonical order must be strictly `commit → gate → audit → finish`, but
  `control-guard.md` (L86-90) does not pin gate/audit ordering — agents walk the
  wrong order and hit `rerun gates and re-audit` loops.
- **Completion audit is hard and reviewer-isolated.** `application/mod.rs` L679-742
  requires a fresh passing audit after the last submit; M6 refuses implementer
  self-approval. `ctl-review` (76-line body) directs the reviewer to pre-read 4
  external rubric files (review-contract / decay-risks / test-decay-risks /
  failure-diagnosis) before working.
- **`prd plan` is unimplemented** (`ctl prd` exposes only `init`). Task decomposition
  is manual `ctl task create`.

## ConfirmedBasis (user-approved direction)

- Reduce ceremony tax: redundant re-declaration, forced multi-round grill on
  single-task work, stale-evidence loops. Architecture invariants (append-only
  events, pure domain, protected-path hard-deny, reviewer ≠ implementer, evidence
  bound to committed tree) are **not** in scope.
- Execution order: low-risk pure-dedup first (1,2,8), then flow slimming (3,4,5),
  then stale ordering (7), then audit tiering (6, light — confirmed below).
- Scheme 6 accepted: a narrow-scope `light` audit tier (write_allow ≤ N files,
  non-protected, non-schema/deps), reviewer-isolated + hard verdict but skipping
  the R1-R6/T1-T6 decay scan and Health Score.

## Schemes

Each scheme: root-cause anchor → change → risk.

### 1. Managed-core tiering (full once, reference elsewhere)  [LOW risk]
Root: `skill_sync.rs::compose` L144-147 inlines the full core per skill.
Change: split `workflow-skills.md` into (a) **Invariants** (~20 lines, L74-95) —
still fully embedded per skill (self-contained worth it); (b) Phase Map /
Frameworks / Provenance (~70 lines) — given in full only via control-guard
auto-load, referenced by one line in workflow skills. `compose()` gains an
`embed_level` param; drift check narrows to "invariants byte-identical +
reference line present".
Yield: ~88 lines × 21 generated skills.

### 2. Drop control-guard OMP Integration restatement  [LOW risk]
Root: `control-guard/SKILL.md` L186-213 restates pipeline routing from the core.
Change: keep only platform mechanics (hook path, gated-tool list, subagent
timeout, session events); delete the workflow-routing restatement (~15 lines).

### 3. Pipeline single-task pass-through default  [LOW risk]
Root: `control-guard.md` L131-160 routes all non-trivial work to grill.
Change: add a classifier before grill: `trivial` (direct edit, exists) /
`single-task converged` (clear objective, ≤2 write_allow files, no design
divergence → `ctl task quick`, grill degrades to a 5-line intent confirm) /
`multi-option|ambiguous|high-risk` (full grill). Escape hatch: user can always
request full grill.

### 4. Grill converge-and-exit + batch confirm  [LOW risk]
Root: `grill SKILL.md` L116-122 mandates one-question-at-a-time.
Change: (a) on convergence, the Station Contract single-task path (L113-114)
becomes the **default** — straight to `ctl task create`, note as attached
provenance, no forced PRD→tasks; (b) independent micro-decisions batch into one
multi-select ask (recommended answer preserved).

### 5. Deferred provenance persistence  [LOW risk]
Root: every station persists an artifact the next reads back.
Change: distinguish **provenance artifacts** (write-only, e.g. a single-task
alignment note) from **contract artifacts** (multi-task must-read, e.g. PRD).
Provenance artifacts are produced in-conversation and persisted once at
`task create` as attached provenance, with no downstream read required.

### 6. Completion-audit tiering (light / full)  [HIGH risk — OpenUncertainty]
Root: `application/mod.rs` L679-742 + M6 — every finish requires a fresh
reviewer-isolated audit running the full rubric + Health Score.
Change: declare an audit tier at `task create` (default `full`). `light` still
requires reviewer ≠ implementer + a hard verdict, but runs only the closure
checklist (build/test/lint evidence existence + protected-path + scope
compliance), skipping the R1-R6/T1-T6 decay scan and Health Score. Applies to
write_allow ≤ N files, non-protected, non-schema/deps.
**Tension (needs decision):** reviewer isolation is an architectural invariant.
`light` weakens decay-risk coverage. This is a truth-value trade-off, not pure
optimization.

### 7. Stale-loop: canonical-order atomization  [MED risk]
Root: `application/mod.rs` L555-599, L704-726 bind evidence to HEAD^{tree}; any
post-audit commit invalidates it, but ordering isn't pinned.
Change (no relaxation of the tree/policy-stale logic itself — that guards
"audit one version, commit another"):
- Protocol: pin canonical order `submit → commit → gate run → review accept → finish`
  in `control-guard.md`, state "audit must follow the last in-scope commit".
- CLI: `ctl task finish` pre-flight — before reporting stale, detect whether
  current HEAD^{tree} matches a recorded gate/audit `tree_hash`; if so, surface
  an actionable hint (which commits intervened, whether write-scope changed)
  instead of a bare error.
- Optional (separate milestone): `ctl task seal` fuses commit+gate+audit.

### 8. Inline rubric summaries in skills  [LOW risk]
Root: `ctl-review/SKILL.md` L12-18 directs pre-reading 4 rubric files.
Change: inline a ~15-line compressed rubric (R1-R6 / T1-T6 id + one-liner +
severity) in the skill body; full rubric stays the authoritative source in
spec/guides. Directive becomes "summary for fast triage; re-check full guide on
Critical". Lightweight check that the summary carries all IDs.

### 9. Skill body slimming (de-triple + de-duplicate core)  [LOW risk]
Root: every workflow skill restates each rule three ways (narrative → Quality
bar / Hard rules / Forbidden → Anti-patterns) and re-declares core invariants
in its body. Evidence: tdd-loop Forbidden (L28-34) ≈ Anti-patterns (L41-46);
to-tasks Hard rules (L37-45) ≈ Anti-patterns (L61-67); grill "Challenge
inherited assumptions" (L63-66) duplicates core First Principles; "Outputs are
artifacts not truth" (L68-71) duplicates core "artifacts not claims".
Change — four rules ("state each rule once, where it fits best"):
1. Merge the checklist triplet: keep narrative + a short Anti-patterns list of
   ONLY errors the narrative did not already cover; delete Quality-bar /
   Hard-rules / Forbidden rows that restate the narrative.
2. Delete body text that re-declares a core invariant ("artifacts not claims",
   "red before green", "do not bypass protected paths", "challenge inherited
   assumptions"); keep phase-specific emphasis only.
3. Shrink Station contract to one line each (produces / downstream) — the full
   phase map lives in core.
4. Trim frontmatter description to routing signal (what + when / when-not);
   move "how" into the body.
Yield: ~155 lines across 7 generated source.md (~24%). Embedded skills
(control-guard body, ctl-review, ctl-diagnose, ctl-spec-update, ctl-cli-reference)
and the 850-line ctl-spec-bootstrap get a separate pass (scheme 9b).

## Resolved uncertainties

- **Scheme 6 truth-value trade-off — RESOLVED (2026-07-11, user-confirmed).**
  Accepted: a narrow-scope `light` audit tier. Reviewer isolation + hard verdict
  preserved; the R1-R6/T1-T6 decay scan and Health Score are skipped only for
  write_allow ≤ N files, non-protected, non-schema/deps tasks.

## Tasks (for ctl-to-tasks / manual ctl task create)

- id: ceremony-core-tiering
  objective: Split managed core into invariants (embedded) vs phase-map (referenced); update compose() + drift check.
  read-scope: src/infrastructure/skill_sync.rs, src/infrastructure/skills.rs, .agent/protocols/workflow-skills.md
  write-allow: src/infrastructure/skill_sync.rs, src/infrastructure/skills.rs, .agent/protocols/workflow-skills.md
  gates: cargo_check, cargo_test, cargo_clippy, cargo_fmt_check
  acceptance: workflow_protocol_sync passes; each generated skill ≤~130 lines; invariants still byte-identical across platforms.
  depends-on: ceremony-desc-dedup

- id: ceremony-desc-dedup
  objective: Remove pipeline-routing restatement from control-guard OMP Integration; keep platform mechanics only.
  read-scope: .omp/skills/control-guard/SKILL.md, .agent/protocols/control-guard.md
  write-allow: .omp/skills/control-guard/SKILL.md, .agent/protocols/control-guard.md
  gates: cargo_test
  acceptance: control_guard_protocol_sync passes; OMP Integration carries only hook/tool/timeout/session mechanics.

- id: ceremony-rubric-inline
  objective: Inline compressed R1-R6/T1-T6 summaries into ctl-review; full rubric stays authoritative; add id-presence check.
  read-scope: .omp/skills/ctl-review/SKILL.md, .ctl/spec/guides/decay-risks.md, .ctl/spec/guides/test-decay-risks.md, .ctl/spec/guides/review-contract.md
  write-allow: .omp/skills/ctl-review/SKILL.md, .agent/skills/ctl-review/source.md
  gates: cargo_test
  acceptance: reviewer can work without pre-reading 4 files; summary covers every R/T id; full guides unchanged.

- id: ceremony-pipeline-slim
  objective: Add single-task pass-through classifier; grill converge-and-exit default; batch micro-decisions; deferred provenance persistence.
  read-scope: .omp/skills/control-guard/SKILL.md, .agent/protocols/control-guard.md, .omp/skills/ctl-grill-with-spec/SKILL.md, .agent/skills/ctl-grill-with-spec/source.md
  write-allow: .agent/protocols/control-guard.md, .agent/skills/ctl-grill-with-spec/source.md
  gates: cargo_test
  acceptance: single-converged-task skips full grill + forced PRD; provenance persisted once at create; full grill still reachable on request.

- id: ceremony-stale-ordering
  objective: Pin canonical submit→commit→gate→audit→finish order in protocol; add finish pre-flight actionable stale hint.
  read-scope: src/application/mod.rs, .agent/protocols/control-guard.md
  write-allow: src/application/mod.rs, src/cli/mod.rs, .agent/protocols/control-guard.md
  gates: cargo_check, cargo_test, cargo_clippy, cargo_fmt_check
  acceptance: finish reports which commits intervened + write-scope-change flag before bare stale error; no change to tree/policy-stale verdict logic.
  depends-on: ceremony-pipeline-slim

- id: ceremony-audit-tier
  objective: Add light/full audit tier; light = reviewer-isolated closure checklist only.
  read-scope: src/application/mod.rs, src/domain/task.rs, .omp/skills/ctl-review/SKILL.md
  write-allow: src/application/mod.rs, src/domain/task.rs, src/cli/mod.rs
  gates: cargo_check, cargo_test, cargo_clippy, cargo_fmt_check
  acceptance: light tier hard-gates finish with reviewer≠implementer but skips decay scan; full tier unchanged.
- id: ceremony-skill-body-slim
  objective: Slim all 7 generated workflow skill bodies — merge the checklist triplet, remove core-invariant restatements, shrink station contracts, trim frontmatter descriptions.
  read-scope: .agent/skills/ctl-grill-with-spec/source.md, .agent/skills/ctl-to-tasks/source.md, .agent/skills/ctl-tdd-loop/source.md, .agent/skills/ctl-to-prd/source.md, .agent/skills/ctl-handoff/source.md, .agent/skills/ctl-architecture-review/source.md, .agent/skills/ctl-decision-map/source.md, .agent/protocols/workflow-skills.md
  write-allow: .agent/skills/ctl-grill-with-spec/source.md, .agent/skills/ctl-to-tasks/source.md, .agent/skills/ctl-tdd-loop/source.md, .agent/skills/ctl-to-prd/source.md, .agent/skills/ctl-handoff/source.md, .agent/skills/ctl-architecture-review/source.md, .agent/skills/ctl-decision-map/source.md
  gates: cargo_test
  acceptance: each source.md ~20-25% shorter; no body text restates a core invariant; each rule stated once; ctl skills sync regenerates clean; drift checks pass.
  depends-on: ceremony-core-tiering
