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
        SpecCommands::Doctor { json } => cmd_spec_doctor(*json),
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

/// Scan `.ctl/spec/**/*.md` for stale code-path references and report any that
/// no longer exist on disk. Read-only. [ROADMAP #2/S]
///
/// A backtick token is treated as a candidate file path when it contains a `/`
/// (so bare symbols like `Phase::InProgress`, flags like `--write-allow`, and
/// commands like `cargo test` are skipped), is not a URL or flag, has no spaces,
/// and is not itself a path under `.ctl/spec/` (self-references within the spec
/// tree are not "stale source"). A trailing `:<line>` anchor is stripped before
/// the existence check, so `src/foo.rs:42` is checked as `src/foo.rs`. This is a
/// heuristic — it favours precision over recall so the report stays actionable.
pub(super) fn cmd_spec_doctor(json: bool) -> Result<()> {
    let project_root = std::env::current_dir()?;
    let spec_dir = project_root.join(".ctl").join("spec");
    if !spec_dir.exists() {
        if json {
            println!(
                "{}",
                serde_json::json!({"has_specs": false, "files_scanned": 0, "stale_count": 0, "stale": []})
            );
        } else {
            println!("No .ctl/spec/ directory — nothing to scan.");
        }
        return Ok(());
    }

    let mut findings: Vec<(std::path::PathBuf, usize, String)> = Vec::new();
    let mut files_scanned = 0u32;
    scan_spec_for_stale_refs(&spec_dir, &project_root, &mut findings, &mut files_scanned)?;

    if json {
        let stale: Vec<serde_json::Value> = findings
            .iter()
            .map(|(f, line, refer)| {
                serde_json::json!({
                    "spec_file": f.strip_prefix(&project_root).unwrap_or(f).to_string_lossy(),
                    "line": line,
                    "reference": refer,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "has_specs": true,
                "files_scanned": files_scanned,
                "stale_count": findings.len(),
                "stale": stale,
            })
        );
        return Ok(());
    }

    if findings.is_empty() {
        println!(
            "No stale references found (scanned {} spec file(s)).",
            files_scanned
        );
    } else {
        println!(
            "STALE references ({} across {} spec file(s)):",
            findings.len(),
            files_scanned
        );
        for (f, line, refer) in &findings {
            let rel = f.strip_prefix(&project_root).unwrap_or(f).display();
            println!("  {}:{}  `{}`  -> missing", rel, line, refer);
        }
    }
    Ok(())
}

fn scan_spec_for_stale_refs(
    dir: &Path,
    project_root: &Path,
    findings: &mut Vec<(std::path::PathBuf, usize, String)>,
    files_scanned: &mut u32,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.metadata()?.is_dir() {
            scan_spec_for_stale_refs(&path, project_root, findings, files_scanned)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            *files_scanned += 1;
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for (i, line) in content.lines().enumerate() {
                for tok in backtick_tokens(line) {
                    if is_candidate_path(tok) && !path_exists(project_root, tok) {
                        findings.push((path.clone(), i + 1, tok.to_string()));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Extract the text of each backtick-quoted span on `line` (the bytes between
/// paired `` ` ``). An unterminated quote yields nothing for the tail.
fn backtick_tokens(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        match after.find('`') {
            Some(end) => {
                let tok = &after[..end];
                if !tok.is_empty() {
                    out.push(tok);
                }
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// Heuristic: a backtick token worth existence-checking. Must contain a `/`
/// (paths), have a `.`-bearing final segment (a file, not a bare directory
/// name like `cli/` or `domain/`), have no spaces (excludes `cargo test`), not
/// contain `<>` placeholders (e.g. `<task>`), not be a URL/flag, and not live
/// under `.ctl/spec/` (spec self-references aren't rot).
fn is_candidate_path(tok: &str) -> bool {
    let last_seg = tok.rsplit('/').next().unwrap_or("");
    tok.contains('/')
        && !tok.contains(' ')
        && !tok.contains('<')
        && !tok.contains('>')
        && !tok.contains('*')
        && !tok.contains('?')
        && last_seg.contains('.')
        && !tok.starts_with("http")
        && !tok.starts_with('-')
        && !tok.starts_with(".ctl/spec/")
        && !tok.starts_with("git@")
}

/// Check whether `tok` (a candidate path reference) resolves under
/// `project_root`. A trailing `:<digits>` line anchor is stripped first so
/// `src/foo.rs:42` checks `src/foo.rs`. Module shorthand is tolerated: if the
/// literal path is missing, `src/<tok>` is also tried, so a spec writing
/// `domain/foo.rs` resolves against `src/domain/foo.rs`.
fn path_exists(project_root: &Path, tok: &str) -> bool {
    let path_part = match tok.rsplit_once(':') {
        Some((head, tail))
            if !tail.is_empty()
                && tail
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b == b'-' || b == b',') =>
        {
            head
        }
        _ => tok,
    };
    if project_root.join(path_part).exists() {
        return true;
    }
    // Module-shorthand fallback (specs often drop the src/ prefix).
    if !path_part.starts_with("src/") && project_root.join(format!("src/{}", path_part)).exists() {
        return true;
    }
    false
}

#[cfg(test)]
mod spec_doctor_tests {
    use super::*;

    #[test]
    fn backtick_tokens_extracts_pairs() {
        assert_eq!(
            backtick_tokens("see `src/a.rs` and `src/b.rs:10`"),
            vec!["src/a.rs", "src/b.rs:10"]
        );
        assert!(backtick_tokens("no backticks").is_empty());
        // unterminated tail yields nothing
        assert!(backtick_tokens("see `src/a.rs").is_empty());
    }

    #[test]
    fn candidate_path_heuristic() {
        assert!(is_candidate_path("src/foo.rs"));
        assert!(is_candidate_path("schemas/x.json"));
        assert!(
            is_candidate_path("domain/foo.rs"),
            "module shorthand is a candidate"
        );
        assert!(!is_candidate_path("cargo test"), "space -> skip");
        assert!(!is_candidate_path("Phase::InProgress"), "no slash -> skip");
        assert!(!is_candidate_path("--write-allow"), "flag -> skip");
        assert!(!is_candidate_path("https://x.com"), "url -> skip");
        assert!(
            !is_candidate_path(".ctl/spec/foo.md"),
            "spec self-ref -> skip"
        );
        assert!(!is_candidate_path("cli/"), "bare directory -> skip");
        assert!(!is_candidate_path("domain/"), "bare directory -> skip");
        assert!(
            !is_candidate_path(".ctl/tasks/<task>/events.jsonl"),
            "placeholder -> skip"
        );
        assert!(!is_candidate_path("fixtures/*.jsonl"), "glob -> skip");
    }

    #[test]
    fn path_exists_strips_anchor_and_falls_back_to_src() {
        // Uses the real project root, where src/cli/mod.rs and src/domain/task.rs exist.
        let root = std::env::current_dir().unwrap();
        assert!(path_exists(&root, "src/cli/mod.rs"));
        assert!(path_exists(&root, "src/cli/mod.rs:42"), "anchor stripped");
        assert!(
            path_exists(&root, "domain/task.rs"),
            "module shorthand resolves via src/ fallback"
        );
        assert!(
            path_exists(&root, "domain/run.rs:60-95"),
            "line range (N-M) stripped before lookup"
        );
        assert!(!path_exists(&root, "src/nope/missing.rs"));
    }

    #[test]
    fn scan_reports_only_missing_refs() {
        let tmp = std::env::temp_dir().join(format!("ctl-spec-doctor-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let spec_dir = tmp.join(".ctl").join("spec");
        std::fs::create_dir_all(&spec_dir).unwrap();
        std::fs::create_dir_all(tmp.join("src/cli")).unwrap();
        std::fs::write(tmp.join("src/cli/mod.rs"), "// stub").unwrap();
        std::fs::write(
            spec_dir.join("guide.md"),
            "# Guide\nReal: `src/cli/mod.rs`. Stale: `src/deleted.rs`.\n",
        )
        .unwrap();

        let mut findings = Vec::new();
        let mut scanned = 0u32;
        scan_spec_for_stale_refs(&spec_dir, &tmp, &mut findings, &mut scanned).unwrap();
        assert_eq!(scanned, 1);
        assert_eq!(findings.len(), 1, "only the stale ref is reported");
        assert_eq!(findings[0].2, "src/deleted.rs");
        assert_eq!(findings[0].1, 2, "stale ref is on line 2");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
