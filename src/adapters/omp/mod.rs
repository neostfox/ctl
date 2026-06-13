use crate::adapters::{ExecutorAdapter, RunManifest};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::Path;

/// OMP executor adapter.
/// Generates run manifests for OMP skill consumption and validates agent-output.json.
pub struct OmpAdapter;

impl ExecutorAdapter for OmpAdapter {
    fn adapter_name(&self) -> &str {
        "omp"
    }

    fn capabilities(&self) -> Value {
        serde_json::json!({
            "adapter": "omp",
            "capabilities": [
                "file_read",
                "file_write",
                "search",
                "edit",
                "lsp",
                "debug",
                "browser",
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
            adapter: "omp".to_string(),
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
        // OMP adapter expects agent-output.json with source="omp"
        let source = output.get("source").and_then(|v| v.as_str()).unwrap_or("");
        if source != "omp" {
            return Err(anyhow!(
                "OMP adapter output must have source=\"omp\", got \"{}\"",
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
                "OMP adapter output must contain touched_files array"
            ));
        }

        Ok(())
    }
}
