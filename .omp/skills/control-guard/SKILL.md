---
name: control-guard
description: "Central orchestrator. Auto-loaded every session. Routes ALL control operations through /ctl-* skills. Auto-triggers: First Principles for proposals, Bayesian Reasoning for failures, Spec Loading before dev, Spec Update after close, 5-dimension root cause analysis for bugs."
---

# Control Guard — Orchestration Layer (M4)

You are the control plane orchestrator. You run every session automatically. You:
1. Detect when control plane mediation is needed
2. Apply the right thinking framework automatically
3. Route to the correct `/ctl-*` skill
4. The main agent NEVER runs `ctl` commands directly

## Inviolable Rule

**The main agent NEVER runs `ctl` commands directly.**

## Skill Routing Table

| Trigger | Auto-trigger first | Route to |
|---|---|---|
| User describes code change | Spec Loading + First Principles | → `/ctl-new` |
| Agent finishes implementing | — | → `/ctl-apply` |
| Changes applied, ready to close | — | → `/ctl-close` |
| Task completed, knowledge gained | Spec Update check | → `/ctl-spec-update` (if needed) |
| Something broke mid-run | Bayesian + Root Cause | → `/ctl-abort` (if confidence ≥ 70%) |
| Gate failure during close | Bayesian + Root Cause | → Fix, re-run gate |
| User asks to generate/refresh specs | — | → `/ctl-spec-bootstrap` |
| User asks "health" / "check" | — | → `/ctl-health` |

## When to Engage

Engage when: modifying source files, clear verifiable objective, multi-file change, feature/bugfix/refactoring.

Skip when: pure conversation, read-only, user says "skip control".

---

## Auto-Trigger 1: Spec Loading (before every task proposal)

**When**: You detect a task-worthy request and are about to propose boundaries.

**Auto-apply**: `/ctl-spec-before` — load project specs before proposing.

1. Read `.ctl/spec/backend/index.md` for architecture overview and pre-dev checklist
2. Read layer-specific specs for the affected layers
3. Read cross-cutting guides if the change spans multiple layers
4. Check `ARCHITECTURE_GUARDRAILS.md` for milestone scope

Extract key constraints to carry into the proposal:
- Dependency direction (domain MUST NOT import from infrastructure/cli)
- Truth model (events.jsonl is append-only, task.json is projection)
- Reducer purity (no side effects, no I/O)
- Allowed dependencies (clap, serde, anyhow, sha2 only)
- Event types need: reducer branch + schema + fixture coverage

---

## Auto-Trigger 2: First Principles (on every task proposal)

**When**: After spec loading, before proposing boundaries.

1. **Restate the problem**: One sentence about what needs to be true when done.
   > Bad: "Add Redis caching" → Good: "Profile data loads too slowly"
2. **List fundamental truths**: Physical constraints, business rules, technical invariants, user needs.
3. **Challenge assumptions**: Fact or convention? What if removed? Solving problem or symptom?
4. **Build up**: Minimum viable scope from truths. Each addition must answer "which truth requires this?"
5. **Validate**: Does it solve the original problem? Simplest experiment?

---

## Auto-Trigger 3: Complexity Classification (after inference)

**When**: After inferring boundaries, before presenting proposal.

| Complexity | Criteria | Action |
|---|---|---|
| **Trivial** | Single-line fix, typo | Skip control plane, implement directly |
| **Simple** | Clear goal, 1-2 files | Ask 1 confirm, then `/ctl-new` |
| **Moderate** | Multi-file, some ambiguity | Light brainstorm (2-3 questions), then `/ctl-new` |
| **Complex** | Vague goal, architectural choices | Full brainstorm before `/ctl-new` |

---

## Auto-Trigger 4: Expansion Sweep (for Moderate/Complex tasks)

**When**: Before converging on MVP scope.

Before presenting the proposal, consider:

1. **Future evolution**: What might this become in 1-3 months? What extension points are worth preserving?
2. **Related scenarios**: What adjacent flows should stay consistent? Parity expectations?
3. **Failure & edge cases**: Conflicts, offline failure, retries, idempotency, rollback, security boundaries.

Then: what's in MVP → `write_allow`. What's excluded → note in risks.

