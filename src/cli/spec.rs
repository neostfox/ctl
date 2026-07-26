use super::*;

/// PRD template (workflow-prd-to-tasks-v1). `{title}` is substituted at runtime.
/// The `## Tasks` section is a deliberate, parseable convention — `id` /
/// `objective` / `write-allow` / `gates` per vertical task — so a later
pub(super) const PRD_TEMPLATE: &str = r#"# PRD: {title}

> Status: draft
> Fill this out (the grill step), then `ctl prd plan --file <this>.md` can turn
> the `## Tasks` section into ctl tasks. Change `Status` to `confirmed` once the
> plan is accepted; until then this is a shape to fill, not an executable spec.

## Objective

<one paragraph: the outcome this PRD delivers, and why>

## Context

<links, constraints, prior art, explicit non-goals>

## Tasks

<!-- One list item per vertical (independently shippable) task. Conventions the
     `ctl prd plan` parser relies on:
       - id:          kebab-case, unique
       - objective:   non-empty, one line
       - write-allow: comma-separated paths (the task's write boundary)
       - gates:       comma-separated gate template ids
       - read-scope:  optional, comma-separated paths (defaults to write-allow)
       - depends-on:  optional, comma-separated task ids (M-d dependency edges)
     Keep each task small enough that one agent can finish it within its
     boundary. -->

- id: <kebab-id>
  objective: <non-empty objective>
  write-allow: <path>[, <path> ...]
  gates: cargo_fmt_check, cargo_check, cargo_clippy, cargo_test
  read-scope: <path>[, <path> ...]
  depends-on: <kebab-id>[, <kebab-id> ...]
"#;

pub(super) fn cmd_prd(command: &PrdCommands, global_dry_run: bool) -> Result<()> {
    use crate::application::prd::{parse_prd, PrdDocument};

    let read_prd = |file: &str| -> Result<PrdDocument> {
        let content = std::fs::read_to_string(file)
            .map_err(|e| anyhow::anyhow!("Failed to read PRD file '{}': {}", file, e))?;
        parse_prd(&content).map_err(|e| anyhow::anyhow!("Failed to parse PRD '{}': {}", file, e))
    };

    match command {
        PrdCommands::Init { title } => {
            print!("{}", PRD_TEMPLATE.replace("{title}", title));
            Ok(())
        }

        PrdCommands::Validate { file, json } => {
            let app = app_open(false)?;
            let doc = read_prd(file)?;
            let validation = app.prd_validate(&doc)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&validation)?);
            } else {
                render_prd_validation(&validation, &doc);
            }
            Ok(())
        }

        PrdCommands::Plan {
            file,
            dry_run,
            alignment,
        } => {
            // The PRD file is the convergence artifact; alignment is divergence.
            let doc = read_prd(file)?;
            let dry = *dry_run || global_dry_run;
            let app = app_open(dry)?;
            let outcomes = app.prd_plan(&doc, alignment.as_deref(), Some(file.as_str()), dry)?;
            render_prd_plan(&outcomes, dry);
            Ok(())
        }

        PrdCommands::Status { file, json } => {
            let app = app_open(false)?;
            let doc = read_prd(file)?;
            let view = app.prd_status_view(&doc)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&view)?);
            } else {
                render_prd_status(&view);
            }
            Ok(())
        }
    }
}

/// Render a validation result as human-readable text. Never a verdict — just
/// the problems found, grouped by severity.
pub(super) fn render_prd_validation(
    validation: &crate::application::prd::PrdValidation,
    doc: &crate::application::prd::PrdDocument,
) {
    let errors = validation.errors();
    let warnings = validation.warnings();
    println!(
        "PRD '{}' (status: {}): {} task(s)",
        doc.title,
        doc.status.as_str(),
        doc.tasks.len()
    );
    if errors.is_empty() && warnings.is_empty() {
        println!("OK — no format, boundary, gate, or overlap problems found.");
        return;
    }
    if !errors.is_empty() {
        println!("ERRORS (block `ctl prd plan`):");
        for p in &errors {
            match &p.task_id {
                Some(tid) => println!("  [{}] {}", tid, p.message),
                None => println!("  {}", p.message),
            }
        }
    }
    if !warnings.is_empty() {
        println!("WARNINGS:");
        for p in &warnings {
            match &p.task_id {
                Some(tid) => println!("  [{}] {}", tid, p.message),
                None => println!("  {}", p.message),
            }
        }
    }
}

