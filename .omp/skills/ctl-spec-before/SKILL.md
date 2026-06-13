---
name: ctl-spec-before
description: "Load project specs and coding conventions before starting implementation. Auto-triggered by control-guard before /ctl-new. Mandatory before writing any code."
---

# /ctl-spec-before — Pre-Development Spec Loading

Auto-triggered before every `/ctl-new`. Loads the project's coding standards, constraints, and conventions so the agent doesn't violate them during implementation.

## Step 1: Read the backend spec index

```powershell
# Read the main spec index
```

Read `.ctl/spec/backend/index.md` — this contains:
- Architecture layer overview and dependency direction
- Pre-development checklist
- Links to layer-specific guidelines

## Step 2: Identify applicable layer specs

Based on the task's `write_allow`, determine which layers are affected:

| Target | Read this spec |
|---|---|
| `src/domain/` | `.ctl/spec/backend/domain-layer.md` |
| `src/application/` | `.ctl/spec/backend/cli-layer.md` (application patterns) |
| `src/cli/` | `.ctl/spec/backend/cli-layer.md` |
| `src/infrastructure/` | `.ctl/spec/backend/infrastructure-layer.md` |
| `src/adapters/` | `.ctl/spec/backend/cli-layer.md` (adapter protocol) |
| Multiple layers | Read ALL affected layer specs + cross-layer guide |

## Step 3: Read cross-cutting guides

Always read when applicable:

| Condition | Read this guide |
|---|---|
| Feature touches 3+ layers | `.ctl/spec/guides/cross-layer-thinking-guide.md` |
| Adding similar code to existing | `.ctl/spec/guides/code-reuse-thinking-guide.md` |
| Adding/modifying event types | Cross-layer guide + domain spec |
| Adding boundary fields | Cross-layer guide (field must touch reducer + schema + CLI + fixtures) |

## Step 4: Check guardrails

```powershell
# Read the architecture guardrails for milestone scope
```

Read `ARCHITECTURE_GUARDRAILS.md` to verify:
- Change belongs to current milestone (M0–M3 only)
- No forbidden dependency is being introduced
- No milestone scope violation

## Step 5: Extract key constraints

From the specs, extract and keep in context during implementation:

1. **Dependency direction**: `cli → application → domain`, `infrastructure/* → domain`. `domain/` MUST NOT import from cli/infrastructure/adapters.
2. **Truth model**: `events.jsonl` is append-only canonical truth. `task.json` is replay projection.
3. **Reducer purity**: `apply(&mut TaskState, &Event)` — no side effects, no I/O, no time.
4. **Path handling**: All paths through `PathNormalizer` before policy decisions.
5. **Allowed dependencies**: clap, serde/serde_json, anyhow, sha2 only.
6. **Event types**: New types need reducer branch + schema + fixture coverage.

## Output

After loading specs, add a constraints section to the task proposal:

```
📋 Constraints loaded from specs:
  - Layer: [affected layers]
  - Dependency direction: [any cross-layer concerns]
  - Milestone scope: [M0-M3 OK / M4+ BLOCKED]
  - New deps needed: [none / YES → must justify]
  - Cross-layer impact: [none / list affected layers]
```

## Integration with control-guard

This skill is auto-triggered by control-guard BEFORE the proposal is presented. The user never needs to invoke it manually. The loaded constraints feed directly into the proposal's risk assessment and boundary validation.
