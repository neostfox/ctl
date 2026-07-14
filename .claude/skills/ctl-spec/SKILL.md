---
name: ctl-spec
description: "Project spec lifecycle in one skill: bootstrap .ctl/spec/ from source on first run or after major refactoring, then capture knowledge (design decisions, patterns, gotchas, root causes) into .ctl/spec/ (project) or ~/.ctl/memory/ (global cross-project) as work proceeds. Auto-triggered by control-guard after ctl init, task finish, or when ctl-diagnose reveals a pattern worth preserving. Writes only under .ctl/, ~/.ctl/memory/, CLAUDE.md, AGENTS.md — never project source."
---

# /ctl-spec — Spec Bootstrap + Knowledge Capture

Two modes, one skill:

- **Bootstrap** (first run / after major refactor): analyze the real codebase, generate `.ctl/spec/` — architecture diagrams, directory trees, layer conventions, concrete patterns extracted from source.
- **Capture** (ongoing, after task finish / diagnosis): write hard-won knowledge into `.ctl/spec/` (project) or `~/.ctl/memory/` (global) so future sessions benefit.

## Operating boundary

Writes ONLY under: `.ctl/`, `~/.ctl/memory/`, `CLAUDE.md`, `AGENTS.md`. Never edits project source, build manifests, or CI config. If a step appears to need a source edit, stop and report it — do not do it.

## Bootstrap — generate .ctl/spec/ from source

Run after `ctl init`, or when specs are stale relative to code changes.

1. Read the real codebase: manifest (`Cargo.toml` / `package.json` / `go.mod` / `pom.xml`), source layout, CI workflows, real test commands, dependencies.
2. Produce `.ctl/spec/` with concrete, executable content:
   - `backend/index.md` — overview + pre-dev checklist + design decisions
   - `backend/<layer>.md` — domain / infrastructure / cli layer guidelines
   - `backend/error-handling.md`, `quality-guidelines.md` — cross-cutting standards
   - `guides/` — thinking checklists (what to consider, not how to code)
3. Include an architecture diagram (Mermaid), a directory tree, and real file paths. Extract patterns and anti-patterns FROM source — never paste generic advice.
4. **Never overwrite existing `.ctl/spec/` wholesale** — refresh in place, preserve user edits. Idempotent.

Supports: Rust, TypeScript/JavaScript (frontend + Node), Java (Maven/Gradle), Go, Python, mixed-language.

**Rule**: "How to write code" → `backend/`. "What to think about" → `guides/`.

## Capture — write knowledge as you earn it

Auto-trigger after `ctl task finish` (when the task revealed non-obvious patterns) or after `ctl-diagnose` (root cause future sessions should avoid), or when the user says "remember this".

### Step 0 — Tier: global vs project

Every captured fact lands in exactly one tier:

| Tier | Location | What belongs |
|---|---|---|
| **Global** | `~/.ctl/memory/` (cross-project) | Stable user preferences, working style — would hold in any repo |
| **Project** | `.ctl/spec/` (this repo) | Conventions, patterns, gotchas, decisions from THIS codebase |

Decision rule: **would this still be true in a brand-new repository?** Yes → global. No → project. Unsure → project (a repo fact leaking into global pollutes every session; a preference kept project-local merely waits to be promoted).

Global format: one fact per file (`<slug>.md`) + a one-line pointer in `~/.ctl/memory/MEMORY.md`. Never store project paths, file names, or repo-specific commands in the global tier.

### Step 1 — Classify the capture

| Trigger | Type | Target spec |
|---|---|---|
| Implemented feature | Convention / Pattern | `backend/<layer>.md` |
| Fixed a bug | Root Cause / Gotcha | `backend/error-handling.md` |
| Discovered a pattern | Pattern | `backend/<layer>.md` |
| Hit a gotcha | Gotcha | relevant `backend/*.md` |
| Design choice | Design Decision | `backend/index.md` |
| Forbidden approach | Anti-pattern | relevant `backend/*.md` |

### Step 2 — Read before editing

Read the target spec to avoid duplication. Check `backend/index.md` for the guidelines index if unsure which file.

### Step 3 — Write the update

Rules:
1. **Be specific** — include code examples, not just abstract rules
2. **Explain why** — state the problem this prevents
3. **Show contracts** — signatures, payload fields, error behavior
4. **One concept per section**
5. **Wrong vs correct examples**

Templates:

**Design Decision** — Context / Options / Choice / Extensibility.
**Pattern** — Problem / Solution / good code / bad code.
**Gotcha** — `> Warning: [non-obvious behavior] — when it happens and how to handle it`.

## Quality checklist

Before finishing a capture:
- [ ] Specific and actionable?
- [ ] Includes a code example?
- [ ] Explains WHY not just WHAT?
- [ ] In the right file (backend/ vs guides/)?
- [ ] Doesn't duplicate existing content?
- [ ] A new team member would understand it?
