---
name: control-guard
description: "Control plane entry point for opencode. Proactively routes task lifecycle — detects multi-step work, creates ctl tasks with clear boundaries, and relies on the ctl plugin to inject scope and gate every tool call. All findings follow the Iron Law: Symptom → Source → Consequence → Remedy."
---

# Control Guard (opencode)

You **proactively enforce** the two-layer governance model in this project:

```
ctl task (parent)        — declared scope, gates, boundaries (ctl ledger)
  └─ opencode subtasks   — your own step tracking within the parent's write_allow
```

Enforcement is automatic: the `.opencode/plugins/ctl-gate.ts` plugin injects the
active task's boundaries into context every turn and **blocks any write/edit/bash
outside scope** via the ctl state machine. If `ctl` is unavailable, mutating
tools **fail closed** (blocked) until it responds — you cannot work around this by
retrying; create or widen a task instead.

## When to Engage (PROACTIVE)

**Propose creating a ctl task when you detect:**
- Multi-file changes (2+ files to modify)
- A feature / bugfix / refactor with a clear objective
- A problem that needs investigation **and** code changes
- Any work that benefits from an audit trail and scope enforcement

**Skip (work freely):** pure Q&A, read-only exploration, a trivial single-file
edit (typo/comment), or when the user explicitly says "skip control".

## Task Proposal Flow

### Step 1 — Analyze
1. Read specs: `.ctl/spec/backend/index.md` and relevant layer specs.
2. Read the files that will be affected.
3. Infer boundaries: `read_scope`, `write_allow` (always minimal), `gates`.

### Step 2 — Present for approval

```
📋 Task Proposal: <id>
  Objective: <one sentence>
  📖 Read:    <files/dirs>
  ✏️ Write:   <minimal — just what changes>
  🚫 Deny:    <protected paths if any>
  🔍 Gates:   <gate list>
  ✅ approve  ✏️ adjust  ❌ skip
```

### Step 3 — Create and start

```bash
ctl task create --id <id> --objective "<text>" \
  --read-scope <path>... --write-allow <path>... --gates <gate>...
ctl task ready --id <id>
ctl task start --id <id>
```

Once the task is active, the plugin gates your tool calls automatically — you do
not need to replicate the boundary check.

### Step 4 — Close the task (completion audit first)

`ctl task finish` is **hard-gated**: it refuses without a fresh passing
`completion_audit` recorded after the last `submit`, plus fresh gate evidence
bound to the current tree and a clean working tree.

```bash
ctl task submit --id <id>                       # → Review (audit + commit window)
# review the whole task diff, then record the verdict as the REVIEWER identity
# (M6: the implementer cannot accept its own audit — CTL_ACTOR must differ):
CTL_ACTOR=reviewer ctl review accept --id <id> --note "<health/summary>"   # pass
#   or, on a failing audit —
CTL_ACTOR=reviewer ctl review reject --id <id> --note "<blocking findings>" # back to fix-up
# only after a pass AND committing the in-scope work:
ctl task finish  --id <id>
ctl task archive --id <id>
```

If gate evidence is stale ("rerun required gates"), run them first:

```bash
ctl gate run --id <id> --gate <gate>   # for each required gate, then re-audit
```

## Boundary Auto-Inference

| Signal | write_allow |
|---|---|
| Single file fix | that file only |
| Module change | `src/<module>/` |
| Cross-module refactor | one `--write-allow` per module |
| Schema change | `schemas/` + `src/domain/` |
| Test addition | `tests/` or the matching source dir |

**write_allow is ALWAYS minimal.** Start narrow; widen only with explicit approval
via `ctl task revise`.

## How the Plugin Works (so you don't fight it)

`.opencode/plugins/ctl-gate.ts` does these automatically:

1. **`experimental.chat.system.transform`** — injects `📋 Active ctl task
   boundaries` into the system prompt every call.
2. **`tool.execute.before`** — gates `write` / `edit` / `patch` / `bash` / `task`
   via `ctl hook gate`; throws (blocks the tool) on an out-of-scope or
   wrong-phase verdict, and **fails closed** for mutating tools if ctl is down.

**If a write is blocked**, the message names the task and path. Don't bypass —
either widen scope via `ctl task revise` or redirect your work.

## Error Handling

- **Write blocked**: `ctl boundary explain --path <path>`, fix the path, retry.
  Never widen to root.
- **Task id exists**: propose a new id. Never mutate existing events.
- **Any `ctl` command fails**: STOP, report the error, do not skip steps.

## Iron Law (diagnosis)

```
NEVER suggest fixes before completing risk diagnosis.
EVERY finding MUST follow: Symptom → Source → Consequence → Remedy.
```

Severity: 🔴 Critical / 🟡 Warning / 🟢 Suggestion.

## Anti-Patterns

- ❌ Start multi-file work without creating a ctl task
- ❌ Modify files outside `write_allow`
- ❌ Hand-edit `events.jsonl` or `task.json`
- ❌ Record your own passing completion audit (M6: reviewer ≠ implementer)
- ❌ Suggest a fix without diagnosing root cause
