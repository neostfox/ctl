//! PRD plan / validate / status (workflow-prd-to-tasks-v1).
//!
//! The cognitive loop is grill(alignment note) → PRD(`ctl prd init` template)
//! → **tasks** → TDD → handoff. The PRD → tasks seam was the only mechanical
//! break: a confirmed PRD's `## Tasks` section could not become governed tasks
//! except by manual `ctl task create` per item. This module closes that seam.
//!
//! The decomposition intelligence already happened during PRD authoring (the
//! human confirmed it). `parse_prd` mechanically reads the rigid `## Tasks`
//! convention; `prd_validate` / `prd_plan` / `prd_status` (ControlApp methods)
//! orchestrate against the store. No model judgement at plan time, no new event
//! types — `prd plan` reuses existing `create_task` + `record_brainstorm_artifacts`.
//!
//! The parser is pure (string in, document out) and rigid by design: it reads
//! exactly the convention `ctl prd init` emits (pinned by a CLI test). It is not
//! a general markdown parser.

use std::collections::BTreeSet;

use crate::application::schedule::detect_write_scope_overlap;

// ── Document model ──────────────────────────────────────────────────────────

/// One parsed task item from a PRD's `## Tasks` section.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PrdTask {
    pub id: String,
    pub objective: String,
    /// Defaults to `write_allow` when absent in the source.
    pub read_scope: Vec<String>,
    pub write_allow: Vec<String>,
    pub gates: Vec<String>,
    /// May be empty. References task ids within this PRD or external (already
    /// existing) tasks; external refs are a warning, not an error.
    pub depends_on: Vec<String>,
}

/// A parsed PRD document — the confirmed plan a later `prd plan` executes.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PrdDocument {
    pub title: String,
    pub status: PrdStatus,
    pub tasks: Vec<PrdTask>,
}

/// The status carried in the PRD header. `Unknown` when no status line is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum PrdStatus {
    Draft,
    Confirmed,
    Superseded,
    Unknown,
}

impl PrdStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PrdStatus::Draft => "draft",
            PrdStatus::Confirmed => "confirmed",
            PrdStatus::Superseded => "superseded",
            PrdStatus::Unknown => "unknown",
        }
    }
}

// ── Parser (pure) ───────────────────────────────────────────────────────────

/// Parse a PRD document from its raw markdown text. Pure: no IO.
///
/// Reads the `# PRD: <title>` heading, a `> Status: <word>` line, and the rigid
/// `## Tasks` convention (`- id:` items with indented `key: value` fields).
/// Field keys are hyphen-or-underscore tolerant (`write-allow` == `write_allow`).
pub fn parse_prd(content: &str) -> anyhow::Result<PrdDocument> {
    let lines: Vec<&str> = content.lines().collect();
    let title = parse_title(&lines)?;
    let status = parse_status(&lines);
    let tasks = parse_tasks(&lines)?;
    Ok(PrdDocument {
        title,
        status,
        tasks,
    })
}

fn parse_title(lines: &[&str]) -> anyhow::Result<String> {
    for line in lines {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("# PRD:") {
            let title = rest.trim();
            if title.is_empty() {
                return Err(anyhow::anyhow!(
                    "PRD heading is empty: expected '# PRD: <title>'"
                ));
            }
            return Ok(title.to_string());
        }
    }
    Err(anyhow::anyhow!(
        "PRD heading not found: expected a '# PRD: <title>' line"
    ))
}

/// The first `> Status: <word>` line (case-insensitive). `Unknown` if absent.
fn parse_status(lines: &[&str]) -> PrdStatus {
    for line in lines {
        let t = line.trim();
        let Some(rest) = t
            .strip_prefix('>')
            .map(str::trim_start)
            .and_then(|r| r.strip_prefix("Status:"))
        else {
            continue;
        };
        let word = rest.trim().to_ascii_lowercase();
        return match word.as_str() {
            "draft" => PrdStatus::Draft,
            "confirmed" => PrdStatus::Confirmed,
            "superseded" => PrdStatus::Superseded,
            _ => PrdStatus::Unknown,
        };
    }
    PrdStatus::Unknown
}

