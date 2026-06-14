use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

// Git worktree workspace management for isolated execution.
/// Create a git worktree for a task (M4 single-executor path).
/// Returns the worktree path.
pub fn create_worktree(project_root: &Path, task_id: &str) -> Result<PathBuf> {
    let worktree_path = project_root
        .join(".ctl")
        .join("tasks")
        .join(task_id)
        .join("worktree");
    create_worktree_at(
        project_root,
        &worktree_path,
        &format!("omp-run-{}", task_id),
    )?;
    Ok(worktree_path)
}

/// Path where a run's isolated worktree lives: `.ctl/runs/<run_id>/worktree`.
/// (M6 worktree-per-agent — keyed by run_id so concurrent runs never collide.)
pub fn run_worktree_path(project_root: &Path, run_id: &str) -> PathBuf {
    project_root
        .join(".ctl")
        .join("runs")
        .join(run_id)
        .join("worktree")
}

/// Create a git worktree for an agent run (M6 worktree-per-agent). Each
/// concurrent run gets its own worktree + branch, so writers never share a
/// working tree even when running in parallel.
pub fn create_run_worktree(project_root: &Path, run_id: &str) -> Result<PathBuf> {
    let worktree_path = run_worktree_path(project_root, run_id);
    create_worktree_at(project_root, &worktree_path, &format!("omp-run-{}", run_id))?;
    Ok(worktree_path)
}

