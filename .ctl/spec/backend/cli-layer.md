# CLI Layer Spec

> Conventions for `src/cli/` — command parsing, output formatting, and architecture checks.

---

## Overview

The CLI layer uses `clap` derive macros to define the `ctl` command surface. It delegates all business logic to `application::ControlApp` and handles output formatting, error reporting, and exit codes.

```
src/cli/mod.rs → Cli struct, Commands enum, run() entry point
```

---

## Command Surface (0.0.11+)

```text
ctl init [--claude] [--opencode] [--omp] [--all] [--platform <name>] [--yes]
ctl task create|quick|revise|ready|start|submit|reopen|finish|cancel|archive|status
ctl board [--kanban|--table] [--active] [--include-archived] [--json]
ctl update --merge [--force|--skip]
ctl self-update [--version <tag>] [--check]
ctl handoff export|capture
ctl gate run|record
ctl replay [--task <id>]
ctl reconcile | validate | doctor
ctl schema validate --file <path>
ctl boundary check|explain|check-by-id
ctl architecture check|review
```

> The binary name is `ctl` (not `control`). The old `control` name was retired before 0.0.1.

### Architecture Guard

`ctl architecture check` verifies:
- Dependency direction (cli → application → domain)
- Schema contracts are valid
- Module purity (domain has no I/O)
- Command surface matches the expected subcommand list
- State transitions, fixture/gate shape, baseline manifest

---

## Clap Patterns

### Command Definition

```rust
#[derive(Parser)]
#[command(name = "ctl")]
#[command(about = "AI Dev Control Plane CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
```

### Repeated Flags

Multi-value flags use `Vec<String>` with explicit long names:

```rust
#[arg(long = "read-scope", required = true)]
read_scope: Vec<String>,
```

### Multi-Platform Init

`ctl init` accepts repeated `--platform`, individual flags (`--claude`, `--opencode`,
`--omp`, `--all`), and `--yes` for scripted onboarding. `PlatformSelection` resolves
all sources into a struct of bools; `detected_platform_selection` inspects the project
root for existing `.claude/`, `.opencode/`, `.omp/` directories.

---

## Output Formatting

### Success Pattern

```rust
println!("Task '{}' created (Planning phase).", id);
println!("Next: ctl task ready --id {}", id);
```

### Kanban Board

`ctl board` defaults to a terminal Kanban (phase columns: PLANNING / READY /
IN PROGRESS / REVIEW / DONE). `--table` falls back to the legacy flat table.
`--json` returns the same structured JSON (unchanged contract).

### Project Update

`ctl update --merge` syncs embedded workflow/hook/skill templates into configured
platforms. User-modified files get a `.new` sibling by default (never overwritten);
`--force` overwrites; `--skip` leaves them untouched. A baseline manifest
(`.ctl/ctl-template-hashes.json`) tracks which files are ctl-owned.

---

## Project Root Discovery

The CLI discovers the project root by walking up from `std::env::current_dir()` looking for `.ctl/`.

---

## Common Mistakes

### Mistake 1: Business Logic in CLI

Delegate to the application layer; the CLI formats output and reads flags.

### Mistake 2: Missing Next-Step Hint

Every successful mutating command should print a suggestion for the next command.

### Mistake 3: Hardcoded Paths

Always discover from `std::env::current_dir()`, never hardcode.

### Gotcha: classify_bash reads prose, not just commands

> **Warning**: `classify_bash` strips quoted spans before segment-splitting (fixed in
> 0.0.11). Keep trigger phrases (`cargo install`, `git push`) off segment starts in
> commit messages as a defensive habit.
