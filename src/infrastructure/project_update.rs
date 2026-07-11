//! Project-local template synchronization for `ctl update --merge`.
//!
//! This module updates only files that `ctl init` owns for an already-configured
//! platform. User-modified files are never overwritten by the default merge
//! policy; they receive a `.new` sibling containing the new embedded template.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST_REL: &str = ".ctl/ctl-template-hashes.json";
const MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default)]
pub struct UpdateOptions {
    pub force: bool,
    pub skip_conflicts: bool,
    pub dry_run: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct UpdateSummary {
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub conflicts: usize,
    pub skipped: usize,
    pub files: Vec<UpdateFile>,
}

#[derive(Debug, Serialize)]
pub struct UpdateFile {
    pub path: String,
    pub action: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Manifest {
    version: u32,
    #[serde(default)]
    hashes: BTreeMap<String, String>,
}

fn manifest_path(project_root: &Path) -> PathBuf {
    project_root.join(MANIFEST_REL)
}

fn read_manifest(project_root: &Path) -> Result<Manifest> {
    let path = manifest_path(project_root);
    if !path.exists() {
        return Ok(Manifest {
            version: MANIFEST_VERSION,
            hashes: BTreeMap::new(),
        });
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("reading ctl template manifest {}", path.display()))?;
    let manifest: Manifest = serde_json::from_str(&content)
        .with_context(|| format!("parsing ctl template manifest {}", path.display()))?;
    if manifest.version != MANIFEST_VERSION {
        anyhow::bail!(
            "unsupported ctl template manifest version {} (expected {})",
            manifest.version,
            MANIFEST_VERSION
        );
    }
    Ok(manifest)
}

fn write_manifest(project_root: &Path, manifest: &Manifest) -> Result<()> {
    let ctl_dir = project_root.join(".ctl");
    fs::create_dir_all(&ctl_dir)
        .with_context(|| format!("creating ctl directory {}", ctl_dir.display()))?;
    let path = manifest_path(project_root);
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(manifest)?)
        .with_context(|| format!("writing ctl template manifest {}", tmp.display()))?;
    // Windows cannot rename over an existing file. The manifest is a derived,
    // non-canonical projection, so replacing it after the temp write is safe.
    if path.exists() {
        fs::remove_file(&path)
            .with_context(|| format!("replacing ctl template manifest {}", path.display()))?;
    }
    fs::rename(&tmp, &path)
        .with_context(|| format!("installing ctl template manifest {}", path.display()))?;
    Ok(())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn hash_file(path: &Path) -> Result<String> {
    Ok(hash_bytes(&fs::read(path)?))
}

fn rel(root: &Path, path: &str) -> PathBuf {
    root.join(path)
}

fn push_templates(
    out: &mut Vec<(String, String)>,
    base: &str,
    files: Vec<crate::infrastructure::skills::EmbeddedFile>,
) {
    for file in files {
        out.push((
            format!("{base}/{}", file.relative_path),
            file.content.to_string(),
        ));
    }
}

/// Enumerate only files belonging to integrations that are present in the project.
fn managed_templates(project_root: &Path) -> Vec<(String, String)> {
    let mut templates = Vec::new();
    if project_root.join(".omp").is_dir() {
        push_templates(
            &mut templates,
            ".omp",
            crate::infrastructure::skills::all_embedded_files(),
        );
        templates.push((
            ".omp/settings.json".to_string(),
            crate::infrastructure::skills::default_omp_settings().to_string(),
        ));
    }
    if project_root.join(".claude").is_dir() {
        push_templates(
            &mut templates,
            ".claude",
            crate::infrastructure::skills::claude_embedded_files(),
        );
    }
    if project_root.join(".opencode").is_dir() {
        push_templates(
            &mut templates,
            ".opencode",
            crate::infrastructure::skills::opencode_embedded_files(),
        );
        templates.push((
            ".opencode/package.json".to_string(),
            crate::infrastructure::skills::default_opencode_package_json().to_string(),
        ));
    }
    templates
}

/// Record the post-init bytes as the baseline for future safe merges.
pub fn record_initial_manifest(project_root: &Path) -> Result<usize> {
    let mut manifest = read_manifest(project_root)?;
    let mut recorded = 0;
    for (relative, template) in managed_templates(project_root) {
        // Preserve an existing baseline across repeated init calls. For a new
        // entry, only bytes that exactly match the embedded template are ctl-owned;
        // pre-existing user customizations must start as conflicts on update.
        if manifest.hashes.contains_key(&relative) {
            continue;
        }
        let path = rel(project_root, &relative);
        if !path.is_file() {
            continue;
        }
        let current_hash = hash_file(&path)?;
        if current_hash == hash_bytes(template.as_bytes()) {
            manifest.hashes.insert(relative, current_hash);
            recorded += 1;
        }
    }
    if recorded > 0 {
        write_manifest(project_root, &manifest)?;
    }
    Ok(recorded)
}

fn conflict_path(path: &Path) -> PathBuf {
    let first = PathBuf::from(format!("{}.new", path.display()));
    if !first.exists() {
        return first;
    }
    for n in 2..=9999 {
        let candidate = PathBuf::from(format!("{}.new{}", path.display(), n));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

/// Update managed integration files without overwriting local modifications.
pub fn update_project(project_root: &Path, options: UpdateOptions) -> Result<UpdateSummary> {
    let templates = managed_templates(project_root);
    let mut manifest = read_manifest(project_root)?;
    let mut summary = UpdateSummary::default();

    for (relative, content) in templates {
        let path = rel(project_root, &relative);
        let new_hash = hash_bytes(content.as_bytes());
        if !path.exists() {
            if !options.dry_run {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, content.as_bytes())?;
                manifest.hashes.insert(relative.clone(), new_hash);
            }
            summary.added += 1;
            summary.files.push(UpdateFile {
                path: relative,
                action: "added".to_string(),
            });
            continue;
        }

        let current_hash = hash_file(&path)?;
        if current_hash == new_hash {
            manifest.hashes.insert(relative.clone(), new_hash);
            summary.unchanged += 1;
            summary.files.push(UpdateFile {
                path: relative,
                action: "unchanged".to_string(),
            });
            continue;
        }

        let baseline_matches = manifest
            .hashes
            .get(&relative)
            .map(|baseline| baseline == &current_hash)
            .unwrap_or(false);
        if baseline_matches || options.force {
            if !options.dry_run {
                fs::write(&path, content.as_bytes())?;
                manifest.hashes.insert(relative.clone(), new_hash);
            }
            summary.updated += 1;
            summary.files.push(UpdateFile {
                path: relative,
                action: "updated".to_string(),
            });
            continue;
        }

        summary.conflicts += 1;
        if options.skip_conflicts {
            summary.skipped += 1;
            summary.files.push(UpdateFile {
                path: relative,
                action: "skipped-conflict".to_string(),
            });
            continue;
        }

        let new_path = conflict_path(&path);
        if !options.dry_run {
            fs::write(&new_path, content.as_bytes())?;
        }
        summary.files.push(UpdateFile {
            path: relative,
            action: format!("conflict -> {}", new_path.display()),
        });
    }

    if !options.dry_run {
        write_manifest(project_root, &manifest)?;
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ctl_project_update_{label}_{nanos}"));
        fs::create_dir_all(root.join(".omp/skills/control-guard")).unwrap();
        root
    }

    #[test]
    fn merge_preserves_user_modified_file_and_emits_conflict_artifact() {
        let root = temp_root("conflict");
        let path = root.join(".omp/skills/control-guard/SKILL.md");
        fs::write(&path, "user customization\n").unwrap();

        let summary = update_project(&root, UpdateOptions::default()).unwrap();

        assert_eq!(summary.conflicts, 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), "user customization\n");
        assert!(path.with_file_name("SKILL.md.new").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preexisting_custom_file_is_not_baselined_as_ctl_owned() {
        let root = temp_root("preexisting");
        let path = root.join(".omp/skills/control-guard/SKILL.md");
        fs::write(&path, "user customization\n").unwrap();

        assert_eq!(record_initial_manifest(&root).unwrap(), 0);
        let summary = update_project(&root, UpdateOptions::default()).unwrap();

        assert_eq!(summary.conflicts, 1);
        assert!(path.with_file_name("SKILL.md.new").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unchanged_file_is_detected_without_overwrite() {
        let root = temp_root("baseline");
        let path = root.join(".omp/skills/control-guard/SKILL.md");
        let template = crate::infrastructure::skills::all_embedded_files()
            .into_iter()
            .find(|file| file.relative_path == "skills/control-guard/SKILL.md")
            .unwrap();
        fs::write(&path, template.content).unwrap();
        record_initial_manifest(&root).unwrap();

        let summary = update_project(&root, UpdateOptions::default()).unwrap();

        assert!(summary.unchanged >= 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), template.content);
        let _ = fs::remove_dir_all(root);
    }
}
