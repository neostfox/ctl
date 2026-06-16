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

/// One change between a worktree and HEAD, with its kind made explicit so the
/// apply path can create/modify/delete/rename instead of copy-only. Mirrors the
/// status letters `git diff --name-status` emits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A file that exists in the worktree but not in HEAD (incl. untracked).
    Add(String),
    /// A file present in both, with differing content.
    Modify(String),
    /// A file present in HEAD but removed in the worktree.
    Delete(String),
    /// A file moved from `from` to `to`. Applying it removes `from` and writes
    /// `to`, so both paths must be in write scope.
    Rename { from: String, to: String },
}

impl Change {
    /// Single-letter status tag, mirroring `git diff --name-status`.
    pub fn status(&self) -> &'static str {
        match self {
            Change::Add(_) => "A",
            Change::Modify(_) => "M",
            Change::Delete(_) => "D",
            Change::Rename { .. } => "R",
        }
    }

    /// Every destination path this change touches in the main workspace. A
    /// rename touches two — the old path (removed) and the new path (created) —
    /// so scope and conflict checks must consider both.
    pub fn paths(&self) -> Vec<&str> {
        match self {
            Change::Add(p) | Change::Modify(p) | Change::Delete(p) => vec![p.as_str()],
            Change::Rename { from, to } => vec![from.as_str(), to.as_str()],
        }
    }
}

/// The set of changes between a worktree and HEAD.
pub type ChangeSet = Vec<Change>;

/// Compute the typed changeset between the worktree and HEAD.
///
/// Tracked changes (modify/delete/rename and staged adds) come from
/// `git diff --name-status` with rename detection (`-M`); brand-new files the
/// agent never staged are invisible to `git diff`, so untracked files are
/// captured separately via `git ls-files --others` and folded in as adds.
/// `-z` (NUL-delimited) output is used throughout so the parser is correct for
/// renames — the old tab-`splitn` collapsed `Rxxx\told\tnew` into a single
/// `old\tnew` path — and robust to paths containing whitespace.
pub fn diff_worktree(_project_root: &Path, worktree_path: &Path) -> Result<ChangeSet> {
    let mut changes = tracked_changes(worktree_path)?;
    for path in untracked_files(worktree_path)? {
        changes.push(Change::Add(path));
    }
    Ok(changes)
}

