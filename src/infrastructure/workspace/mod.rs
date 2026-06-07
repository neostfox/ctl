use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

// Git worktree workspace management for isolated execution.
/// Create a git worktree for a task.
/// Returns the worktree path.
pub fn create_worktree(project_root: &Path, task_id: &str) -> Result<PathBuf> {
    let worktree_path = project_root
        .join(".trellis")
        .join("tasks")
        .join(task_id)
        .join("worktree");

    if worktree_path.exists() {
        return Err(anyhow!(
            "Worktree already exists at {}",
            worktree_path.display()
        ));
    }

    // Create worktree directory
    std::fs::create_dir_all(&worktree_path)?;

    // Initialize as a git worktree via `git worktree add`
    let branch_name = format!("omp-run-{}", task_id);
    let output = std::process::Command::new("git")
        .args([
            "worktree",
            "add",
            &worktree_path.to_string_lossy(),
            "-b",
            &branch_name,
            "HEAD",
        ])
        .current_dir(project_root)
        .output()?;

    if !output.status.success() {
        // Cleanup on failure
        let _ = std::fs::remove_dir_all(&worktree_path);
        return Err(anyhow!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(worktree_path)
}

/// Compute the diff between the worktree and HEAD.
/// Returns a list of (status, path) tuples.
pub fn diff_worktree(_project_root: &Path, worktree_path: &Path) -> Result<Vec<(String, String)>> {
    let output = std::process::Command::new("git")
        .args(["diff", "--name-status", "HEAD"])
        .current_dir(worktree_path)
        .output()?;

    if !output.status.success() {
        return Err(anyhow!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut result = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Format: "STATUS\tpath" or "STATUS\told_path\tnew_path" for renames
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() >= 2 {
            result.push((parts[0].to_string(), parts[1].to_string()));
        }
    }
    Ok(result)
}

/// Apply files from worktree to main workspace.
/// Copies specified files from the worktree to the project root.
pub fn apply_files(project_root: &Path, worktree_path: &Path, files: &[String]) -> Result<()> {
    for file in files {
        let src = worktree_path.join(file);
        let dst = project_root.join(file);

        if !src.exists() {
            return Err(anyhow!("File not found in worktree: {}", file));
        }

        // Create parent directory if needed
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::copy(&src, &dst)?;
    }
    Ok(())
}

/// Remove a git worktree.
pub fn cleanup_worktree(project_root: &Path, worktree_path: &Path) -> Result<()> {
    let output = std::process::Command::new("git")
        .args([
            "worktree",
            "remove",
            &worktree_path.to_string_lossy(),
            "--force",
        ])
        .current_dir(project_root)
        .output()?;

    if !output.status.success() {
        // Force cleanup: just remove the directory
        let _ = std::fs::remove_dir_all(worktree_path);
    }

    Ok(())
}

/// Detect high-risk changes from a diff.
/// Returns a list of (risk_type, file_path) pairs.
pub fn detect_high_risk(files: &[(String, String)]) -> Vec<(String, String)> {
    let protected_prefixes = [".omp/", ".trellis/spec/", "schemas/"];
    let protected_exact = ["Cargo.toml", "Cargo.lock"];

    let mut risks = Vec::new();
    for (status, path) in files {
        // File deletion
        if status.starts_with('D') {
            risks.push(("file_deleted".to_string(), path.clone()));
            continue;
        }

        // Protected paths
        for prefix in &protected_prefixes {
            if path.starts_with(prefix) {
                risks.push(("protected_path_change".to_string(), path.clone()));
                break;
            }
        }

        // Dependency changes
        for exact in &protected_exact {
            if path == exact {
                risks.push(("dependency_change".to_string(), path.clone()));
            }
        }
    }
    risks
}
