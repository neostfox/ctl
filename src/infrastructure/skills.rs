//! Embedded control-plane skills and hooks, compiled into the binary via `include_str!`.
//! `cmd_init` writes these into the target project's `.omp/` directory
//! so the AI model automatically loads the control plane on every session.

use anyhow::{anyhow, Result};
use std::path::Path;

pub struct EmbeddedFile {
    pub relative_path: &'static str, // e.g. "skills/control-guard/SKILL.md"
    pub content: &'static str,
}

pub fn all_embedded_files() -> Vec<EmbeddedFile> {
    vec![
        // Skills (6: entry/router + planning + review + diagnosis + spec bootstrap/update)
        EmbeddedFile {
            relative_path: "skills/control-guard/SKILL.md",
            content: include_str!("../../.omp/skills/control-guard/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-brainstorm/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-brainstorm/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-review/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-review/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-diagnose/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-diagnose/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-spec-update/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-spec-update/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-spec-bootstrap/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-spec-bootstrap/SKILL.md"),
        },
        // Fixed review-rule files the skills reference. These are universal
        // (not project-specific), so they ship verbatim with `ctl init` rather
        // than being regenerated per project by ctl-spec-bootstrap. Closes the
        // distribution gap: ctl-review/ctl-diagnose pointed at these but they
        // lived only in the gitignored, per-project `.ctl/spec/`.
        EmbeddedFile {
            relative_path: "spec/guides/review-contract.md",
            content: include_str!("../../.omp/spec/guides/review-contract.md"),
        },
        EmbeddedFile {
            relative_path: "spec/guides/decay-risks.md",
            content: include_str!("../../.omp/spec/guides/decay-risks.md"),
        },
        EmbeddedFile {
            relative_path: "spec/guides/test-decay-risks.md",
            content: include_str!("../../.omp/spec/guides/test-decay-risks.md"),
        },
        EmbeddedFile {
            relative_path: "spec/guides/failure-diagnosis.md",
            content: include_str!("../../.omp/spec/guides/failure-diagnosis.md"),
        },
        // Attribution for adapted MIT skill packs
        EmbeddedFile {
            relative_path: "skills/NOTICE.md",
            content: include_str!("../../.omp/skills/NOTICE.md"),
        },
        // Hooks (OMP native extension format)
        EmbeddedFile {
            relative_path: "hooks/pre/ctl-context.ts",
            content: include_str!("../../.omp/hooks/pre/ctl-context.ts"),
        },
    ]
}

/// Default `.omp/settings.json` — only skills autoLoad.
/// Hooks are auto-discovered by OMP from `hooks/pre/*.js`.
pub fn default_omp_settings() -> &'static str {
    r#"{
  "skills": {
    "autoLoad": [
      ".omp/skills/control-guard/SKILL.md"
    ]
  }
}"#
}
pub fn inject_all(project_root: &std::path::Path) -> anyhow::Result<usize> {
    let omp_dir = project_root.join(".omp");

    // Write all embedded files (skills + hooks)
    let mut count = 0usize;
    for file in all_embedded_files() {
        let file_path = omp_dir.join(file.relative_path);
        let parent = file_path.parent().unwrap();
        std::fs::create_dir_all(parent)?;
        if !file_path.exists() {
            std::fs::write(&file_path, file.content)?;
            count += 1;
        }
    }

    // Create or merge .omp/settings.json
    let settings_path = omp_dir.join("settings.json");
    if !settings_path.exists() {
        std::fs::write(&settings_path, default_omp_settings())?;
    } else {
        merge_settings(&settings_path)?;
    }

    Ok(count)
}

