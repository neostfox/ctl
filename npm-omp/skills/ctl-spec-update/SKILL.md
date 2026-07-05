---
name: ctl-spec-update
description: "Capture knowledge from debugging, implementation, or discussion into two tiers: project facts into .ctl/spec/, stable cross-project preferences into ~/.ctl/memory/ (global, adapter-referenced). Auto-triggered by control-guard after a task finishes (ctl task finish) or after ctl-diagnose reveals a pattern worth preserving."
---

# /ctl-spec-update — Knowledge Capture into Specs

Auto-triggered after task completion or when a debugging session reveals something worth preserving. Updates `.ctl/spec/` so future sessions benefit from hard-won knowledge.

## When to auto-trigger

- After `ctl task finish` completes successfully AND the task revealed non-obvious patterns
- After Bayesian diagnosis identifies a root cause that future sessions should avoid
- When the user says "remember this" or "note this down"
- When a gate failure reveals a missing spec check

## Step 0: Choose the tier — global vs project

Every captured fact lands in exactly one tier:

| Tier | Location | What belongs there |
|---|---|---|
| **Global** | `~/.ctl/memory/` (user-level, cross-project) | Stable user preferences and ways of working: how the user wants proposals confirmed, review style, risk tolerance, recurring cross-repo workflows |
| **Project** | `.ctl/spec/` (this repo) | Everything derived from THIS codebase: conventions, patterns, gotchas, design decisions, root causes |

Decision rule: **would this still be true in a brand-new repository?** Yes →
global. No → project. When unsure, choose project — a repo fact leaking into
the global tier pollutes every session, while a preference kept project-local
merely waits to be promoted.

Global-tier format (one truth across platforms, adapters reference it): one
fact per file (`<slug>.md` — the fact, why, how to apply, a few lines), plus a
one-line pointer appended to `~/.ctl/memory/MEMORY.md` — the index a session
loads first. Never store project paths, file names, or repo-specific commands
in the global tier. Writing `~/.ctl/memory/` is an out-of-repo write: the gate
observes and records it (`.ctl/decisions.jsonl`) rather than denying —
expected and disclosed.

## Step 1: Identify what was learned

| Trigger | Example | Target spec |
|---------|---------|-------------|
| Implemented a feature | Added new event type | `.ctl/spec/backend/domain-layer.md` |
| Fixed a bug | Subtle cross-layer format mismatch | `.ctl/spec/guides/cross-layer-thinking-guide.md` |
| Discovered a pattern | Better way to handle path normalization | `.ctl/spec/backend/infrastructure-layer.md` |
| Hit a gotcha | `PathNormalizer` returns `\` on Windows | `.ctl/spec/backend/error-handling.md` |
| Established a convention | Event seq must be strictly ascending | `.ctl/spec/backend/quality-guidelines.md` |
| Design decision | Chose worktree isolation over in-place edits | `.ctl/spec/backend/index.md` (design decisions) |

## Step 2: Classify the update

| Type | Action |
|------|--------|
| **Design Decision** | Add "Design Decisions" section with context/options/choice |
| **Convention** | Add to relevant section with examples |
| **Pattern** | Add to "Patterns" section with good/bad code examples |
| **Forbidden Pattern** | Add to "Anti-patterns" with explanation |
| **Gotcha** | Add warning callout |
| **Root Cause** | Add to error-handling or quality-guidelines |

## Step 3: Read before editing

Read the target spec to avoid duplication:

```powershell
# Read the target spec file first
```

Check `.ctl/spec/backend/index.md` for the guidelines index if unsure which file.

## Step 4: Write the update

Rules:
1. **Be specific**: Include code examples, not just abstract rules
2. **Explain why**: State the problem this prevents
3. **Show contracts**: Add signatures, payload fields, error behavior
4. **One concept per section**
5. **Include wrong vs correct examples**

### Update templates

**Design Decision:**
```markdown
### Decision: [Name]
**Context**: What problem were we solving?
**Options**: A vs B vs C
**Choice**: We chose X because...
**Extensibility**: How to extend this later...
```

**Pattern:**
```markdown
### Pattern: [Name]
**Problem**: What it solves.
**Solution**: How it works.
// Good: code example
// Bad: code example
```

**Gotcha:**
```markdown
> **Warning**: [non-obvious behavior]
> When this happens and how to handle it.
```

## Step 5: Update the index if needed

If a new section was added or status changed, update `.ctl/spec/backend/index.md`.

## Spec directory structure

```
.ctl/spec/
├── backend/           # Coding standards (concrete, executable)
│   ├── index.md       # Overview + pre-dev checklist + quality check
│   ├── domain-layer.md
│   ├── infrastructure-layer.md
│   ├── cli-layer.md
│   ├── error-handling.md
│   ├── quality-guidelines.md
│   ├── storage-guidelines.md
│   ├── logging-output-guidelines.md
│   ├── directory-structure.md
│   └── m3-dogfood-findings.md
└── guides/            # Thinking checklists (what to consider)
    ├── index.md
    ├── cross-layer-thinking-guide.md
    └── code-reuse-thinking-guide.md
```

**Rule**: "How to write code" → `backend/`. "What to think about" → `guides/`.

## Quality checklist

Before finishing:
- [ ] Specific and actionable?
- [ ] Includes code example?
- [ ] Explains WHY not just WHAT?
- [ ] In the right file (backend/ vs guides/)?
- [ ] Doesn't duplicate existing content?
- [ ] A new team member would understand it?
