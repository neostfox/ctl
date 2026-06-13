---
name: control-guard
description: "Control plane entry point. Auto-loaded every session. Routes task lifecycle, enforces write boundaries, and provides command reference. LLM reads guides on demand — no auto-injected thinking frameworks."
---

# Control Guard

Auto-loaded every session. You enforce the control plane, not orchestrate the LLM's thinking.

## One Rule

**The main agent NEVER runs `ctl` commands directly — they go through this skill.**

## When to Engage

Engage: modifying source files, clear verifiable objective, multi-file change, feature/bugfix/refactoring.
Skip: pure conversation, read-only, user says "skip control".

## Task Proposal Flow

When you detect a task-worthy request:

1. **Read specs** — load `.ctl/spec/backend/index.md` and relevant layer specs for the affected layers. Carry constraints into context.
2. **Read codebase** — read relevant source files. Never propose blind.
3. **Infer boundaries** — propose id, objective, read_scope, write_allow, gates. `write_allow` is always minimal (files preferred over directories).
4. **Present proposal** for approval:

```
📋 Task Proposal: <id>

  Objective: <one sentence>
  
  📖 Read:    <files>
  ✏️ Write:   <files>
  🔍 Gates:   <gate list>
  ⚠️ Risks:   <risks>
  📋 Specs:   <which spec files were loaded>
  
  ✅ approve  ✏️ adjust  ❌ skip
```

5. **After approval** → execute task lifecycle commands below.

## Command Reference

| Action | Commands (run in order) |
|---|---|
| **New task** | `ctl task create --id <id> --objective "<text>" --read-scope <path>... --write-allow <path>... --gates <gate>...` → `ctl task ready --id <id>` → `ctl task start --id <id>` → `ctl run start --id <id> --adapter omp` |
| **Apply changes** | `ctl run diff --id <id>` → `ctl run apply --id <id>` → `ctl run ingest --id <id>` |
| **Close task** | `ctl run gates --id <id>` → `ctl task submit --id <id>` → `ctl task finish --id <id>` → `ctl task archive --id <id>` |
| **Abort task** | `ctl run stop --id <id>` → `ctl task cancel --id <id>` |
| **Check status** | `ctl task status --id <id>` |
| **Health check** | `ctl doctor` |
| **Generate specs** | `/ctl-spec-bootstrap` |
| **Update specs** | `/ctl-spec-update` |

## Implementation Phase

After task is created and started:

1. Work inside the OMP worktree
2. **Before every file write**: verify target is within `write_allow`
3. When implementation is complete → run Apply commands
4. After Apply → run Close commands
5. After Close → check if spec update is needed (new patterns? non-obvious gotchas?) → if yes, `/ctl-spec-update`

## On Failure

When something breaks (gate fail, boundary violation, crash):

1. Read the relevant guide **on demand** — don't preload:
   - Estimating fix scope → `.ctl/spec/guides/complexity-classification.md`
   - Diagnosing cause → `.ctl/spec/guides/failure-diagnosis.md`
   - Deep root cause → `.ctl/spec/guides/first-principles.md`
2. These are **reference materials**, not auto-triggers. Read them when you need them.
3. If diagnosis reveals a pattern worth preserving → `/ctl-spec-update`

## Error Handling

- **Path rejected**: `ctl boundary explain --path <path>`, fix path, retry. Never widen to root.
- **Task id exists**: Propose new id. Never mutate existing events.
- **Active run blocks start**: Abort first.
- **Any command fails**: STOP, report error, do not skip steps.

## Anti-Patterns

- ❌ Run `ctl` directly without going through this skill
- ❌ Skip spec loading before proposing boundaries
- ❌ Modify files outside `write_allow`
- ❌ Manually edit `events.jsonl` or `task.json`
- ❌ Let knowledge stay in chat — capture to specs via `/ctl-spec-update`