/// Merge control-guard autoLoad + hooks into an existing settings.json.
/// Preserves all other settings the user may have configured.
fn merge_settings(settings_path: &std::path::Path) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(settings_path)?;
    let mut settings: serde_json::Value = serde_json::from_str(&content)?;

    // 1. Merge skills.autoLoad
    let autoload_target = ".omp/skills/control-guard/SKILL.md";
    if settings.get("skills").is_none() {
        settings["skills"] = serde_json::json!({});
    }
    let skills_obj = settings["skills"]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("settings.json 'skills' is not an object"))?;
    if skills_obj.get("autoLoad").is_none() {
        skills_obj.insert("autoLoad".to_string(), serde_json::json!([]));
    }

    let arr = settings["skills"]["autoLoad"]
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("settings.json 'skills.autoLoad' is not an array"))?;

    let already_exists = arr.iter().any(|v| v.as_str() == Some(autoload_target));
    if !already_exists {
        arr.push(serde_json::Value::String(autoload_target.to_string()));
    }
    // Hooks are auto-discovered by OMP from hooks/pre/*.js — no settings merge needed.

    // Write back atomically
    let temp_path = settings_path.with_extension("json.tmp");
    let output = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&temp_path, &output)?;
    std::fs::rename(&temp_path, settings_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TmpDir {
        path: PathBuf,
    }
    impl TmpDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ctl-skills-test-{}-{}",
                std::process::id(),
                tag
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn inject_ships_fixed_review_rule_guides() {
        let d = TmpDir::new("guides");
        inject_all(&d.path).unwrap();
        for f in [
            "review-contract.md",
            "decay-risks.md",
            "test-decay-risks.md",
            "failure-diagnosis.md",
        ] {
            assert!(
                d.path.join(".omp/spec/guides").join(f).exists(),
                "ctl init must ship the fixed guide {f}"
            );
        }
    }

    /// The distribution gap: ctl-review linked guide files that `ctl init` never
    /// shipped. Assert every `../../spec/guides/*.md` the skill references now
    /// resolves to a real file after init.
    #[test]
    fn ctl_review_guide_references_resolve_after_init() {
        let d = TmpDir::new("refs");
        inject_all(&d.path).unwrap();
        let skill =
            std::fs::read_to_string(d.path.join(".omp/skills/ctl-review/SKILL.md")).unwrap();
        let mut checked = 0;
        for chunk in skill.split("../../spec/guides/").skip(1) {
            let end = chunk.find(".md").expect("guide link ends in .md") + 3;
            let rel = &chunk[..end];
            assert!(
                d.path.join(".omp/spec/guides").join(rel).exists(),
                "ctl-review references missing guide: {rel}"
            );
            checked += 1;
        }
        assert!(
            checked >= 3,
            "expected several guide references, got {checked}"
        );
    }
}

// ── Managed control-guard protocol: reusable drift checker ───────────────────
//
// The canonical protocol (`.agent/protocols/control-guard.md`) is embedded
// verbatim as a "managed core" block inside every platform control-guard skill.
// These helpers are the single source of truth for parsing and comparing that
// block. They are compiled into the binary (NOT `#[cfg(test)]`) so
// `ctl adapter doctor` REUSES the exact same check the CI drift test asserts —
// see `control_guard_protocol_sync` below and `application::adapter_doctor_report`.

/// Canonical control-guard protocol source, relative to the project root.
pub const CANONICAL_PROTOCOL_PATH: &str = ".agent/protocols/control-guard.md";

const CORE_START_PREFIX: &str = "<!-- ctl:control-guard-core:start version=";
const CORE_END_MARKER: &str = "<!-- ctl:control-guard-core:end -->";

/// A platform's control-guard wiring: which adapter it backs, the skill file
/// carrying the managed core, and the host entry point the skill must reference
/// (OMP hook / opencode plugin).
pub struct PlatformSkill {
    /// Registry adapter name this platform backs (e.g. "omp", "opencode").
    pub adapter: &'static str,
    /// Display label (e.g. "OMP").
    pub label: &'static str,
    /// Skill file carrying the managed-core block.
    pub skill_path: &'static str,
    /// Host entry point referenced OUTSIDE the managed core.
    pub entry_point: &'static str,
}

/// Every platform control-guard skill, keyed by the adapter it backs. Adding a
/// platform means adding it here; the CI drift test and `ctl adapter doctor`
/// both iterate this list.
pub fn platform_skills() -> &'static [PlatformSkill] {
    &[
        PlatformSkill {
            adapter: "omp",
            label: "OMP",
            skill_path: ".omp/skills/control-guard/SKILL.md",
            entry_point: ".omp/hooks/pre/ctl-context.ts",
        },
        PlatformSkill {
            adapter: "opencode",
            label: "opencode",
            skill_path: ".opencode/skills/control-guard/SKILL.md",
            entry_point: ".opencode/plugins/ctl-gate.ts",
        },
    ]
}

/// The platform wiring backing a given adapter, if any.
pub fn platform_skill_for(adapter: &str) -> Option<&'static PlatformSkill> {
    platform_skills().iter().find(|p| p.adapter == adapter)
}

/// Normalize protocol text for comparison: LF endings, trailing per-line
/// whitespace stripped, leading/trailing blank lines trimmed — tolerant of
/// insertion whitespace, strict on content.
pub fn normalize_protocol(s: &str) -> String {
    s.replace("\r\n", "\n")
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}