/// Parse the `## Tasks` section into task items. Stops at the next `## ` heading
/// or EOF. HTML comments (`<!-- ... -->`, possibly multi-line) are skipped.
fn parse_tasks(lines: &[&str]) -> anyhow::Result<Vec<PrdTask>> {
    // Locate the `## Tasks` heading (exact, case-sensitively — the template is fixed).
    let start = lines
        .iter()
        .position(|l| l.trim() == "## Tasks")
        .ok_or_else(|| anyhow::anyhow!("PRD has no '## Tasks' section"))?;

    let mut tasks = Vec::new();
    let mut current: Option<PrdTask> = None;
    let mut in_comment = false;

    for line in &lines[start + 1..] {
        let trimmed = line.trim();

        // Track multi-line HTML comment blocks.
        if in_comment {
            if trimmed.contains("-->") {
                in_comment = false;
            }
            continue;
        }
        if let Some(idx) = trimmed.find("<!--") {
            if !trimmed[idx..].contains("-->") {
                in_comment = true;
            }
            // A line may have content before the comment; keep scanning — but the
            // convention never mixes code and comment on one line, so skip whole.
            continue;
        }

        // Next top-level heading ends the section.
        if trimmed.starts_with("## ") {
            break;
        }

        // Blank lines separate items but don't end the section.
        if trimmed.is_empty() {
            continue;
        }

        // `- id: <value>` starts a new task item.
        if let Some(id_part) = trimmed.strip_prefix("- id:") {
            if let Some(task) = current.take() {
                tasks.push(task);
            }
            let id = id_part.trim().to_string();
            current = Some(PrdTask {
                id,
                objective: String::new(),
                read_scope: Vec::new(),
                write_allow: Vec::new(),
                gates: Vec::new(),
                depends_on: Vec::new(),
            });
            continue;
        }

        // Indented `key: value` field of the current task.
        if line.starts_with(char::is_whitespace) {
            if let Some(task) = current.as_mut() {
                if let Some((key, value)) = split_field(trimmed) {
                    apply_field(task, key, value)?;
                }
                // Lines that don't match `key: value` inside an item are ignored —
                // the convention is rigid but we don't fail on stray indentation.
            }
            continue;
        }

        // Any other non-blank, non-heading, non-indented line ends task parsing.
        break;
    }

    if let Some(task) = current.take() {
        tasks.push(task);
    }

    Ok(tasks)
}

/// Split a `key: value` line. Returns `(normalized_key, raw_value)`.
/// Key is lowercased and hyphens become underscores (`write-allow` → `write_allow`).
fn split_field(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    let key = key.trim().to_ascii_lowercase().replace('-', "_");
    // Re-borrow the normalized key lifetime via a leak-free path: keys are a
    // small known set, so compare against statics instead of returning owned.
    // We return the raw trimmed key slice for a dispatch table in apply_field.
    let key_static = match key.as_str() {
        "objective" => "objective",
        "write_allow" => "write_allow",
        "read_scope" => "read_scope",
        "gates" => "gates",
        "depends_on" => "depends_on",
        _ => return None,
    };
    Some((key_static, value.trim()))
}

fn apply_field(task: &mut PrdTask, key: &str, value: &str) -> anyhow::Result<()> {
    match key {
        "objective" => task.objective = value.to_string(),
        "write_allow" => task.write_allow = parse_csv(value),
        "read_scope" => task.read_scope = parse_csv(value),
        "gates" => task.gates = parse_csv(value),
        "depends_on" => task.depends_on = parse_csv(value),
        _ => {}
    }
    Ok(())
}

/// Comma-separated values: split, trim, drop empties.
fn parse_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// ── Validation (pure format checks; boundary/overlap need IO — see ControlApp) ─

/// Severity of a validation problem. Errors block `prd plan`; warnings surface
/// but do not block (e.g. an external dependency not in this PRD).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ProblemSeverity {
    Error,
    Warning,
}

/// One validation problem, scoped to a task (or the document when `task_id` is None).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PrdProblem {
    pub task_id: Option<String>,
    pub severity: ProblemSeverity,
    pub message: String,
}

