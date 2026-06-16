use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub struct PathNormalizer {
    root: PathBuf,
    protected_paths: Vec<String>,
}

impl PathNormalizer {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            protected_paths: vec![
                ".git".into(),
                ".ctl".into(),
                ".ctl/tasks".into(),
                ".control".into(),
                "schemas".into(),
                "Cargo.toml".into(),
                "Cargo.lock".into(),
            ],
        }
    }

    /// Normalize and validate a path against boundary rules.
    ///
    /// Rejects: absolute paths, `..`, UNC (`\\server\share`), drive prefixes,
    /// symlinks, junctions, and root escapes. Does NOT check protected paths
    /// (use `normalize_write` for write-scope validation).
    pub fn normalize(&self, path_str: &str) -> Result<PathBuf> {
        // Reject Windows UNC paths explicitly before component parsing
        if path_str.starts_with("\\\\") || path_str.starts_with("//") {
            return Err(anyhow!("UNC paths are not allowed: {}", path_str));
        }

        let path = Path::new(path_str);

        if path.is_absolute() {
            return Err(anyhow!("Absolute paths are not allowed: {}", path_str));
        }

        let mut normalized = PathBuf::new();
        for comp in path.components() {
            match comp {
                Component::ParentDir => return Err(anyhow!(".. is not allowed")),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(anyhow!("Absolute path components not allowed"))
                }
                Component::CurDir => continue,
                Component::Normal(c) => normalized.push(c),
            }
        }

        if normalized.as_os_str().is_empty() {
            return Err(anyhow!("Empty path after normalization"));
        }

        // Walk ancestry checking for symlinks/junctions
        let mut current = self.root.clone();
        for comp in normalized.components() {
            current.push(comp);
            if current.exists() {
                let meta = fs::symlink_metadata(&current)?;
                // is_symlink() returns true for both symlinks and junctions on Windows
                if meta.file_type().is_symlink() {
                    return Err(anyhow!(
                        "Symlink/Junction detected in ancestry: {:?}",
                        current
                    ));
                }
            }
        }

        // PATH-001: Must canonicalize before policy decision.
        // If the target doesn't exist yet (new file), canonicalize the parent.
        let canon = match fs::canonicalize(&current) {
            Ok(c) => c,
            Err(_) => {
                let parent = current
                    .parent()
                    .ok_or_else(|| anyhow!("Cannot verify path (no parent): {}", path_str))?;
                let parent_canon = fs::canonicalize(parent).map_err(|_| {
                    anyhow!("Cannot canonicalize parent directory for: {}", path_str)
                })?;
                let file_name = current
                    .file_name()
                    .ok_or_else(|| anyhow!("Invalid path (no filename): {}", path_str))?;
                parent_canon.join(file_name)
            }
        };
        let root_canon = fs::canonicalize(&self.root)
            .map_err(|_| anyhow!("Cannot canonicalize root: {}", self.root.display()))?;
        if !canon.starts_with(&root_canon) {
            return Err(anyhow!("Path escapes root directory: {}", path_str));
        }

        Ok(normalized)
    }

    /// Normalize and validate a path for write operations.
    ///
    /// Performs all `normalize()` checks plus protected-path enforcement.
    pub fn normalize_write(&self, path_str: &str) -> Result<PathBuf> {
        let normalized = self.normalize(path_str)?;
        if self.is_protected(&normalized) {
            return Err(anyhow!("Write path is protected: {}", normalized.display()));
        }
        Ok(normalized)
    }

    /// Check whether a normalized path is under a protected root.
    /// Uses separator-boundary matching so ".git" does not match "gitignored".
    fn is_protected(&self, path: &Path) -> bool {
        let s = path.to_string_lossy().to_lowercase();
        // Carve-outs from the blanket `.ctl` protection: AI-writable control-plane
        // config, treated like `.ctl/spec` (which the gate already exempts). These let
        // the workflow doc be revised and the legacy scripts dir be retired under
        // governance instead of requiring a human to bypass the boundary.
        let s_fwd = s.replace('\\', "/");
        for w in [".ctl/workflow.md", ".ctl/scripts"] {
            if s_fwd == w || s_fwd.starts_with(&format!("{w}/")) {
                return false;
            }
        }
        for p in &self.protected_paths {
            let p_lower = p.to_lowercase();
            if s == p_lower
                || s.starts_with(&format!("{}/", p_lower))
                || s.starts_with(&format!("{}\\", p_lower))
            {
                return true;
            }
        }
        false
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn unique_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omp_norm_test_{}", nanos));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::create_dir_all(dir.join(".ctl")).unwrap();
        fs::create_dir_all(dir.join(".control")).unwrap();
        fs::create_dir_all(dir.join("schemas")).unwrap();
        fs::write(dir.join("Cargo.toml"), "").unwrap();
        fs::write(dir.join("Cargo.lock"), "").unwrap();
        dir
    }
    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }
    // ---- Pure logic: no filesystem required ----
    #[test]
    fn reject_unc_backslash() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize("\\\\server\\share\\file").is_err());
    }
    #[test]
    fn reject_unc_slash() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize("//server/share/file").is_err());
    }
    #[test]
    fn reject_absolute_unix() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize("/etc/passwd").is_err());
    }
    #[test]
    fn reject_parent_dir_simple() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize("../escape").is_err());
    }
    #[test]
    fn reject_parent_dir_nested() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize("src/../../../etc/passwd").is_err());
    }
    #[test]
    fn reject_empty_after_normalize() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize(".").is_err());
        assert!(norm.normalize("./").is_err());
    }
    #[test]
    fn normalize_strips_dot_prefix() {
        let dir = unique_dir();
        let norm = PathNormalizer::new(dir.clone());
        let result = norm.normalize("./src/main.rs").unwrap();
        assert_eq!(result, PathBuf::from("src/main.rs"));
        cleanup(&dir);
    }
    #[test]
    fn accept_relative_path() {
        let dir = unique_dir();
        let norm = PathNormalizer::new(dir.clone());
        assert!(norm.normalize("src/main.rs").is_ok());
        cleanup(&dir);
    }
    #[test]
    fn windows_backslash_relative() {
        let dir = unique_dir();
        let norm = PathNormalizer::new(dir.clone());
        assert!(norm.normalize("src\\main.rs").is_ok());
        cleanup(&dir);
    }
    // ---- Protected path tests (write paths) ----
    #[test]
    fn reject_protected_git_root() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize_write(".git").is_err());
    }
    #[test]
    fn reject_protected_git_nested() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize_write(".git/config").is_err());
    }
    #[test]
    fn reject_protected_trellis() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize_write(".ctl/control/events.jsonl").is_err());
    }
    #[test]
    fn reject_canonical_task_events_path() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm
            .normalize_write(".ctl/tasks/example-task/events.jsonl")
            .is_err());
    }
    #[test]
    fn reject_protected_control_events() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize_write(".control/events.jsonl").is_err());
    }
    #[test]
    fn reject_protected_schemas() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize_write("schemas/foo.json").is_err());
    }
    #[test]
    fn reject_protected_cargo_toml() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize_write("Cargo.toml").is_err());
    }
    #[test]
    fn reject_protected_cargo_lock() {
        let root = PathBuf::from(".");
        let norm = PathNormalizer::new(root);
        assert!(norm.normalize_write("Cargo.lock").is_err());
    }
    #[test]
    fn accept_carveout_ctl_workflow_md() {
        // Carved out of .ctl protection — the workflow doc is AI-writable config.
        // Use a temp root that actually contains `.ctl/`: `normalize` canonicalizes
        // the parent dir, and `.ctl/` is gitignored (absent on a fresh checkout),
        // so a root of "." only works where ctl has already run.
        let dir = unique_dir();
        let norm = PathNormalizer::new(dir.clone());
        assert!(norm.normalize_write(".ctl/workflow.md").is_ok());
        cleanup(&dir);
    }
    #[test]
    fn accept_carveout_ctl_scripts() {
        let dir = unique_dir();
        let norm = PathNormalizer::new(dir.clone());
        assert!(norm.normalize_write(".ctl/scripts").is_ok());
        cleanup(&dir);
    }
    #[test]
    fn ctl_tasks_still_protected_after_carveout() {
        // The carve-out must not widen to the canonical ledger.
        let norm = PathNormalizer::new(PathBuf::from("."));
        assert!(norm.normalize_write(".ctl/tasks/foo/events.jsonl").is_err());
    }
    #[test]
    fn accept_protected_paths_for_read() {
        let dir = unique_dir();
        let norm = PathNormalizer::new(dir.clone());
        assert!(
            norm.normalize("schemas/foo.json").is_ok(),
            "schemas should be readable"
        );
        assert!(
            norm.normalize(".git/config").is_ok(),
            ".git should be readable"
        );
        assert!(
            norm.normalize("Cargo.toml").is_ok(),
            "Cargo.toml should be readable"
        );
        cleanup(&dir);
    }
    #[test]
    fn accept_non_protected_prefix() {
        // "gitignored" must NOT match ".git"
        let dir = unique_dir();
        fs::create_dir_all(dir.join("gitignored")).unwrap();
        let norm = PathNormalizer::new(dir.clone());
        assert!(norm.normalize("gitignored/foo").is_ok());
        cleanup(&dir);
    }
    #[test]
    fn accept_non_protected_similar_name() {
        // "schemas-backup" must NOT match "schemas"
        let dir = unique_dir();
        fs::create_dir_all(dir.join("schemas-backup")).unwrap();
        let norm = PathNormalizer::new(dir.clone());
        assert!(norm.normalize("schemas-backup/foo.json").is_ok());
        cleanup(&dir);
    }
    // ---- Canonicalize behavior (PATH-001) ----
    #[test]
    fn accept_new_file_in_existing_dir() {
        let dir = unique_dir();
        let norm = PathNormalizer::new(dir.clone());
        // src/ exists but new_file.rs doesn't — parent canonicalization should succeed
        let result = norm.normalize("src/new_file.rs");
        assert!(
            result.is_ok(),
            "New file in existing dir should be accepted: {:?}",
            result
        );
        assert_eq!(result.unwrap(), PathBuf::from("src/new_file.rs"));
        cleanup(&dir);
    }
    #[test]
    fn reject_nonexistent_deep_path() {
        let dir = unique_dir();
        let norm = PathNormalizer::new(dir.clone());
        // src/ exists but src/deep/ does not — parent canonicalization fails
        let result = norm.normalize("src/deep/nested.rs");
        assert!(
            result.is_err(),
            "Path with nonexistent parent dir must be rejected"
        );
        cleanup(&dir);
    }
    #[test]
    fn reject_root_canonicalize_failure() {
        let norm = PathNormalizer::new(PathBuf::from("/nonexistent/root/definitely/not/real"));
        let result = norm.normalize("some_file.rs");
        assert!(
            result.is_err(),
            "Should reject when root cannot be canonicalized"
        );
    }
    // ---- Symlink/Junction test ----
    #[test]
    fn reject_symlink_in_ancestry() {
        let dir = unique_dir();
        let target = dir.join("target_dir");
        fs::create_dir_all(&target).unwrap();
        let link = dir.join("src").join("link");
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    &link.to_string_lossy(),
                    &target.to_string_lossy(),
                ])
                .output()
                .expect("Failed to execute mklink");
            if !output.status.success() {
                panic!(
                    "mklink /J failed — cannot verify junction rejection: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
        #[cfg(not(windows))]
        {
            std::os::unix::fs::symlink(&target, &link)
                .expect("Failed to create symlink — cannot verify symlink rejection");
        }
        let norm = PathNormalizer::new(dir.clone());
        assert!(
            norm.normalize("src/link/file.txt").is_err(),
            "Symlink/Junction path must be rejected"
        );
        cleanup(&dir);
    }
}
