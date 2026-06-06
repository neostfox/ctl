use anyhow::{anyhow, Result};

/// A gate template defines a fixed command to run.
/// No arbitrary shell — only predefined templates are allowed.
#[derive(Debug, Clone)]
pub struct GateTemplate {
    pub id: &'static str,
    pub description: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
}

/// All available gate templates. This is the authoritative list.
pub static GATE_TEMPLATES: &[GateTemplate] = &[
    GateTemplate {
        id: "cargo_fmt_check",
        description: "Check code formatting with cargo fmt",
        command: "cargo",
        args: &["fmt", "--check"],
    },
    GateTemplate {
        id: "cargo_check",
        description: "Check compilation with cargo check",
        command: "cargo",
        args: &["check"],
    },
    GateTemplate {
        id: "cargo_test",
        description: "Run tests with cargo test",
        command: "cargo",
        args: &["test"],
    },
    GateTemplate {
        id: "cargo_clippy",
        description: "Run clippy lint checks",
        command: "cargo",
        args: &["clippy", "--", "-D", "warnings"],
    },
];

/// Result of running a gate.
#[derive(Debug)]
#[allow(dead_code)]
pub struct GateRunResult {
    pub gate_id: String,
    pub passed: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Find a gate template by ID.
pub fn find_template(id: &str) -> Option<&'static GateTemplate> {
    GATE_TEMPLATES.iter().find(|t| t.id == id)
}

/// M0 freezes gate templates only; it must not execute shell commands.
pub fn run_gate(gate_id: &str, _working_dir: &std::path::Path) -> Result<GateRunResult> {
    let _template =
        find_template(gate_id).ok_or_else(|| anyhow!("Unknown gate template: {}", gate_id))?;
    Err(anyhow!(
        "Gate execution is disabled in M0 until EXEC-002 runner policy is implemented"
    ))
}

/// List all available gate templates.
pub fn list_templates() -> Vec<&'static GateTemplate> {
    GATE_TEMPLATES.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_template_known() {
        assert!(find_template("cargo_fmt_check").is_some());
        assert!(find_template("cargo_check").is_some());
        assert!(find_template("cargo_test").is_some());
        assert!(find_template("cargo_clippy").is_some());
    }

    #[test]
    fn test_find_template_unknown() {
        assert!(find_template("nonexistent_gate").is_none());
    }

    #[test]
    fn test_list_templates_count() {
        assert_eq!(list_templates().len(), 4);
    }

    #[test]
    fn test_run_gate_unknown() {
        let dir = std::env::current_dir().unwrap();
        assert!(run_gate("nonexistent_gate", &dir).is_err());
    }
}