/// The outcome of validating a parsed PRD document.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PrdValidation {
    pub problems: Vec<PrdProblem>,
}

impl PrdValidation {
    pub fn errors(&self) -> Vec<&PrdProblem> {
        self.problems
            .iter()
            .filter(|p| p.severity == ProblemSeverity::Error)
            .collect()
    }

    pub fn warnings(&self) -> Vec<&PrdProblem> {
        self.problems
            .iter()
            .filter(|p| p.severity == ProblemSeverity::Warning)
            .collect()
    }

    /// True when there are zero errors.
    pub fn ok(&self) -> bool {
        self.errors().is_empty()
    }

    pub(crate) fn error(&mut self, task_id: Option<&str>, message: impl Into<String>) {
        self.problems.push(PrdProblem {
            task_id: task_id.map(str::to_string),
            severity: ProblemSeverity::Error,
            message: message.into(),
        });
    }

    pub(crate) fn warning(&mut self, task_id: Option<&str>, message: impl Into<String>) {
        self.problems.push(PrdProblem {
            task_id: task_id.map(str::to_string),
            severity: ProblemSeverity::Warning,
            message: message.into(),
        });
    }
}

/// Pure format validation (no IO): id shape, uniqueness, non-empty required
/// fields, and within-PRD dependency references. Boundary normalization,
/// protected-path warning, gate-template validity, and cross-task overlap
/// need the application layer (project root + normalizer) and are run there.
pub fn validate_format(doc: &PrdDocument) -> PrdValidation {
    let mut v = PrdValidation::default();

    if doc.tasks.is_empty() {
        v.error(None, "## Tasks section has no task items");
    }

    let mut seen_ids = BTreeSet::new();
    let valid_ids: BTreeSet<&str> = doc.tasks.iter().map(|t| t.id.as_str()).collect();

    for task in &doc.tasks {
        let tid = task.id.as_str();

        if !is_kebab(&task.id) {
            v.error(
                Some(tid),
                format!(
                    "id '{}' is not kebab-case (lowercase a-z, 0-9, hyphens)",
                    task.id
                ),
            );
        }
        if !seen_ids.insert(tid) {
            v.error(None, format!("duplicate task id '{}'", tid));
        }
        if task.objective.trim().is_empty() {
            v.error(Some(tid), "objective is empty");
        }
        if task.write_allow.is_empty() {
            v.error(
                Some(tid),
                "write-allow is empty (at least one path required)",
            );
        }
        if task.gates.is_empty() {
            v.error(
                Some(tid),
                "gates is empty (at least one gate template required)",
            );
        }

        // Within-PRD dependency refs must resolve; external refs are a warning.
        for dep in &task.depends_on {
            if !valid_ids.contains(dep.as_str()) {
                v.warning(
                    Some(tid),
                    format!(
                        "depends-on '{}' is not a task in this PRD — assumed already satisfied",
                        dep
                    ),
                );
            }
        }
    }

    v
}

/// kebab-case: lowercase ascii alphanumeric plus hyphens, non-empty, no leading/
/// trailing hyphen, no double hyphen.
fn is_kebab(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut prev_hyphen = true; // rejects a leading hyphen
    for ch in s.chars() {
        let is_hyphen = ch == '-';
        if is_hyphen && prev_hyphen {
            return false;
        }
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '-' {
            return false;
        }
        prev_hyphen = is_hyphen;
    }
    !prev_hyphen // rejects a trailing hyphen
}

/// Pairwise cross-task write-allow overlap, returning one problem per colliding
/// pair. Operates on the already-parsed (pre-normalization) sets — `prd_validate`
/// on ControlApp normalizes first, then calls this.
pub fn overlap_problems(doc: &PrdDocument) -> Vec<(String, String, Vec<String>)> {
    let mut out = Vec::new();
    for i in 0..doc.tasks.len() {
        for j in (i + 1)..doc.tasks.len() {
            let a: BTreeSet<String> = doc.tasks[i].write_allow.iter().cloned().collect();
            let b: BTreeSet<String> = doc.tasks[j].write_allow.iter().cloned().collect();
            let overlap = detect_write_scope_overlap(&a, &b);
            if !overlap.is_empty() {
                out.push((doc.tasks[i].id.clone(), doc.tasks[j].id.clone(), overlap));
            }
        }
    }
    out
}