---

## Phase 1: Full Proposal Flow

When you detect a task-worthy request:

### 1.1 Spec loading (auto)

Auto-trigger `/ctl-spec-before`. Load constraints into context.

### 1.2 First principles (auto)

Auto-apply FP-1 through FP-5. Derive minimum scope from truths.

### 1.3 Read codebase

Read relevant source files. Never propose blind.

### 1.4 Infer fields

| Field | Inference rule |
|---|---|
| `id` | Kebab-case, 2-4 words |
| `objective` | From FP-1 restatement |
| `read_scope` | Context files + tests |
| `write_allow` | Minimum from FP-4 build-up |
| `gates` | Default: `cargo_fmt_check`, `cargo_check`, `cargo_test` |

Heuristics:
- **Bug fix**: bug file(s) + tests
- **New feature**: new/modified source + tests
- **Refactoring**: moved files + dependents
- **Config/build**: high-risk, flag for approval

### 1.5 Complexity classification (auto)

Determine Trivial/Simple/Moderate/Complex.

### 1.6 Expansion sweep (auto, Moderate+)

Diverge before converging. Add edge cases and future considerations to risks.

### 1.7 Present proposal

```
📋 Task Proposal: fix-auth-timeout

  Objective: 确保 auth 模块超时后返回错误而非挂起
  
  📖 Read:    src/auth/timeout.rs, tests/auth_test.rs
  ✏️ Write:   src/auth/timeout.rs, tests/auth_test.rs
  🔍 Gates:   cargo_fmt_check, cargo_check, cargo_test
  ⚠️ Risks:   callers may need timeout handling updates
  📋 Specs:   domain-layer, cross-layer guide
  
  Constraints:
  - Layer: domain + infrastructure
  - No new deps needed
  - Event type needs schema + fixture
  
  ✅ approve  ✏️ adjust  ❌ skip
```

Wait for approval. Do NOT proceed without it.

### 1.8 After approval → `/ctl-new`

---

## Phase 2: Implementation in Worktree

After `/ctl-new` succeeds, agent works inside the OMP worktree.

**Before every file write**: verify target is within `write_allow`.

**When implementation is complete**: auto-invoke `/ctl-apply`.

---

## Phase 3: Apply and Close (auto-chain)

After `/ctl-apply` → auto `/ctl-close`.

After `/ctl-close` completes → check if spec update is needed:
- Did the task reveal non-obvious patterns?
- Did Bayesian diagnosis find a root cause worth preserving?
- Any new conventions established?

If yes → auto-trigger `/ctl-spec-update`.

---

## Auto-Trigger 5: Bayesian Reasoning (on every failure)

**When**: Gate fails, OMP crashes, boundary violation, health check fails, considering abort.

### B-1: Establish Priors

| Hypothesis | Prior | Reasoning |
|------------|-------|-----------|
| H1: (most likely) | 40% | ... |
| H2: (second) | 30% | ... |
| H3: (other) | 30% | Catch-all |

### B-2: Observe Evidence

What exactly happened? How reliable? Could multiple hypotheses explain this?

### B-3: Update Beliefs

Which hypothesis does the evidence support? Direction > calculation.

### B-4: Seek Discriminating Evidence

"What would I see if H1 is true but not H3?" Check for that.

### B-5: State Confidence

| Confidence | Action |
|---|---|
| 90%+ | Proceed with fix, monitor |
| 70-90% | Proceed, add fallback |
| 50-70% | Test hypothesis first |
| <50% | Need more evidence |

### B-6: Watch for Fallacies

| Fallacy | Correction |
|---|---|
| Base rate neglect | How often does this happen for other reasons? |
| Confirmation bias | Actively seek evidence AGAINST top hypothesis |
| Anchoring | Priors from current context, not last time |

---

## Auto-Trigger 6: Root Cause Analysis (after Bayesian converges on a bug)

**When**: Bayesian reasoning converges on a specific root cause (confidence ≥ 70%).

### 5-Dimension Root Cause Analysis

Classify the bug:

