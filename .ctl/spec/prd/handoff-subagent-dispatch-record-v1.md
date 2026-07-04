# Handoff: subagent-dispatch-record-v1 (last buildable attestation slice)

> Not yet created. From the confirmed PRD `.ctl/spec/prd/attestation-layer.md`.
> Sibling slices done: `run-finish-emit-v1`, `run-attestation-fields-v1`. Deferred:
> `dispatch-attestation-host-wiring-v1` (U-D, after this lands).

## Objective

Record a subagent **dispatch** as a canonical fact: role, adapter, parent
task (+ optional run), and instruction/context/output sha256 (of supplied
artifacts). Record-and-disclose — role/adapter are host-supplied **labels**, the
hashes are content hashes of host-supplied artifacts; ctl does **not** verify
what actually ran. Closes audit Findings 2 & 7 at the dispatch layer.

## Key decisions (do not relitigate)

- **U-A record-and-disclose** (confirmed): ctl hashes supplied artifacts (`sha2`)
  and stores host-reported values; never claims verification. Disclose
  "host-attested" everywhere.
- **U-B schema via `ctl apply`** (confirmed): protected `schemas/` paths use the
  reviewed-exception flow — NOT `write_allow`.
- **The dispatch event lives on the PARENT TASK's ledger** (recommended): a new
  `subagent_dispatched` **task** event (reuse the task aggregate / `apply` /
  `validate_and_append`), not a new aggregate. The dispatch is part of the task's
  history.
- **The envelope-schema change is MANDATORY here** (unlike run-attestation-fields-v1):
  TASK events ARE schema-validated (`validate_event` → `validate_instance` against
  `control.event-envelope.v1`), so `subagent_dispatched` MUST be added to the
  envelope `type` enum (schema lines ~43-87) AND given an `if/then` payload block
  (mirror `run_started`/`evidence_accepted` blocks, `additionalProperties:false`).
  Without it, `validate_and_append` will REJECT the event. Via `ctl apply`.
- No crypto/HTTP deps (guardrail: `clap/serde/anyhow/sha2/libc` only).

## Plan (files / scope)

- write_allow: `src/domain/task.rs`, `src/application/mod.rs`, `src/cli/mod.rs`.
- via `ctl apply` (protected): `schemas/control.event-envelope.v1.schema.json`;
  and `schemas/control.task-view.v1.schema.json` IF the dispatch surfaces in the
  `task.json` projection (verify the task-view projection first — see uncertainty).
- gates: floor (cargo_fmt_check/check/clippy/test). AFK except the `ctl apply`
  approval step (HITL).

Shape: `subagent_dispatched` payload = `{ role, adapter, parent_run?,
instruction_hash?, context_hash?, output_hash? }`; reducer records into a new
`TaskState.dispatches: Vec<DispatchRecord>` (`#[serde(default)]`). CLI:
`ctl dispatch record --task <id> --role <r> --adapter <a> [--run <id>]
[--instruction-artifact <p>] [--context-artifact <p>] [--output-artifact <p>]`,
hashing via the existing `hash_file`/`hash_evidence` (application/mod.rs ~4409 /
~1111). Consider a read-only `ctl dispatch list --task <id>` viewer.

## Open uncertainties (carry forward)

- **Does the dispatch need to appear in the `task.json` projection?** Check the
  task-view writer (the task analog of `write_run_view`). If yes →
  `control.task-view.v1.schema.json` also needs a `ctl apply` edit.
- **`ctl dispatch list` viewer** — include now or defer? (Recommend include; small.)
- **parent_run linkage** — optional field (a dispatch may occur outside a run).

## Next safe action (single step)

Dispatch a read-only Explore agent to map: (1) `validate_event`/`validate_instance`
on the task append path; (2) the envelope schema `type` enum + an `if/then` block
to mirror for `subagent_dispatched`; (3) the `task.rs` `apply` reducer arm pattern
+ `TaskState` field + whether/how `task.json` is projected; (4) a CLI record
command to mirror (`ctl uncertainty record`). Then `ctl task create` the slice,
ready/start, and TDD red→green.

## Do-not-do

- Don't add the envelope/task-view schema to `write_allow` (protected → `ctl apply`).
- Don't skip the envelope `type`-enum addition — task events ARE validated; the
  event is rejected otherwise.
- Don't try to gate the Claude `Task` tool (U-1: structurally impossible via
  PreToolUse).
- Don't add crypto/signing deps. Don't record your own passing audit.

## Environment hazards (Windows / this repo)

- **PowerShell mangles JSON args** to native exes — use the **Bash tool** with
  single-quoted JSON for any `--data`-style arg.
- **`CTL_ACTOR` does not persist** across tool calls — set inline:
  `$env:CTL_ACTOR='ctl-review'; ctl review accept …` (and for `ctl approval grant`).
- **`ctl apply` flow**: `ctl apply --id <t> --path <p> --reason <r>` →
  `CTL_ACTOR=ctl-review ctl approval grant --id <t> --request <req>` → gate now
  allows that one path.
- **Lifecycle**: ready → start → edit → submit → commit (Bash tool, in the Review
  window) → `ctl gate run` ×4 → `CTL_ACTOR=ctl-review ctl review accept` → finish
  → archive. The gate binds to the sole `in_progress` task (no `CTL_TASK_ID`
  needed while only one is active).
- Platform: **Claude Code** (PreToolUse gate; writes stay **inline** in the main
  agent — read-only investigation may be dispatched to subagents).
- Dispatch binding: `CTL_TASK_ID` is **not** set.
