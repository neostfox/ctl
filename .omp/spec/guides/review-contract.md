# Review Contract

The shared output contract for **every** review the control plane runs — whether the
main agent reviews inline or a `ctl-review` sub-agent is dispatched. It binds three
existing rubrics into one verdict format so reviews are comparable, auditable, and
(later) machine-readable as events.

Rubrics this contract composes (do not duplicate them here — reference them):
- Production code: [`decay-risks.md`](decay-risks.md) — R1–R6 with severity tiers.
- Tests: [`test-decay-risks.md`](test-decay-risks.md) — T1–T6 with severity tiers.
- When a review uncovers a *defect* (not just decay): [`failure-diagnosis.md`](failure-diagnosis.md).

---

## The Iron Law (inherited)

```
NEVER suggest fixes before completing diagnosis.
EVERY finding MUST follow: Symptom -> Source -> Consequence -> Remedy.
```

A finding missing any of the four fields is noise, not a finding — drop it or complete it.

## Finding schema

Each finding is one block:

```
[<severity>] <risk-code> - <one-line title>
  Symptom:     <what is observably wrong>   (file:line)
  Source:      <the underlying cause>
  Consequence: <what it costs if left>
  Remedy:      <the concrete fix>
```

- `severity` in Critical / Warning / Suggestion (tiers defined per risk in the rubrics).
- `risk-code` in R1-R6 / T1-T6, or `DEFECT` when it is a correctness bug (then also run failure-diagnosis).

## Health Score

A single number a gate can threshold on:

```
score = 100 - 15*(critical) - 5*(warning) - 1*(suggestion)
floor at 0
```

Report the arithmetic, e.g. `Health: 79 (100 - 15 - 5 - 1)`.

## Verdict (machine-readable head)

Every review ends with a verdict line so it can be recorded as evidence on the task
ledger. The read-only reviewer emits the line; the dispatcher (control-guard) records it via
`ctl review accept|reject` (see control-guard "verdict -> event"):

```
VERDICT: pass|fail   score=<n>   critical=<n> warning=<n> suggestion=<n>   mode=edit|audit
```

- `pass` requires **zero Critical findings**. Warnings/suggestions do not block but are reported.
- A `fail` verdict on the completion audit sends the task back to fix-up, not to `finish`.
- For `mode=audit` this is a **hard gate**: `ctl task finish` refuses to complete without a
  fresh `completion_audit` pass recorded after the last `submit` (M-f). The pass must be
  recorded by a **reviewer identity distinct from the implementer** (`CTL_ACTOR`; M6) — an
  implementer cannot self-approve, though it may self-`reject`. `mode=edit` verdicts are
  enforced via the `ctl apply` primitive (a granted path-scoped exception).

---

## Two review modes

### Mode A — edit review (pre-apply)

Scope: a single proposed change (one file or a small diff), reviewed **before** it lands.
- Trigger: an out-of-scope edit request, or a batch of risk-bearing changes (per control-guard).
- Read only the proposed diff + the files it touches + the relevant spec/layer guide.
- Output: findings + verdict, `mode=edit`. The main agent honors the verdict.

### Mode B — completion audit (pre-finish)

Scope: the **whole task diff** (`git diff` for the task's branch/working tree), reviewed
before `ctl task submit` / `finish`.
- Apply R1-R6 and T1-T6 across every changed file.
- Run the **closure checklist** below before issuing `pass`.
- Output: findings + Health Score + verdict, `mode=audit`.

## Closure checklist (completion audit only)

Borrowed from the "where's the evidence?" discipline — completion claims require artifacts,
not assertions. The audit must confirm and cite each:

- [ ] Build passes — paste/point to the command + result (`cargo check`).
- [ ] Tests pass — command + result (`cargo test`), including any new test for new behavior.
- [ ] Lint/format clean — `cargo fmt --check`, `cargo clippy` if gated.
- [ ] Every changed public behavior has a test, or an explicit, justified note why not.
- [ ] No debug logging, no commented-out code, no suppressed warnings left in.
- [ ] Spec updated when a new pattern/convention/decision was introduced (else state "nothing to update" with reasoning).
- [ ] The diff matches the task objective — no scope creep, no unrelated edits.

An audit that cannot produce evidence for a checked item must report it as a Critical
closure-discipline finding, not wave it through.