| Category | Characteristics | Example |
|---|---|---|
| **A. Missing Spec** | No documentation on how to do it | New event type without fixture |
| **B. Cross-Layer Contract** | Interface between layers unclear | CLI arg format ≠ event payload format |
| **C. Change Propagation Failure** | Changed one place, missed others | New reducer branch, no CLI command |
| **D. Test Coverage Gap** | Unit passes, integration fails | Works alone, breaks with other events |
| **E. Implicit Assumption** | Code relies on undocumented assumption | Path separator `\` vs `/` on Windows |

### Why fixes failed (if multiple attempts)

- **Surface fix**: Fixed symptom, not root cause
- **Incomplete scope**: Found root cause, didn't cover all cases
- **Tool limitation**: Search missed it, type check wasn't strict
- **Mental model**: Kept looking in same layer, didn't think cross-layer

### Systematic expansion

- **Similar issues**: Where else might this exist?
- **Design flaw**: Fundamental architecture issue?
- **Process flaw**: Development process improvement?

### Knowledge capture → `/ctl-spec-update`

If the root cause reveals something worth preserving:
- New event type convention → update domain-layer spec
- Cross-layer format mismatch → update cross-layer guide
- Path handling gotcha → update infrastructure spec
- Testing gap → update quality guidelines

---

## Auto-Trigger 7: Spec Update (after close or diagnosis)

**When**: After task completion OR after root cause analysis reveals a pattern.

Auto-check: did this task or diagnosis produce knowledge worth preserving?

| Signal | Target |
|---|---|
| New design decision | `.ctl/spec/backend/index.md` |
| Layer-specific pattern | Relevant layer spec |
| Cross-layer gotcha | `.ctl/spec/guides/cross-layer-thinking-guide.md` |
| Code reuse insight | `.ctl/spec/guides/code-reuse-thinking-guide.md` |
| Error handling lesson | `.ctl/spec/backend/error-handling.md` |
| Quality gap | `.ctl/spec/backend/quality-guidelines.md` |

If yes → auto-trigger `/ctl-spec-update`.

---

## Orchestration Flow (complete)

```text
User message arrives
  │
  ├─ Code change request?
  │   YES → AUTO: Spec Loading
  │       → AUTO: First Principles
  │       → Infer boundaries from codebase
  │       → AUTO: Complexity Classification
  │       → AUTO: Expansion Sweep (Moderate+)
  │       → Present proposal for approval
  │       → User approves → /ctl-new
  │       → Agent implements in worktree
  │       → Auto /ctl-apply when done
  │       → Auto /ctl-close after apply
  │       → AUTO: Spec Update (if knowledge gained)
  │
  ├─ Status/progress question?
  │   YES → /ctl-status
  │
  ├─ Health/check request?
  │   YES → /ctl-health
  │
  ├─ Abort/give up/something broke?
  │   YES → AUTO: Bayesian Reasoning
  │       → AUTO: Root Cause Analysis
  │       → AUTO: Spec Update (if pattern found)
  │       → /ctl-abort (only if confidence ≥ 70%)
  │
  └─ None of the above?
      → Work freely
```

---

## Recovery Routing

| Situation | Auto-triggers | Route to |
|---|---|---|
| OMP crash | Bayesian → Root Cause → Spec Update | → `/ctl-abort` if ≥ 70% |
| Task held | Bayesian (scope vs code?) | → `/ctl-status` to inspect |
| Scope too narrow | — | → Cancel, new `/ctl-new` |
| Gate failure | Bayesian → Root Cause | → Fix, re-run gate |
| Health failure | Bayesian (common cause?) | → Fix root, re-run all |

---

## Anti-Patterns

- ❌ NEVER run `ctl` directly — always through `/ctl-*`
- ❌ NEVER skip spec loading before proposing boundaries
- ❌ NEVER skip first principles before deriving write_allow
- ❌ NEVER skip Bayesian before aborting or diagnosing failure
- ❌ NEVER skip root cause analysis after finding a bug
- ❌ NEVER skip gate verification — all gates must pass
- ❌ NEVER modify files outside `write_allow`
- ❌ NEVER manually edit `events.jsonl` or `task.json`
- ❌ NEVER express binary certainty with incomplete evidence
- ❌ NEVER let knowledge stay in chat — capture to specs via `/ctl-spec-update`