/// Extract `(version, normalized_core)` from a skill's managed-core block. Errors
/// unless exactly one well-formed block exists — catching missing, duplicate, or
/// one-sided markers, and an unparseable version.
pub fn extract_managed_core(skill: &str) -> Result<(String, String)> {
    let starts = skill.matches(CORE_START_PREFIX).count();
    let ends = skill.matches(CORE_END_MARKER).count();
    if starts != 1 {
        return Err(anyhow!(
            "expected exactly one managed-core start marker, found {starts}"
        ));
    }
    if ends != 1 {
        return Err(anyhow!(
            "expected exactly one managed-core end marker, found {ends}"
        ));
    }
    let start_idx = skill.find(CORE_START_PREFIX).unwrap();
    let end_idx = skill.find(CORE_END_MARKER).unwrap();
    if start_idx >= end_idx {
        return Err(anyhow!("managed-core end marker precedes start marker"));
    }
    let after = &skill[start_idx + CORE_START_PREFIX.len()..];
    let line_len = after.find('\n').unwrap_or(after.len());
    let version = after[..line_len]
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches("-->")
        .trim()
        .to_string();
    if version.is_empty() {
        return Err(anyhow!(
            "could not parse version from managed-core start marker"
        ));
    }
    let body_start = start_idx + CORE_START_PREFIX.len() + line_len + 1;
    Ok((version, normalize_protocol(&skill[body_start..end_idx])))
}

/// Outcome of checking one skill's managed core against the canonical protocol.
pub enum DriftStatus {
    /// Core matches canonical and its version is declared there.
    InSync(String),
    /// Core diverged, markers malformed, or version undeclared — with a reason.
    Drift(String),
    /// The skill file is absent, so drift could not be evaluated.
    Missing,
}

/// Evaluate managed-protocol drift for one skill, reading files under
/// `project_root`. The runtime counterpart of the CI drift test, reusing the
/// same parse/normalize/compare primitives.
pub fn evaluate_protocol_drift(project_root: &Path, skill_rel: &str) -> DriftStatus {
    let skill = match std::fs::read_to_string(project_root.join(skill_rel)) {
        Ok(s) => s,
        Err(_) => return DriftStatus::Missing,
    };
    let canonical = match std::fs::read_to_string(project_root.join(CANONICAL_PROTOCOL_PATH)) {
        Ok(s) => normalize_protocol(&s),
        Err(e) => return DriftStatus::Drift(format!("canonical protocol unreadable: {e}")),
    };
    let (version, core) = match extract_managed_core(&skill) {
        Ok(vc) => vc,
        Err(e) => return DriftStatus::Drift(format!("{skill_rel}: {e}")),
    };
    if core != canonical {
        return DriftStatus::Drift(format!(
            "{skill_rel}: managed core drifted from {CANONICAL_PROTOCOL_PATH}"
        ));
    }
    if !canonical.contains(&format!("CONTROL_GUARD_PROTOCOL_VERSION = {version}")) {
        return DriftStatus::Drift(format!(
            "version {version} not declared in {CANONICAL_PROTOCOL_PATH}"
        ));
    }
    DriftStatus::InSync(version)
}

/// Deterministic drift check for the control-guard protocol: the canonical
/// source and the managed-core block embedded in every platform skill must be
/// byte-identical (normalized) and declare the same version. Drift fails CI
/// (this runs under `cargo test`). There is no generator — skills are authored
/// directly; this only refuses to let them diverge. Adding a platform means
/// adding its skill to `platform_skills()` above. This test and the runtime
/// `evaluate_protocol_drift` share the same parse/normalize primitives.
#[cfg(test)]
mod control_guard_protocol_sync {
    use super::{
        evaluate_protocol_drift, extract_managed_core, normalize_protocol, platform_skill_for,
        platform_skills, DriftStatus, CANONICAL_PROTOCOL_PATH,
    };
    use std::path::PathBuf;

    /// Tokens that are platform-specific and must NOT leak into the managed core.
    const PLATFORM_TOKENS: &[&str] = &[
        "tool.execute.before",
        "experimental.chat.system.transform",
        ".opencode/plugins",
        ".omp/hooks",
        "PreToolUse",
        "job poll",
        "OMP todo",
        "@oh-my-pi",
        "ctl-gate.ts",
        "subagent_type",
    ];

