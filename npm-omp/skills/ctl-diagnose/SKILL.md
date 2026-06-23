---
name: ctl-diagnose
description: "Root-cause a defect with disciplined Bayesian reasoning and capture the lesson so the class of bug cannot recur. Triggers when: a gate fails, a boundary is violated, a build/test breaks, behavior is unexpected, or the same bug has been 'fixed' more than once. Do NOT trigger for: code-quality review (use ctl-review), planning (use ctl-brainstorm), or routine implementation with no failure."
---

# ctl-diagnose

When something breaks, you diagnose before you fix — and you grade your evidence so the
conclusion is auditable. The full method already lives in
**[`failure-diagnosis.md`](../../spec/guides/failure-diagnosis.md)** (Bayesian priors,
discriminating evidence, confidence→action, root-cause categories A-E). Read it. This skill
adds the evidence-grading and falsification discipline on top, and the loop-breaking capture
at the end.

## Workflow placement

This is the **diagnose phase** of the ctl workflow foundation
(`.agent/protocols/workflow-skills.md`): Bayesian reasoning is *placed* here, not
floating, exactly as the workflow-core specifies (First Principles lives in
`ctl-grill-with-spec`; Bayesian lives here). The iron rule of that phase — **no fix
before a red-capable feedback loop** — is the same one enforced below. This skill
keeps its standalone yao-bayesian lineage (see `NOTICE.md`) and its OMP-specific
`failure-diagnosis.md` guide, so it is not embedded in the shared workflow-core
block; it is reached from the workflow phase map by routing.

## Fact-driven attribution (the hard rule)

Never state a root cause you have not confirmed with a tool. "Probably the environment",
"likely a race" — without evidence these are blame-shifting, not diagnosis. Read the code,
run the repro, check the log. No verified cause → no conclusion.

## Step 1: Grade your evidence

Not all evidence is equal. Tag each piece, and let strong evidence move belief more than weak:

| Grade | What it is | Weight |
|---|---|---|
| A | Reproduced failure, stack trace, `git bisect` result | strong — can confirm/refute a hypothesis |
| B | Test output, type error, logs you read directly | medium |
| C | Code inspection, expert reasoning about the path | usable, with stated caveats |
| D | "The model thinks", heuristic guess | weak — a hypothesis, not support |
| E | Stale comment, outdated doc, marketing | do not rely on |

## Step 2: Bayesian convergence (per failure-diagnosis.md)

Set priors over 2-4 hypotheses, observe evidence, update, and **seek discriminating
evidence**: "what would I see if H1 is true but H2 is not?" Then check for exactly that.

**Disconfirming gate (mandatory before concluding):** state what evidence would *reverse*
your top hypothesis, and confirm you looked for it and did not find it. A diagnosis that
has not tried to falsify itself is not ready.

Report confidence and act per the failure-diagnosis confidence→action table (≥90% fix;
70-90% fix + fallback; 50-70% test first; <50% gather more).

## Step 3: Classify root cause

Use the A-E categories in `failure-diagnosis.md` (Missing Spec / Cross-Layer Contract /
Change Propagation / Test Coverage Gap / Implicit Assumption).

## Step 4: Break the loop (prevention)

If the same issue was fixed more than once, do not stop at the fix. For each prior failed
fix, name why it failed (surface fix / incomplete scope / tool limitation / wrong mental
model), then choose a **prevention mechanism** that makes recurrence structurally harder:

| Mechanism | Makes the bug… |
|---|---|
| Documentation | known (update a spec/guide) |
| Architecture | impossible (type-safe wrapper, newtype) |
| Compile-time | caught at build (strict types, no escape hatch) |
| Test coverage | caught by CI (regression/integration test) |
| Runtime check | observable (assertion, validation) |

## Step 5: Capture

Hand the durable lesson to **ctl-spec-update** — a gotcha, a convention, or a new test —
so the next session inherits it. A diagnosis that doesn't change the specs will be
re-discovered.

## Anti-patterns

- ❌ Naming a cause you haven't verified with a tool.
- ❌ Concluding without running the disconfirming gate.
- ❌ Treating a D/E-grade guess as if it were A-grade evidence.
- ❌ Fixing the symptom and moving on when the bug has recurred before.
- ❌ Diagnosing without capturing the lesson back into specs.