// ── Plan / status outcome types ─────────────────────────────────────────────

/// The result of planning one task from a PRD. In a dry run, `created` is false
/// and `seq` is None (nothing persisted); in a real plan both are set.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PrdPlanOutcome {
    pub task_id: String,
    pub objective: String,
    pub write_allow: Vec<String>,
    pub gates: Vec<String>,
    pub depends_on: Vec<String>,
    pub created: bool,
    pub seq: Option<i64>,
    /// True when brainstorm provenance (divergence + convergence) was recorded.
    pub provenance_recorded: bool,
}

/// One row of the observable-loop status view.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PrdTaskStatusRow {
    pub id: String,
    /// False when the task has not been created yet (only planned in the PRD).
    pub exists: bool,
    pub phase: Option<String>,
    pub provenance: Option<crate::domain::task::BrainstormProvenanceView>,
}

/// The full PRD status view: lineage + progress at a glance.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PrdStatusView {
    pub title: String,
    pub status: PrdStatus,
    pub total: usize,
    pub completed: usize,
    pub rows: Vec<PrdTaskStatusRow>,
}

/// Derive a stable brainstorm id from a PRD title: `BS-PRD-<slug>`.
pub fn brainstorm_id_for(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    let slug: String = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("BS-PRD-{}", slug)
}
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# PRD: Demo feature

> Status: confirmed
> Fill this out (the grill step), then `ctl prd plan` can turn the ## Tasks into ctl tasks.

## Objective

Ship the demo.

## Context

Some context.

## Tasks

<!-- One item per vertical task. -->
- id: auth-layer
  objective: add auth boundary
  write-allow: src/auth
  gates: cargo_check, cargo_test
  depends-on: config-task

- id: config-task
  objective: parse config
  write-allow: src/config.rs
  read-scope: src
  gates: cargo_fmt_check, cargo_check
