---
name: ctl-tdd-loop
description: "Drive implementation as a strict TDD loop: one behavior, one red-capable test, captured failure evidence, minimal implementation, captured green evidence, refactor only after green. Tests public behavior, not private detail; never claims TDD without red evidence. Triggers when: implementing a behavior change under a task, especially one opted into the ctl tdd-red-green interlock. Do NOT trigger for: pure refactors with no behavior change, docs, or diagnosis (ctl-diagnose)."
---


## The loop (phase body)

1. **Pick one behavior** — the smallest observable change in public behavior.
2. **Write one red-capable test** for it — a test that *can* fail and currently
   describes behavior that does not yet exist.
3. **Run it and capture the failure** as ctl evidence:
   `ctl gate run --id <id> --gate cargo_test` records the FAIL on the ledger.
4. **Implement the minimal change** that makes it pass — nothing speculative.
5. **Run it and capture the pass**: `ctl gate run --id <id> --gate cargo_test`
   records the PASS. The FAIL-before-PASS history is the red→green proof the
   interlock checks.
6. **Refactor only after green**, with the test still passing.
7. **Repeat** for the next behavior.

### Why the evidence matters

A task opted into TDD (`ctl task create --tdd ... --gates cargo_test`) cannot
`finish` unless the recorded `cargo_test` history contains a failing result at an
earlier seq than a passing one. So "I did TDD" is not an assertion — it is gate
evidence on the ledger. Skip the red capture and the interlock blocks finish.

### Forbidden

- Writing a large batch of speculative tests up front.
- Modifying a test to fit the implementation **without recording why** (the change
  must be justified in the task record, not silent).
- Testing private implementation detail when public behavior can be tested.
- Claiming TDD with no red evidence — the interlock exists precisely to catch this.

### If ctl evidence can't capture red/green

If a particular gate can't represent the red/green run, write the loop's evidence
into the task artifact and note the gap; do not pretend the interlock passed.

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
