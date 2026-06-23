# Failure Diagnosis

Read this guide when something breaks: gate failure, boundary violation, crash, unexpected behavior.

## Step 1: Bayesian Reasoning

Establish priors before jumping to conclusions:

| Hypothesis | Prior | Reasoning |
|---|---|---|
| H1: (most likely) | 40% | ... |
| H2: (second) | 30% | ... |
| H3: (other) | 30% | Catch-all |

Then:
1. **Observe evidence**: What exactly happened? How reliable? Could multiple hypotheses explain this?
2. **Update beliefs**: Which hypothesis does the evidence support? Direction > calculation.
3. **Seek discriminating evidence**: "What would I see if H1 is true but not H3?" Check for that.
4. **State confidence**:

| Confidence | Action |
|---|---|
| 90%+ | Proceed with fix, monitor |
| 70-90% | Proceed, add fallback |
| 50-70% | Test hypothesis first |
| <50% | Need more evidence |

5. **Watch for fallacies**:
   - Base rate neglect: How often does this happen for other reasons?
   - Confirmation bias: Actively seek evidence AGAINST top hypothesis
   - Anchoring: Priors from current context, not last time

## Step 2: Root Cause Analysis (after Bayesian converges)

When confidence ≥ 70%, classify the root cause:

| Category | Characteristics | Example |
|---|---|---|
| **A. Missing Spec** | No documentation on how to do it | New event type without fixture |
| **B. Cross-Layer Contract** | Interface between layers unclear | CLI arg format ≠ event payload format |
| **C. Change Propagation** | Changed one place, missed others | New reducer branch, no CLI command |
| **D. Test Coverage Gap** | Unit passes, integration fails | Works alone, breaks with other events |
| **E. Implicit Assumption** | Code relies on undocumented behavior | Path separator `\` vs `/` on Windows |

## Step 3: Why fixes failed (if multiple attempts)

- **Surface fix**: Fixed symptom, not root cause
- **Incomplete scope**: Found root cause, didn't cover all cases
- **Tool limitation**: Search missed it, type check wasn't strict
- **Mental model**: Kept looking in same layer, didn't think cross-layer

## Step 4: Systematic expansion

- **Similar issues**: Where else might this exist?
- **Design flaw**: Fundamental architecture issue?
- **Process flaw**: Development process improvement?

## Step 5: Knowledge capture

If the root cause reveals something worth preserving → `/ctl-spec-update`.

Target the right spec:

| Signal | Target spec |
|---|---|
| New event type convention | `domain-layer.md` |
| Cross-layer format mismatch | `cross-layer-thinking-guide.md` |
| Path handling gotcha | `infrastructure-layer.md` |
| Testing gap | `quality-guidelines.md` |
