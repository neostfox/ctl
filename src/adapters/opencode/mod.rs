use crate::adapters::{ExecutorAdapter, RunManifest};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::Path;

/// opencode executor adapter.
/// Generates run manifests for opencode plugin consumption and validates agent-output.json.
/// Mirrors the OMP adapter contract; the only material difference is the evidence `source`
/// tag ("opencode"), which keeps the cross-adapter audit trail unambiguous.
pub struct OpenCodeAdapter;

impl ExecutorAdapter for OpenCodeAdapter {
    fn adapter_name(&self) -> &str {
        "opencode"
    }

    fn capabilities(&self) -> Value {
        serde_json::json!({
            "adapter": "opencode",
            "capabilities": [
                "file_read",
                "file_write",
                "search",
                "edit",
                "bash",
                "lsp",
                "task",
            ],
            "workspace": "disposable_worktree",
            "output_format": "agent-output.json",
        })
    }

    fn prepare_run(
        &self,
        task_id: &str,
        run_id: &str,
        lease_id: &str,
        worktree: &Path,
        write_allow: &[String],
        write_deny: &[String],
        gates: &[String],
    ) -> Result<RunManifest> {
        let now = crate::application::now_iso8601();
        Ok(RunManifest {
            schema: "control.run-manifest.v1".to_string(),
            run_id: run_id.to_string(),
            task_id: task_id.to_string(),
            adapter: "opencode".to_string(),
            assignment_path: format!(".ctl/tasks/{}/assignment.json", task_id),
            worktree_path: worktree.to_string_lossy().to_string(),
            lease_id: lease_id.to_string(),
            write_allow: write_allow.to_vec(),
            write_deny: write_deny.to_vec(),
            gates: gates.to_vec(),
            created_at: now,
        })
    }

    fn validate_output(&self, output: &Value) -> Result<()> {
        // opencode adapter expects agent-output.json with source="opencode"
        let source = output.get("source").and_then(|v| v.as_str()).unwrap_or("");
        if source != "opencode" {
            return Err(anyhow!(
                "opencode adapter output must have source=\"opencode\", got \"{}\"",
                source
            ));
        }

        // Validate touched_files is present and is an array
        if output
            .get("touched_files")
            .and_then(|v| v.as_array())
            .is_none()
        {
            return Err(anyhow!(
                "opencode adapter output must contain touched_files array"
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_name_is_opencode() {
        assert_eq!(OpenCodeAdapter.adapter_name(), "opencode");
    }

    #[test]
    fn capabilities_report_opencode() {
        let caps = OpenCodeAdapter.capabilities();
        assert_eq!(caps["adapter"], "opencode");
        assert_eq!(caps["output_format"], "agent-output.json");
    }

    #[test]
    fn validate_output_requires_opencode_source() {
        // Wrong source is rejected (e.g. an OMP result fed to the opencode adapter).
        let omp_shaped = serde_json::json!({ "source": "omp", "touched_files": [] });
        assert!(OpenCodeAdapter.validate_output(&omp_shaped).is_err());

        // Correct source + touched_files passes.
        let ok = serde_json::json!({ "source": "opencode", "touched_files": ["src/main.rs"] });
        assert!(OpenCodeAdapter.validate_output(&ok).is_ok());
    }

    #[test]
    fn validate_output_requires_touched_files_array() {
        let missing = serde_json::json!({ "source": "opencode" });
        assert!(OpenCodeAdapter.validate_output(&missing).is_err());
    }
}