";

    #[test]
    fn parses_title_status_and_two_tasks() {
        let doc = parse_prd(SAMPLE).unwrap();
        assert_eq!(doc.title, "Demo feature");
        assert_eq!(doc.status, PrdStatus::Confirmed);
        assert_eq!(doc.tasks.len(), 2);

        let auth = &doc.tasks[0];
        assert_eq!(auth.id, "auth-layer");
        assert_eq!(auth.objective, "add auth boundary");
        assert_eq!(auth.write_allow, vec!["src/auth"]);
        assert_eq!(auth.gates, vec!["cargo_check", "cargo_test"]);
        // read-scope absent → stays empty (ControlApp defaults it to write_allow).
        assert!(auth.read_scope.is_empty());
        assert_eq!(auth.depends_on, vec!["config-task"]);

        let cfg = &doc.tasks[1];
        assert_eq!(cfg.read_scope, vec!["src"]);
    }

    #[test]
    fn status_defaults_to_unknown_when_absent() {
        let no_status = SAMPLE.replace("> Status: confirmed\n", "");
        let doc = parse_prd(&no_status).unwrap();
        assert_eq!(doc.status, PrdStatus::Unknown);
    }

    #[test]
    fn draft_and_superseded_parse() {
        for (word, expect) in [
            ("draft", PrdStatus::Draft),
            ("confirmed", PrdStatus::Confirmed),
            ("superseded", PrdStatus::Superseded),
        ] {
            let prd = SAMPLE.replace("confirmed", word);
            assert_eq!(parse_prd(&prd).unwrap().status, expect);
        }
    }

    #[test]
    fn missing_tasks_section_is_an_error() {
        let prd = "# PRD: X\n\n> Status: confirmed\n\n## Objective\n\nDo.\n";
        let err = parse_prd(prd).unwrap_err();
        assert!(err.to_string().contains("## Tasks"), "{}", err);
    }

    #[test]
    fn missing_title_is_an_error() {
        let prd = "> Status: confirmed\n\n## Tasks\n\n- id: a\n  objective: x\n  write-allow: s\n  gates: cargo_check\n";
        assert!(parse_prd(prd).is_err());
    }

    #[test]
    fn html_comment_block_is_skipped() {
        let prd = "# PRD: X\n\n> Status: confirmed\n\n## Tasks\n\n<!-- a comment\nspanning two lines -->\n- id: a\n  objective: x\n  write-allow: s\n  gates: cargo_check\n";
        let doc = parse_prd(prd).unwrap();
        assert_eq!(doc.tasks.len(), 1);
        assert_eq!(doc.tasks[0].id, "a");
    }

    #[test]
    fn underscore_keys_work_too() {
        let prd = "# PRD: X\n\n> Status: confirmed\n\n## Tasks\n\n- id: a\n  objective: x\n  write_allow: s\n  gates: cargo_check\n";
        let doc = parse_prd(prd).unwrap();
        assert_eq!(doc.tasks[0].write_allow, vec!["s"]);
    }

    #[test]
    fn format_validation_catches_empties_dup_and_bad_id() {
        let prd = "# PRD: X\n\n> Status: confirmed\n\n## Tasks\n\n\
            - id: Bad_ID\n  objective: ok\n  write-allow: s\n  gates: cargo_check\n\
            - id: a\n  write-allow: s\n  gates: cargo_check\n\
            - id: a\n  objective: dup\n  write-allow: s\n  gates: cargo_check\n";
        let doc = parse_prd(prd).unwrap();
        let v = validate_format(&doc);
        assert!(!v.ok(), "should have errors");
        let msgs: Vec<&str> = v.problems.iter().map(|p| p.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("kebab")), "{:?}", msgs);
        assert!(
            msgs.iter().any(|m| m.contains("objective is empty")),
            "{:?}",
            msgs
        );
        assert!(msgs.iter().any(|m| m.contains("duplicate")), "{:?}", msgs);
    }

    #[test]
    fn external_dependency_is_a_warning_not_an_error() {
        let prd = "# PRD: X\n\n> Status: confirmed\n\n## Tasks\n\n\
            - id: a\n  objective: x\n  write-allow: s\n  gates: cargo_check\n  depends-on: external-task\n";
        let doc = parse_prd(prd).unwrap();
        let v = validate_format(&doc);
        assert!(v.ok(), "external dep must not be an error");
        assert_eq!(v.warnings().len(), 1);
    }

    #[test]
    fn overlap_detection_flags_overlapping_writes() {
        let doc = PrdDocument {
            title: "X".into(),
            status: PrdStatus::Confirmed,
            tasks: vec![
                PrdTask {
                    id: "a".into(),
                    objective: "o".into(),
                    read_scope: vec![],
                    write_allow: vec!["src".into()],
                    gates: vec!["cargo_check".into()],
                    depends_on: vec![],
                },
                PrdTask {
                    id: "b".into(),
                    objective: "o".into(),
                    read_scope: vec![],
                    write_allow: vec!["src/auth".into()],
                    gates: vec!["cargo_check".into()],
                    depends_on: vec![],
                },
            ],
        };
        let overlaps = overlap_problems(&doc);
        assert_eq!(overlaps.len(), 1, "src and src/auth overlap");
        assert_eq!(overlaps[0].0, "a");
        assert_eq!(overlaps[0].1, "b");
    }

    #[test]
    fn disjoint_writes_have_no_overlap() {
        let doc = PrdDocument {
            title: "X".into(),
            status: PrdStatus::Confirmed,
            tasks: vec![
                PrdTask {
                    id: "a".into(),
                    objective: "o".into(),
                    read_scope: vec![],
                    write_allow: vec!["src/auth".into()],
                    gates: vec!["cargo_check".into()],
                    depends_on: vec![],
                },
                PrdTask {
                    id: "b".into(),
                    objective: "o".into(),
                    read_scope: vec![],
                    write_allow: vec!["src/config".into()],
                    gates: vec!["cargo_check".into()],
                    depends_on: vec![],
                },
            ],
        };
        assert!(overlap_problems(&doc).is_empty());
    }
}
