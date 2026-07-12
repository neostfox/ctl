---
name: ctl-tdd-loop
description: "Drive a strict TDD loop under a task (one behavior → red-capable test → minimal impl → green). Triggers when: implementing a behavior change, especially a task opted into the ctl tdd-red-green interlock. Do NOT trigger for: pure refactors with no behavior change, docs, or diagnosis (ctl-diagnose)."
---

# ctl-tdd-loop (Claude Code)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-tdd-loop/source.md` by `ctl skills sync`. Claude Code-specific mechanics live after the core.

<!-- ctl:workflow-core:start version=1 -->
# ctl Workflow Skills — Core Protocol

WORKFLOW_PROTOCOL_VERSION = 1

This is the platform-neutral workflow-skills core. It is split into an
**embedded** part (division of labor + invariants), carried verbatim inside
every workflow skill's managed-core block, and a **reference** part (phase map +
frameworks + provenance) that lives only in this file — the auto-loaded
control-guard carries the pipeline routing, and each skill's body covers its own
phase. The canonical copy lives at `.agent/protocols/workflow-skills.md`; a CI
drift check fails if any embedded copy diverges. Edit this file and re-sync
every workflow skill together — never one in isolation. Nothing platform-specific
(tool names, hook mechanics, plugin paths) and nothing phase-specific belongs in
the embedded part; that lives in each skill outside the managed core.

## Division of labor (non-negotiable)

Skills and agents manage **semantic workflow** — what to think about, in what
order, and which artifact each phase produces. ctl manages **facts, scope,
evidence, gates, ledgers, and honest disclosure**. A workflow skill never relaxes
a boundary, never declares a task complete, and never substitutes its own
judgement for ctl evidence. Workflow discipline is not proof: it does not replace
gates, audits, reviewer independence, or tamper evidence, and it never creates a
verdict.

## Invariants every phase honors

- Produce **artifacts, not claims**. "Done" is an evidence artifact ctl can see,
  never an assertion — "where is the evidence?"
- Keep **draft separate from confirmed basis**; disclose open uncertainty rather
  than hiding it.
- **Red before green**: no green claim without prior red evidence for the same
  behavior.
- **No fix before a reproduction loop.**
- **Architecture review is read-only**; a refactor needs a fresh governed task.
- External workflow inspiration is **L0 reference material** (see Provenance) —
  never an authority, never vendored as an active control.
<!-- ctl:workflow-core:end -->

*The phase map, frameworks, and provenance are reference material in `.agent/protocols/workflow-skills.md` — not embedded here. The auto-loaded control-guard carries the pipeline routing; this skill's body covers its own phase.*

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

## Claude Code Integration (platform-specific)

Opt a task in at create time: `ctl task create --tdd ... --gates cargo_test` (sugar for the `tdd-red-green` risk trigger). Capture each run with `ctl gate run --id <id> --gate cargo_test`; the Claude Code PreToolUse gate governs the edits between runs. `ctl task finish` enforces the red→green interlock — there is no flag to skip it.
