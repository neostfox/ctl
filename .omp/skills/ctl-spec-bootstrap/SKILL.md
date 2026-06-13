---
name: ctl-spec-bootstrap
description: "Analyze the current project source code and generate .ctl/spec/ documentation. Run once when introducing ctl to a new project, or to refresh specs after significant refactoring. Produces concrete, codebase-backed specs — no placeholder text."
---

# /ctl-spec-bootstrap — Generate Project Specs from Source

Analyzes the real codebase and produces `.ctl/spec/` documentation with concrete patterns, file paths, conventions, and anti-patterns extracted from source. Run when:

- Introducing `ctl` to a new project (`ctl init` already ran)
- After significant refactoring that invalidates existing specs
- User explicitly requests `/ctl-spec-bootstrap`

## Step 0: Verify prerequisites

```powershell
# Ensure ctl is initialized
ctl doctor
```

If `.ctl/` doesn't exist, run `ctl init` first.

## Step 1: Detect project type

Read the root to determine language and build system:

| File found | Project type | Spec shape |
|---|---|---|
| `Cargo.toml` | Rust | `backend/` layer specs by module |
| `package.json` | Node.js/TypeScript | `backend/` layer specs by src directory |
| `go.mod` | Go | `backend/` layer specs by package |
| `pyproject.toml` / `setup.py` | Python | `backend/` layer specs by module |
| Mixed | Polyglot | `backend/` per language + `guides/cross-language.md` |

If `.ctl/spec/` already exists, read existing files first — this is a **refresh**, not overwrite.

## Step 2: Map architecture

### 2.1 Directory structure

Read the top-level directory tree (depth 3). Identify:

- **Source directories**: where implementation lives
- **Test directories**: test conventions (unit, integration, fixture)
- **Config directories**: schemas, fixtures, CI
- **Entry point**: `main.rs`, `index.ts`, `main.go`, `__main__.py`
- **Build artifacts**: `target/`, `dist/`, `build/` (skip these)

### 2.2 Layer boundaries

For each source directory, determine its role:

| Role | Indicators |
|---|---|
| **CLI / Entry** | Argument parsing, command dispatch, output formatting |
| **Application / Service** | Business orchestration, validation before persistence |
| **Domain / Core** | Pure data types, state machines, business rules, no I/O |
| **Infrastructure / Adapter** | Filesystem, network, database, external APIs |
| **Adapter / Interface** | Protocol definitions, DTOs, external interfaces |
| **Shared / Utils** | Cross-cutting utilities, helpers |

### 2.3 Dependency direction

Trace imports between layers:

1. Read the main module files (`mod.rs`, `index.ts`, `__init__.py`)
2. Identify which modules import which
3. Verify the direction: outer → inner. Flag violations.

**Output**: A dependency diagram like:
```
cli → application → domain
infrastructure/* → domain
adapters → application (DTO only)
```

## Step 3: Extract coding conventions

### 3.1 Error handling

Search for error patterns:
- What error type is used? (`anyhow`, `thiserror`, custom `Result<T, E>`, exceptions, etc.)
- Where are errors created? (validation layer vs propagation)
- How are errors surfaced to users? (exit codes, stderr, error types)

### 3.2 Naming conventions

From actual code, extract:
- Module/function/type naming style (snake_case, camelCase, PascalCase)
- File naming patterns
- Test naming patterns
- Constant naming

### 3.3 Testing conventions

From test files:
- Test location (colocated vs separate `tests/` dir)
- Test naming convention
- Test structure (arrange-act-assert, given-when-then)
- Fixture usage patterns
- Mock strategy (if any)

### 3.4 State management

If the project has state:
- Where is state defined?
- How is it mutated? (reducer pattern, direct mutation, ORM)
- What is the canonical truth source?
- Are there projections/caches?

### 3.5 I/O boundaries

Identify where I/O happens:
- File reads/writes
- Network calls
- Database access
- Process spawning

Verify these are isolated to infrastructure/adapter layers.

## Step 4: Generate spec files