/// Tracked changes vs HEAD, parsed from `git diff --name-status -z -M`.
fn tracked_changes(worktree_path: &Path) -> Result<Vec<Change>> {
    let output = std::process::Command::new("git")
        .args(["diff", "--name-status", "-z", "-M", "HEAD"])
        .current_dir(worktree_path)
        .output()?;

    if !output.status.success() {
        return Err(anyhow!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // With `-z`, both the field separator and record terminator are NUL, so a
    // record is `<status>\0<path>\0` — or `<status>\0<old>\0<new>\0` for
    // renames/copies. Walk the tokens, consuming one or two paths per status.
    let mut tokens = stdout.split('\0').filter(|t| !t.is_empty());
    let mut changes = Vec::new();
    while let Some(status) = tokens.next() {
        let kind = status.chars().next().unwrap_or('?');
        let mut next_path = || {
            tokens
                .next()
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("git diff: status '{}' missing path", status))
        };
        match kind {
            'A' => changes.push(Change::Add(next_path()?)),
            'D' => changes.push(Change::Delete(next_path()?)),
            'R' => {
                let from = next_path()?;
                let to = next_path()?;
                changes.push(Change::Rename { from, to });
            }
            // Copy (only with `-C`, which we don't pass): the source survives,
            // so it's an add of the destination, not a rename.
            'C' => {
                let _from = next_path()?;
                let to = next_path()?;
                changes.push(Change::Add(to));
            }
            // M (modified), T (type change), and anything else: treat as a
            // content modification of the single path that follows.
            _ => changes.push(Change::Modify(next_path()?)),
        }
    }
    Ok(changes)
}

/// Untracked, non-ignored files in the worktree (`git ls-files --others
/// --exclude-standard`). These are new files `git diff` never reports.
fn untracked_files(worktree_path: &Path) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .current_dir(worktree_path)
        .output()?;

    if !output.status.success() {
        return Err(anyhow!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .split('\0')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect())
}

/// Apply a changeset from the worktree onto the main workspace.
///
/// Unlike the old copy-only path, this honours each change's kind: adds and
/// modifications copy the worktree file over (creating parent dirs), deletes
/// remove the file from the main workspace, and renames remove the old path
/// before writing the new one. A delete whose target is already gone is
/// treated as success (idempotent); a missing source for an add/modify/rename
/// is an error.
pub fn apply_changes(project_root: &Path, worktree_path: &Path, changes: &[Change]) -> Result<()> {
    for change in changes {
        match change {
            Change::Add(path) | Change::Modify(path) => {
                copy_into_workspace(worktree_path, project_root, path)?;
            }
            Change::Delete(path) => {
                remove_from_workspace(project_root, path)?;
            }
            Change::Rename { from, to } => {
                remove_from_workspace(project_root, from)?;
                copy_into_workspace(worktree_path, project_root, to)?;
            }
        }
    }
    Ok(())
}

/// Copy a worktree file onto the main workspace, creating parent dirs.
fn copy_into_workspace(worktree_path: &Path, project_root: &Path, rel: &str) -> Result<()> {
    let src = worktree_path.join(rel);
    let dst = project_root.join(rel);
    if !src.exists() {
        return Err(anyhow!("File not found in worktree: {}", rel));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&src, &dst)?;
    Ok(())
}

/// Remove a file from the main workspace. Already-absent is success.
fn remove_from_workspace(project_root: &Path, rel: &str) -> Result<()> {
    let dst = project_root.join(rel);
    match std::fs::remove_file(&dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow!("failed to remove '{}': {}", rel, e)),
    }
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

/// Detect high-risk changes from a changeset.
/// Returns a list of (risk_type, file_path) pairs.
pub fn detect_high_risk(changes: &[Change]) -> Vec<(String, String)> {
    let mut risks = Vec::new();
    for change in changes {
        match change {
            // A deletion is high-risk regardless of path.
            Change::Delete(path) => {
                risks.push(("file_deleted".to_string(), path.clone()));
            }
            // A rename removes the old path (file_deleted) and creates the new
            // one (which may itself land on a protected/dependency path).
            Change::Rename { from, to } => {
                risks.push(("file_deleted".to_string(), from.clone()));
                protected_risks(to, &mut risks);
            }
            Change::Add(path) | Change::Modify(path) => {
                protected_risks(path, &mut risks);
            }
        }
    }
    risks
}

/// Push protected-path and dependency risks for a single written path.
fn protected_risks(path: &str, risks: &mut Vec<(String, String)>) {
    let protected_prefixes = [".omp/", ".ctl/spec/", "schemas/"];
    let protected_exact = ["Cargo.toml", "Cargo.lock"];

    for prefix in &protected_prefixes {
        if path.starts_with(prefix) {
            risks.push(("protected_path_change".to_string(), path.to_string()));
            break;
        }
    }
    for exact in &protected_exact {
        if path == *exact {
            risks.push(("dependency_change".to_string(), path.to_string()));
        }
    }
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

    /// Commit a baseline `src/lib.rs` so the tree has a HEAD to diff against.
    fn init_with_baseline(d: &TmpDir) {
        git_init(d.path());
        std::fs::write(d.path.join("src/lib.rs").as_path(), "fn base() {}\n").unwrap();
        git(d.path(), &["add", "-A"]);
        git(d.path(), &["commit", "-qm", "init"]);
    }

    #[test]
    fn diff_captures_untracked_file_as_add() {
        let d = TmpDir::new("diff-untracked");
        init_with_baseline(&d);
        // A brand-new file the agent never `git add`-ed: invisible to
        // `git diff HEAD`, must still be picked up as an Add.
        std::fs::write(d.path.join("src/new.rs").as_path(), "fn n() {}\n").unwrap();
        let changes = diff_worktree(d.path(), d.path()).unwrap();
        assert_eq!(changes, vec![Change::Add("src/new.rs".to_string())]);
    }

    #[test]
    fn diff_detects_modify_and_delete() {
        let d = TmpDir::new("diff-mod-del");
        git_init(d.path());
        std::fs::write(d.path.join("src/a.rs").as_path(), "fn a() {}\n").unwrap();
        std::fs::write(d.path.join("src/b.rs").as_path(), "fn b() {}\n").unwrap();
        git(d.path(), &["add", "-A"]);
        git(d.path(), &["commit", "-qm", "init"]);
        std::fs::write(d.path.join("src/a.rs").as_path(), "fn a() { /* x */ }\n").unwrap();
        std::fs::remove_file(d.path.join("src/b.rs").as_path()).unwrap();

        let changes = diff_worktree(d.path(), d.path()).unwrap();
        assert!(
            changes.contains(&Change::Modify("src/a.rs".to_string())),
            "expected Modify(src/a.rs) in {:?}",
            changes
        );
        assert!(
            changes.contains(&Change::Delete("src/b.rs".to_string())),
            "expected Delete(src/b.rs) in {:?}",
            changes
        );
    }

    #[test]
    fn diff_parses_rename_without_splitn_bug() {
        let d = TmpDir::new("diff-rename");
        git_init(d.path());
        // Non-trivial content so git scores the move as a 100% rename.
        std::fs::write(
            d.path.join("src/old.rs").as_path(),
            "fn renamed_me() { let _ = 1 + 2 + 3; }\n",
        )
        .unwrap();
        git(d.path(), &["add", "-A"]);
        git(d.path(), &["commit", "-qm", "init"]);
        git(d.path(), &["mv", "src/old.rs", "src/new.rs"]);

        let changes = diff_worktree(d.path(), d.path()).unwrap();
        // The old splitn(2, '\t') parse produced a single bogus path
        // "src/old.rs\tsrc/new.rs"; the typed parse must yield distinct paths.
        assert_eq!(
            changes,
            vec![Change::Rename {
                from: "src/old.rs".to_string(),
                to: "src/new.rs".to_string(),
            }]
        );
    }

    #[test]
    fn apply_changes_creates_modifies_and_deletes() {
        let d = TmpDir::new("apply-cmd");
        let wt = d.path().join("wt");
        let proj = d.path().join("proj");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::create_dir_all(proj.join("src")).unwrap();

        // Worktree state for the create + modify; project state for the delete.
        std::fs::write(wt.join("src/added.rs"), "added\n").unwrap();
        std::fs::write(wt.join("src/changed.rs"), "new\n").unwrap();
        std::fs::write(proj.join("src/changed.rs"), "old\n").unwrap();
        std::fs::write(proj.join("src/gone.rs"), "remove me\n").unwrap();

        apply_changes(
            &proj,
            &wt,
            &[
                Change::Add("src/added.rs".to_string()),
                Change::Modify("src/changed.rs".to_string()),
                Change::Delete("src/gone.rs".to_string()),
            ],
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(proj.join("src/added.rs")).unwrap(),
            "added\n"
        );
        assert_eq!(
            std::fs::read_to_string(proj.join("src/changed.rs")).unwrap(),
            "new\n"
        );
        assert!(
            !proj.join("src/gone.rs").exists(),
            "delete must actually remove the file (not be silently dropped)"
        );
    }

    #[test]
    fn apply_changes_renames_in_workspace() {
        let d = TmpDir::new("apply-rename");
        let wt = d.path().join("wt");
        let proj = d.path().join("proj");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::create_dir_all(proj.join("src")).unwrap();
        // Worktree holds the file at its new path; project still has the old.
        std::fs::write(wt.join("src/new.rs"), "moved\n").unwrap();
        std::fs::write(proj.join("src/old.rs"), "moved\n").unwrap();

        apply_changes(
            &proj,
            &wt,
            &[Change::Rename {
                from: "src/old.rs".to_string(),
                to: "src/new.rs".to_string(),
            }],
        )
        .unwrap();

        assert!(!proj.join("src/old.rs").exists(), "old path must be removed");
        assert_eq!(
            std::fs::read_to_string(proj.join("src/new.rs")).unwrap(),
            "moved\n"
        );
    }

    #[test]
    fn apply_delete_of_missing_target_is_idempotent() {
        let d = TmpDir::new("apply-del-missing");
        let wt = d.path().join("wt");
        let proj = d.path().join("proj");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&proj).unwrap();
        // Deleting something already absent must succeed, not error.
        apply_changes(&proj, &wt, &[Change::Delete("src/never.rs".to_string())]).unwrap();
    }

    #[test]
    fn high_risk_flags_delete_and_rename() {
        let changes = vec![
            Change::Delete("src/dropped.rs".to_string()),
            Change::Rename {
                from: "src/a.rs".to_string(),
                to: "schemas/control.schema.json".to_string(),
            },
        ];
        let risks = detect_high_risk(&changes);
        assert!(risks.contains(&("file_deleted".to_string(), "src/dropped.rs".to_string())));
        // A rename removes its source (file_deleted) and may land on a
        // protected path (protected_path_change on the destination).
        assert!(risks.contains(&("file_deleted".to_string(), "src/a.rs".to_string())));
        assert!(risks.contains(&(
            "protected_path_change".to_string(),
            "schemas/control.schema.json".to_string()
        )));
    }

    #[test]
    fn change_paths_covers_both_ends_of_a_rename() {
        let r = Change::Rename {
            from: "a".to_string(),
            to: "b".to_string(),
        };
        assert_eq!(r.paths(), vec!["a", "b"]);
        assert_eq!(Change::Add("x".to_string()).paths(), vec!["x"]);
    }
}