/// Render a plan outcome (dry-run preview or real creation summary).
pub(super) fn render_prd_plan(outcomes: &[crate::application::prd::PrdPlanOutcome], dry: bool) {
    if dry {
        println!("[dry-run] Would plan {} task(s):", outcomes.len());
    } else {
        println!("Planned {} task(s):", outcomes.len());
    }
    for o in outcomes {
        let deps = if o.depends_on.is_empty() {
            String::from("(none)")
        } else {
            o.depends_on.join(", ")
        };
        let prov = if o.provenance_recorded {
            " + provenance"
        } else {
            ""
        };
        match o.seq {
            Some(seq) => println!(
                "  {} [seq {}] — {} | write: {} | gates: {} | deps: {}{}",
                o.task_id,
                seq,
                o.objective,
                o.write_allow.join(", "),
                o.gates.join(", "),
                deps,
                prov
            ),
            None => println!(
                "  {} — {} | write: {} | gates: {} | deps: {}",
                o.task_id,
                o.objective,
                o.write_allow.join(", "),
                o.gates.join(", "),
                deps
            ),
        }
    }
    if dry {
        println!("Next: edit the PRD, validate, then `ctl prd plan --file <this>.md`");
    } else if let Some(first) = outcomes.first() {
        println!(
            "Next: ctl task ready --id {}  (then ctl board to see all planned tasks)",
            first.task_id
        );
    }
}

/// Render the observable-loop status view.
pub(super) fn render_prd_status(view: &crate::application::prd::PrdStatusView) {
    println!(
        "PRD '{}' — status: {} — {}/{} completed",
        view.title,
        view.status.as_str(),
        view.completed,
        view.total
    );
    for row in &view.rows {
        match (&row.phase, &row.provenance) {
            (None, _) => println!("  {} — not created yet", row.id),
            (Some(phase), Some(prov)) => {
                let conv = prov
                    .convergence
                    .as_ref()
                    .map(|a| a.path.as_str())
                    .unwrap_or("(none)");
                println!("  {} — {} — provenance: {}", row.id, phase, conv);
            }
            (Some(phase), None) => println!("  {} — {} — no provenance recorded", row.id, phase),
        }
    }
}

pub(super) fn cmd_spec(command: &SpecCommands, global_dry_run: bool) -> Result<()> {
    match command {
        SpecCommands::Fact { command } => cmd_spec_fact(command, global_dry_run),
    }
}

pub(super) fn cmd_spec_fact(command: &FactCommands, global_dry_run: bool) -> Result<()> {
    match command {
        FactCommands::Add {
            statement,
            source,
            category,
            dry_run,
        } => {
            let dry = *dry_run || global_dry_run;
            let app = app_open(dry)?;
            if dry {
                println!(
                    "[dry-run] Would record fact: \"{}\" (source: {}, category: {})",
                    statement,
                    source,
                    category.as_deref().unwrap_or("(none)")
                );
                return Ok(());
            }
            let fact = app.spec_fact_add(statement, source, category.as_deref())?;
            println!(
                "Recorded fact '{}' (category: {}) at {} — source: {}",
                fact.fact_id,
                category.as_deref().unwrap_or("uncategorized"),
                fact.recorded_at,
                fact.source
            );
            println!("  {}", fact.statement);
            println!("Next: ctl spec fact list");
            Ok(())
        }

        FactCommands::List {
            category,
            search,
            json,
        } => {
            let app = app_open(false)?;
            let facts = app.spec_fact_list(category.as_deref(), search.as_deref())?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&facts)?);
                return Ok(());
            }
            if facts.is_empty() {
                println!("No facts found.");
                return Ok(());
            }
            println!("Knowledge base: {} fact(s)", facts.len());
            for f in &facts {
                let cat = f.category.as_deref().unwrap_or("uncategorized");
                println!("  {} [{}] — {}", f.fact_id, cat, f.statement);
                println!("    source: {}", f.source);
            }
            Ok(())
        }

        FactCommands::Promote { id, to } => {
            let app = app_open(false)?;
            let path = app.spec_fact_promote(id, to)?;
            println!("Promoted fact '{}' into {}", id, path.display());
            Ok(())
        }
    }
}
