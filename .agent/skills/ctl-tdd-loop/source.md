---
name: ctl-tdd-loop
description: "Drive a strict TDD loop under a task (one behavior → red-capable test → minimal impl → green). Triggers when: implementing a behavior change, especially a task opted into the ctl tdd-red-green interlock. Do NOT trigger for: pure refactors with no behavior change, docs, or diagnosis (ctl-diagnose)."
---


## The loop (phase body)

1. **Pick one behavior** — the smallest observable change in public behavior.
2. **Write one red-capable test** — one that *can* fail, describing behavior that does not yet exist.
3. **Run it and capture the FAIL** as evidence: `ctl gate run --id <id> --gate cargo_test` records it on the ledger.
4. **Implement the minimal change** that makes it pass — nothing speculative.
5. **Run it and capture the PASS** (same `ctl gate run`). The FAIL-before-PASS history is the red→green proof the interlock checks.
6. **Refactor only after green**, with the test still passing.
7. **Repeat** for the next behavior.

### Why the evidence matters

A task opted into TDD (`ctl task create --tdd ... --gates cargo_test`) cannot `finish` unless the recorded `cargo_test` history has a failing result at an earlier seq than a passing one. "I did TDD" is gate evidence on the ledger, not an assertion — skip the red capture and the interlock blocks finish.

### If ctl evidence can't capture red/green

If a particular gate can't represent the red/green run, write the loop's evidence into the task artifact and note the gap; do not pretend the interlock passed.

### Anti-patterns

- ❌ Green-only history (test only ever passed) on a TDD-opted task.
- ❌ One giant test covering five behaviors.
- ❌ Asserting on internals instead of observable behavior.
- ❌ Editing the test to match a bug, silently.

<!-- integration:omp -->

Opt a task in at create time: `ctl task create --tdd ... --gates cargo_test` (sugar for the
`tdd-red-green` risk trigger). Capture each run with `ctl gate run --id <id> --gate
cargo_test`; the OMP gate governs the edits between runs. `ctl task finish` enforces the
red→green interlock — there is no flag to skip it.
<!-- integration:opencode -->

Opt a task in at create time: `ctl task create --tdd ... --gates cargo_test`. Capture each
run with `ctl gate run --id <id> --gate cargo_test`; `.opencode/plugins/ctl-gate.ts` governs
the edits between runs. `ctl task finish` enforces the red→green interlock — no skip flag.

**Recommended role** (autonomous dispatch — see control-guard): `build` — red→green
implementation. Writable role, so it needs an active in_progress task; route deep
root-causing of a stuck test to `oracle`.
<!-- integration:claude -->

Opt a task in at create time: `ctl task create --tdd ... --gates cargo_test` (sugar for the `tdd-red-green` risk trigger). Capture each run with `ctl gate run --id <id> --gate cargo_test`; the Claude Code PreToolUse gate governs the edits between runs. `ctl task finish` enforces the red→green interlock — there is no flag to skip it.