Write concrete specs. Every rule must reference real code. No template placeholders.

### File structure

```
.ctl/spec/
├── backend/
│   ├── index.md                    # Overview + architecture diagram + pre-dev checklist
│   ├── directory-structure.md      # Module layout with descriptions
│   ├── domain-layer.md             # Core types, state machine, purity rules
│   ├── application-layer.md        # Service/orchestration patterns
│   ├── infrastructure-layer.md     # I/O, storage, external integrations
│   ├── cli-layer.md                # Command surface, parsing, output
│   ├── error-handling.md           # Error types, propagation, user messages
│   ├── quality-guidelines.md       # Forbidden patterns, required patterns, testing
│   └── logging-output-guidelines.md # Output conventions, exit codes
└── guides/
    ├── index.md                    # Thinking guides overview
    └── cross-layer-thinking-guide.md # What to consider when touching multiple layers
```

**Rules**:
- Skip files that don't apply to the project (e.g., no CLI → no `cli-layer.md`)
- Merge into existing specs when refreshing (don't delete sections that are still valid)
- "How to write code" → `backend/`. "What to think about" → `guides/`

### 4.1 index.md template

```markdown
# [Project Name] Development Guidelines

> Coding conventions for [project description].

## Architecture

[Dependency diagram from Step 2.3]

[Layer descriptions from Step 2.2]

## Pre-Development Checklist

Before writing code:
- [ ] Read the layer spec for the target module
- [ ] Verify change scope against dependency direction
- [ ] Check for forbidden patterns in quality-guidelines.md

## Quality Check

After implementation:
- [ ] [Build command] passes
- [ ] [Test command] passes
- [ ] [Lint command] passes
- [ ] No layer boundary violations

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| ... | ... | Filled/Refreshed |
```

### 4.2 Per-layer specs

Each layer spec MUST contain:
1. **Purpose**: One sentence on what this layer does
2. **Allowed imports**: What this layer may depend on
3. **Forbidden imports**: What this layer MUST NOT depend on
4. **Patterns**: Good examples from actual code (with file paths)
5. **Anti-patterns**: Things to avoid, with reasoning

### 4.3 Quality guidelines

Extract from actual code:
- **Forbidden patterns**: Things that cause bugs (with real examples of why)
- **Required patterns**: Things that must always be done (with real examples)
- **Testing requirements**: Coverage expectations, fixture rules

## Step 5: Verify generated specs

Before finishing, validate:

1. **No placeholders**: Search for `TODO`, `FIXME`, `[placeholder]`, `<example>`. Remove all.
2. **Real file paths**: Every path mentioned must actually exist in the project.
3. **Real code examples**: Every code block must be from actual source (or a correct simplification).
4. **Consistency**: Rules in different files don't contradict each other.
5. **Completeness**: Every source directory is covered.

## Step 6: Report

```
✅ Specs generated for [project name]

  .ctl/spec/backend/
    index.md                      (architecture overview + checklists)
    directory-structure.md        (module layout)
    domain-layer.md               (core types, purity rules)
    application-layer.md          (orchestration patterns)
    infrastructure-layer.md       (I/O, storage)
    cli-layer.md                  (command surface)
    error-handling.md             (error patterns)
    quality-guidelines.md         (forbidden/required patterns)
    logging-output-guidelines.md  (output conventions)
  .ctl/spec/guides/
    index.md                      (thinking guides)
    cross-layer-thinking-guide.md (multi-layer considerations)

  Source files analyzed: [N]
  Patterns extracted: [N]
  Anti-patterns documented: [N]

  Refresh with: /ctl-spec-bootstrap
```

## Rules

- **Idempotent**: Re-running updates stale sections, preserves manually-added content.
- **Source-backed**: Every recommendation points at a real file or repeated pattern.
- **No generic advice**: Don't write "use good variable names" — write "use snake_case for functions (see `src/domain/task.rs:apply()`)".
- **Language-appropriate**: Convention names and patterns must match the project's actual language.
- **Respect existing**: If `.ctl/spec/` already has good content, refresh only what changed.
