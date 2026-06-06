# CLI Layer Spec

> Conventions for `src/cli/` — command parsing, output formatting, and architecture checks.

---

## Overview

The CLI layer uses `clap` derive macros to define the `control` command surface. It delegates all business logic to `application::ControlApp` and handles output formatting, error reporting, and exit codes.

```
src/cli/mod.rs → Cli struct, Commands enum, run() entry point
```

---

## Command Surface (M0–M1)

```text
control init
control task create --id <id> --objective <text> --read-scope <path>... --write-allow <path>... --gates <gate>...
control task revise --id <id> [--objective <text>] [--read-scope <path>...] ...
control task ready --id <id>
control task status --id <id>
control task cancel --id <id>
control replay [--task <id>]
control validate
control doctor
control schema validate --file <path>
control boundary check --path <path>
control boundary explain --path <path>
control architecture check
```

### Architecture Guard

`control architecture check` verifies:
- Only M1 commands are exposed (no M2+ commands in M0 build)
- Schema contracts are valid
- No `.control/events.jsonl` canonical store usage
- Dependency direction is maintained

---

## Clap Patterns

### Command Definition

```rust
#[derive(Parser)]
#[command(name = "control")]
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

### Optional Flags

```rust
#[arg(long = "write-deny")]
write_deny: Vec<String>,  // defaults to empty Vec
```

---

## Entry Point Pattern

```rust
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init => { /* ... */ },
        Commands::Task { command } => { /* ... */ },
        // ...
    }
    Ok(())
}
```

**Important**: `run()` returns `anyhow::Result<()>`. The `main()` function calls `cli::run()` and lets the error propagate. `anyhow` error formatting goes to stderr.

---

## Output Formatting

### Success Pattern

```rust
println!("Task '{}' created (Planning phase).", id);
println!("Next: control task ready --id {}", id);
```

### Status Display

```rust
println!("Task: {}", state.id);
println!("Phase: {:?}", state.phase);
println!("Objective: {}", state.objective.as_deref().unwrap_or("(none)"));
println!("Read scope: {:?}", state.read_scope);
println!("Gates: {:?}", state.gates);
if state.is_held {
    println!("Status: HELD");
}
```

### Error Output

Errors from `application` are caught and formatted:

```rust
if let Err(e) = app.create_task(&id, input) {
    eprintln!("Error: {}", e);
    std::process::exit(1);
}
```

---

## Project Root Discovery

The CLI discovers the project root by walking up from `std::env::current_dir()` looking for `.trellis/`:

```rust
fn find_project_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(".trellis").exists() {
            return Some(dir);
        }
        if !dir.pop() { return None; }
    }
}
```

---

## Architecture Checks

### Command Surface Whitelist

The `architecture check` command verifies that only M1-allowed commands are exposed in the binary. This prevents accidental milestone-scope creep.

### Schema Contract Verification

- All schemas in `schemas/` are loadable.
- No unknown `unevaluatedProperties` violations.

### Store Path Verification

- No code writes to `.control/events.jsonl`.
- Canonical events only go to `.trellis/tasks/<id>/events.jsonl`.

---

## Common Mistakes

### Mistake 1: Business Logic in CLI

```rust
// BAD: validation logic in CLI
if objective.is_empty() {
    eprintln!("Error: objective must not be empty");
    return Ok(());
}

// GOOD: delegate to application layer
if let Err(e) = app.create_task(&id, input) {
    eprintln!("Error: {}", e);
}
```

### Mistake 2: Missing Next-Step Hint

Every successful mutating command should print a suggestion for the next command. Users should never wonder "what now?"

### Mistake 3: Hardcoded Paths

```rust
// BAD
let root = Path::new("C:\\Users\\shaob\\project");

// GOOD: discover from environment
let root = find_project_root().expect("Not in a control project");
```

### Mistake 4: Panicking on User Input

```rust
// BAD
let id = args.id.unwrap();

// GOOD: clap handles required args; Optional args use if-let
```