    fn manifest_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn read(rel: &str) -> String {
        let p = manifest_root().join(rel);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("missing {rel}: {e}"))
    }

    #[test]
    fn managed_core_is_identical_across_canonical_and_all_skills() {
        let canonical = normalize_protocol(&read(CANONICAL_PROTOCOL_PATH));

        let mut prev_version: Option<String> = None;
        for ps in platform_skills() {
            let (label, path, entry_point) = (ps.label, ps.skill_path, ps.entry_point);
            let skill = read(path);
            let (version, core) =
                extract_managed_core(&skill).unwrap_or_else(|e| panic!("{label}: {e}"));

            // (3)/(9) the core must equal the canonical source exactly. If the
            // canonical changes but a skill isn't re-synced, this fails.
            assert_eq!(
                core, canonical,
                "{label}: managed core drifted from {CANONICAL_PROTOCOL_PATH} — re-sync the skill"
            );

            // (1) all skills declare the same version, and it matches canonical.
            if let Some(prev) = &prev_version {
                assert_eq!(
                    &version, prev,
                    "{label}: protocol version disagrees with another skill"
                );
            }
            prev_version = Some(version.clone());
            assert!(
                canonical.contains(&format!("CONTROL_GUARD_PROTOCOL_VERSION = {version}")),
                "{label}: canonical source must declare CONTROL_GUARD_PROTOCOL_VERSION = {version}"
            );

            // (6) the platform entry point appears OUTSIDE the managed core.
            assert!(
                skill.contains(entry_point),
                "{label}: skill must reference its platform entry point {entry_point}"
            );
            assert!(
                !core.contains(entry_point),
                "{label}: platform entry point {entry_point} leaked into the managed core"
            );
        }

        // (7) no platform-specific token leaked into the (shared) core.
        for tok in PLATFORM_TOKENS {
            assert!(
                !canonical.contains(tok),
                "platform token '{tok}' must not appear in the canonical core"
            );
        }
    }

    /// The runtime drift evaluator must agree with the CI test: every platform
    /// skill is InSync against canonical when read from the project root.
    #[test]
    fn runtime_drift_evaluator_reports_in_sync_for_every_platform() {
        let root = manifest_root();
        for ps in platform_skills() {
            match evaluate_protocol_drift(&root, ps.skill_path) {
                DriftStatus::InSync(v) => assert!(!v.is_empty(), "{}: version parsed", ps.label),
                DriftStatus::Drift(why) => panic!("{}: unexpected drift — {why}", ps.label),
                DriftStatus::Missing => panic!("{}: skill unexpectedly missing", ps.label),
            }
        }
    }

    #[test]
    fn runtime_drift_evaluator_reports_missing_for_absent_skill() {
        let root = manifest_root();
        assert!(matches!(
            evaluate_protocol_drift(&root, ".omp/skills/does-not-exist/SKILL.md"),
            DriftStatus::Missing
        ));
    }

    #[test]
    fn platform_skill_lookup_is_by_adapter_name() {
        assert!(platform_skill_for("omp").is_some());
        assert!(platform_skill_for("opencode").is_some());
        assert!(platform_skill_for("nope").is_none());
    }

    #[test]
    fn extract_managed_core_rejects_missing_markers() {
        assert!(extract_managed_core("no markers here").is_err());
    }

    /// A core that diverges from canonical is reported as Drift, not InSync —
    /// verified with a synthetic skill written under a temp root.
    #[test]
    fn runtime_drift_evaluator_flags_a_tampered_core() {
        let root = std::env::temp_dir().join(format!("ctl-drift-{}", std::process::id()));
        let skill_rel = ".omp/skills/control-guard/SKILL.md";
        std::fs::create_dir_all(root.join(skill_rel).parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join(CANONICAL_PROTOCOL_PATH).parent().unwrap()).unwrap();
        std::fs::write(
            root.join(CANONICAL_PROTOCOL_PATH),
            "CONTROL_GUARD_PROTOCOL_VERSION = 1\nreal core\n",
        )
        .unwrap();
        std::fs::write(
            root.join(skill_rel),
            "intro\n<!-- ctl:control-guard-core:start version=1 -->\nTAMPERED core\n<!-- ctl:control-guard-core:end -->\nrest\n",
        )
        .unwrap();
        let status = evaluate_protocol_drift(&root, skill_rel);
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            matches!(status, DriftStatus::Drift(_)),
            "tampered core must drift"
        );
    }
}