/// Shared worktree creation: make `worktree_path` a fresh `git worktree` on a
/// new `branch_name` off HEAD. Cleans up the directory if `git worktree add`
/// fails so a partial attempt never blocks a retry.
fn create_worktree_at(project_root: &Path, worktree_path: &Path, branch_name: &str) -> Result<()> {
    if worktree_path.exists() {
        return Err(anyhow!(
            "Worktree already exists at {}",
            worktree_path.display()
        ));
    }

    // Create worktree directory
    std::fs::create_dir_all(worktree_path)?;

    // Initialize as a git worktree via `git worktree add`
    let output = std::process::Command::new("git")
        .args([
            "worktree",
            "add",
            &worktree_path.to_string_lossy(),
            "-b",
            branch_name,
            "HEAD",
        ])
        .current_dir(project_root)
        .output()?;

    if !output.status.success() {
        // Cleanup on failure
        let _ = std::fs::remove_dir_all(worktree_path);
        return Err(anyhow!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
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

/// Scoped working-tree cleanliness check (M-g commit interlock).
///
/// Runs `git status --porcelain` limited to `scope` pathspecs and returns the
/// dirty paths (tracked-modified, staged, or untracked) within that scope.
/// `.ctl/` is gitignored, so runtime ledger churn is excluded automatically;
/// scoping additionally narrows the check to the task's `write_allow`.
///
/// Returns `Ok(None)` when the project is not a git repository or `git` is
/// unavailable — the caller decides whether an unverifiable tree is fatal.
/// `Ok(Some(vec))` with an empty vec means the scope is clean.
pub fn dirty_paths_in_scope(project_root: &Path, scope: &[String]) -> Result<Option<Vec<String>>> {
    let mut args: Vec<String> = vec!["status".into(), "--porcelain".into()];
    if !scope.is_empty() {
        args.push("--".into());
        args.extend(scope.iter().cloned());
    }

    let output = match std::process::Command::new("git")
        .args(&args)
        .current_dir(project_root)
        .output()
    {
        Ok(o) => o,
        Err(_) => return Ok(None), // git binary unavailable — unverifiable
    };

    if !output.status.success() {
        // Typically exit 128 = not a git repository. Treat as unverifiable
        // rather than fabricating a clean/dirty verdict.
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut dirty = Vec::new();
    for line in stdout.lines() {
        // Porcelain v1 lines are "XY <path>" (renames: "XY <old> -> <new>").
        // Skip the two status chars + separating space to recover the path(s).
        let path = line.get(3..).unwrap_or("").trim();
        if !path.is_empty() {
            dirty.push(path.to_string());
        }
    }
    Ok(Some(dirty))
}

/// Git tree hash of the current committed `HEAD` (`HEAD^{tree}`).
///
/// This is the canonical artifact identity used to bind gate and completion-audit
/// evidence to a specific code state (artifact binding). Only the committed tree
/// is bound — not the index or working tree — matching the M-g commit interlock,
/// so the binding is stable across the gate→audit→finish window once work is
/// committed.
///
/// Returns `Ok(None)` when the project is not a git repository, `git` is
/// unavailable, or `HEAD` has no commit yet — the caller decides whether an
/// unverifiable tree is fatal (finish skips the binding interlock, mirroring M-g).
pub fn head_tree_hash(project_root: &Path) -> Result<Option<String>> {
    let output = match std::process::Command::new("git")
        .args(["rev-parse", "HEAD^{tree}"])
        .current_dir(project_root)
        .output()
    {
        Ok(o) => o,
        Err(_) => return Ok(None), // git binary unavailable — unverifiable
    };
    if !output.status.success() {
        // Not a git repo (exit 128) or no commit yet — unverifiable, not fatal here.
        return Ok(None);
    }
    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if hash.is_empty() {
        return Ok(None);
    }
    Ok(Some(hash))
}

/// Detect high-risk changes from a diff.
/// Returns a list of (risk_type, file_path) pairs.
pub fn detect_high_risk(files: &[(String, String)]) -> Vec<(String, String)> {
    let protected_prefixes = [".omp/", ".ctl/spec/", "schemas/"];
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TmpDir {
        path: PathBuf,
    }
    impl TmpDir {
        fn new(tag: &str) -> Self {
            // Avoid Date/rand (banned in some contexts); a process+counter tag
            // is unique enough for serial test runs.
            let path =
                std::env::temp_dir().join(format!("ctl-wt-test-{}-{}", std::process::id(), tag));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("src")).unwrap();
            Self { path }
        }
        fn path(&self) -> &Path {
            &self.path
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git runs")
            .status
            .success();
        assert!(ok, "git {:?} failed", args);
    }

    fn git_init(dir: &Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "t@t"]);
        git(dir, &["config", "user.name", "t"]);
    }

    #[test]
    fn dirty_paths_none_outside_git_repo() {
        let d = TmpDir::new("nogit");
        let res = dirty_paths_in_scope(d.path(), &["src".to_string()]).unwrap();
        assert!(res.is_none(), "non-git dir is unverifiable → None");
    }

    #[test]
    fn dirty_paths_clean_committed_tree() {
        let d = TmpDir::new("clean");
        git_init(d.path());
        std::fs::write(d.path.join("src/lib.rs").as_path(), "fn a() {}\n").unwrap();
        git(d.path(), &["add", "-A"]);
        git(d.path(), &["commit", "-qm", "init"]);
        let res = dirty_paths_in_scope(d.path(), &["src".to_string()])
            .unwrap()
            .unwrap();
        assert!(res.is_empty(), "committed tree is clean: {:?}", res);
    }

    #[test]
    fn dirty_paths_detects_untracked_in_scope() {
        let d = TmpDir::new("untracked");
        git_init(d.path());
        // Commit a file first so src/ is tracked; otherwise git collapses the
        // whole untracked directory to "src/" in porcelain output.
        std::fs::write(d.path.join("src/lib.rs").as_path(), "fn a() {}\n").unwrap();
        git(d.path(), &["add", "-A"]);
        git(d.path(), &["commit", "-qm", "init"]);
        std::fs::write(d.path.join("src/new.rs").as_path(), "x\n").unwrap();
        let res = dirty_paths_in_scope(d.path(), &["src".to_string()])
            .unwrap()
            .unwrap();
        assert_eq!(res, vec!["src/new.rs".to_string()]);
    }

    #[test]
    fn dirty_paths_ignores_changes_outside_scope() {
        let d = TmpDir::new("outscope");
        git_init(d.path());
        std::fs::create_dir_all(d.path.join("docs")).unwrap();
        std::fs::write(d.path.join("docs/x.md").as_path(), "x\n").unwrap();
        // Dirty file lives in docs/, but we only scope src/.
        let res = dirty_paths_in_scope(d.path(), &["src".to_string()])
            .unwrap()
            .unwrap();
        assert!(
            res.is_empty(),
            "out-of-scope dirt must not count: {:?}",
            res
        );
    }
}
