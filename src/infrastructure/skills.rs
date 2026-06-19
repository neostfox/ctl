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
        // Workflow skills foundation (workflow-skills-foundation-v1): ctl-native
        // rewrites of the grill → PRD → tasks → TDD → handoff disciplines. Each
        // embeds the managed workflow-core verbatim (drift-checked against
        // `.agent/protocols/workflow-skills.md`); they ship with `ctl init` like
        // the other OMP skills and are routed (not auto-loaded).
        EmbeddedFile {
            relative_path: "skills/ctl-grill-with-spec/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-grill-with-spec/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-to-prd/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-to-prd/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-to-tasks/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-to-tasks/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-tdd-loop/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-tdd-loop/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-handoff/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-handoff/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-architecture-review/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-architecture-review/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-cli-reference/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-cli-reference/SKILL.md"),
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

// ── Managed workflow-skills protocol (workflow-skills-foundation-v1) ─────────
//
// A second, independent managed-block family. The canonical workflow-core
// (`.agent/protocols/workflow-skills.md`) is embedded verbatim inside EVERY
// workflow skill across both platforms; the `workflow_protocol_sync` test below
// refuses to let any copy diverge. Same parse/normalize primitives as
// control-guard, different markers.

/// Canonical workflow-skills protocol source, relative to the project root.
pub const WORKFLOW_CANONICAL_PROTOCOL_PATH: &str = ".agent/protocols/workflow-skills.md";

const WORKFLOW_CORE_START_PREFIX: &str = "<!-- ctl:workflow-core:start version=";
const WORKFLOW_CORE_END_MARKER: &str = "<!-- ctl:workflow-core:end -->";

/// One workflow skill file carrying the managed workflow-core block.
pub struct WorkflowSkill {
    /// Logical skill name shared across platforms (e.g. "ctl-grill-with-spec").
    pub skill: &'static str,
    /// Platform that backs it ("omp" | "opencode").
    pub platform: &'static str,
    /// Skill file path carrying the managed-core block.
    pub path: &'static str,
    /// A platform-specific token that MUST appear in this file (outside the
    /// core) — proves the skill is wired to its host, and that platform mechanics
    /// live outside the shared core.
    pub platform_marker: &'static str,
}

/// Every workflow skill, both platforms. Adding a workflow skill means adding its
/// two rows here (and embedding the OMP copy in `all_embedded_files`); the drift
/// test iterates this list. The five logical skills are the foundation set:
/// grill-with-spec, to-prd, to-tasks, tdd-loop, handoff.
pub fn workflow_skills() -> &'static [WorkflowSkill] {
    &[
        WorkflowSkill {
            skill: "ctl-grill-with-spec",
            platform: "omp",
            path: ".omp/skills/ctl-grill-with-spec/SKILL.md",
            platform_marker: "OMP Integration",
        },
        WorkflowSkill {
            skill: "ctl-grill-with-spec",
            platform: "opencode",
            path: ".opencode/skills/ctl-grill-with-spec/SKILL.md",
            platform_marker: "opencode Integration",
        },
        WorkflowSkill {
            skill: "ctl-to-prd",
            platform: "omp",
            path: ".omp/skills/ctl-to-prd/SKILL.md",
            platform_marker: "OMP Integration",
        },
        WorkflowSkill {
            skill: "ctl-to-prd",
            platform: "opencode",
            path: ".opencode/skills/ctl-to-prd/SKILL.md",
            platform_marker: "opencode Integration",
        },
        WorkflowSkill {
            skill: "ctl-to-tasks",
            platform: "omp",
            path: ".omp/skills/ctl-to-tasks/SKILL.md",
            platform_marker: "OMP Integration",
        },
        WorkflowSkill {
            skill: "ctl-to-tasks",
            platform: "opencode",
            path: ".opencode/skills/ctl-to-tasks/SKILL.md",
            platform_marker: "opencode Integration",
        },
        WorkflowSkill {
            skill: "ctl-tdd-loop",
            platform: "omp",
            path: ".omp/skills/ctl-tdd-loop/SKILL.md",
            platform_marker: "OMP Integration",
        },
        WorkflowSkill {
            skill: "ctl-tdd-loop",
            platform: "opencode",
            path: ".opencode/skills/ctl-tdd-loop/SKILL.md",
            platform_marker: "opencode Integration",
        },
        WorkflowSkill {
            skill: "ctl-handoff",
            platform: "omp",
            path: ".omp/skills/ctl-handoff/SKILL.md",
            platform_marker: "OMP Integration",
        },
        WorkflowSkill {
            skill: "ctl-handoff",
            platform: "opencode",
            path: ".opencode/skills/ctl-handoff/SKILL.md",
            platform_marker: "opencode Integration",
        },
        WorkflowSkill {
            skill: "ctl-architecture-review",
            platform: "omp",
            path: ".omp/skills/ctl-architecture-review/SKILL.md",
            platform_marker: "OMP Integration",
        },
        WorkflowSkill {
            skill: "ctl-architecture-review",
            platform: "opencode",
            path: ".opencode/skills/ctl-architecture-review/SKILL.md",
            platform_marker: "opencode Integration",
        },
    ]
}

/// Extract `(version, normalized_core)` from a skill's managed **workflow**-core
/// block. Same contract as [`extract_managed_core`], different markers.
pub fn extract_workflow_core(skill: &str) -> Result<(String, String)> {
    extract_core_block(skill, WORKFLOW_CORE_START_PREFIX, WORKFLOW_CORE_END_MARKER)
}

/// The shared body of a workflow skill: everything between the end of the managed
/// core and the start of the platform integration section. This is the
/// phase-specific instruction text that must be IDENTICAL across a skill's two
/// platform files (only the core is platform-neutral; the phase body is too, and
/// is kept in sync by construction + the parity test). Returns the normalized
/// text, or the whole post-core remainder if no platform heading is found.
pub fn workflow_phase_body(skill: &str) -> Result<String> {
    let after = skill
        .split(WORKFLOW_CORE_END_MARKER)
        .nth(1)
        .ok_or_else(|| anyhow!("workflow-core end marker not found"))?;
    // Cut at whichever platform integration heading is present. These exact
    // headings are emitted by every workflow skill.
    let cut = ["\n## OMP Integration", "\n## opencode Integration"]
        .iter()
        .filter_map(|h| after.find(h))
        .min()
        .unwrap_or(after.len());
    Ok(normalize_protocol(&after[..cut]))
}

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
    extract_core_block(skill, CORE_START_PREFIX, CORE_END_MARKER)
}

/// Extract `(version, normalized_core)` from a managed block delimited by the
/// given `start_prefix` / `end_marker`. The single parse/normalize primitive
/// shared by both the control-guard and workflow protocol checks (and their
/// runtime evaluators), so the two families cannot drift in how they read a
/// block. Errors unless exactly one well-formed block exists — catching missing,
/// duplicate, or one-sided markers, and an unparseable version.
fn extract_core_block(
    skill: &str,
    start_prefix: &str,
    end_marker: &str,
) -> Result<(String, String)> {
    let starts = skill.matches(start_prefix).count();
    let ends = skill.matches(end_marker).count();
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
    let start_idx = skill.find(start_prefix).unwrap();
    let end_idx = skill.find(end_marker).unwrap();
    if start_idx >= end_idx {
        return Err(anyhow!("managed-core end marker precedes start marker"));
    }
    let after = &skill[start_idx + start_prefix.len()..];
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
    let body_start = start_idx + start_prefix.len() + line_len + 1;
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

// ── Managed workflow-skills protocol: drift test ─────────────────────────────
//
// The workflow foundation (workflow-skills-foundation-v1) embeds ONE canonical
// workflow-core (`.agent/protocols/workflow-skills.md`) verbatim inside every
// workflow skill across BOTH platforms. These tests refuse to let the copies, or
// the platform-shared phase bodies, diverge — the same discipline as
// `control_guard_protocol_sync`, reusing the same parse/normalize primitives.
#[cfg(test)]
mod workflow_protocol_sync {
    use super::{
        all_embedded_files, extract_workflow_core, normalize_protocol, workflow_phase_body,
        workflow_skills, WORKFLOW_CANONICAL_PROTOCOL_PATH,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// Platform mechanics that must NEVER appear in the shared workflow core.
    const PLATFORM_TOKENS: &[&str] = &[
        "tool.execute.before",
        "experimental.chat.system.transform",
        ".opencode/plugins",
        ".omp/hooks",
        "PreToolUse",
        "ctl-gate.ts",
    ];

    fn manifest_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn read(rel: &str) -> String {
        let p = manifest_root().join(rel);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("missing {rel}: {e}"))
    }

    /// (1) The canonical workflow protocol source exists and declares its version.
    #[test]
    fn canonical_workflow_protocol_exists_and_declares_version() {
        let canon = read(WORKFLOW_CANONICAL_PROTOCOL_PATH);
        assert!(
            canon.contains("WORKFLOW_PROTOCOL_VERSION = 1"),
            "canonical workflow protocol must declare WORKFLOW_PROTOCOL_VERSION = 1"
        );
    }

    /// (2)/(3) Both platforms exist for every logical workflow skill, and the OMP
    /// copy is shipped by `ctl init` (embedded).
    #[test]
    fn both_platforms_present_and_omp_is_shipped() {
        let mut by_skill: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for ws in workflow_skills() {
            by_skill.entry(ws.skill).or_default().push(ws.platform);
        }
        assert!(!by_skill.is_empty(), "expected workflow skills");
        for (skill, platforms) in &by_skill {
            assert!(platforms.contains(&"omp"), "{skill}: missing OMP skill");
            assert!(
                platforms.contains(&"opencode"),
                "{skill}: missing OpenCode skill"
            );
        }

        let embedded: Vec<&str> = all_embedded_files()
            .iter()
            .map(|f| f.relative_path)
            .collect();
        for ws in workflow_skills().iter().filter(|w| w.platform == "omp") {
            let rel = ws.path.strip_prefix(".omp/").unwrap();
            assert!(
                embedded.contains(&rel),
                "ctl init must ship OMP workflow skill {rel}"
            );
        }
    }

    /// (3)/(4) The managed core in every workflow skill equals the canonical
    /// source exactly, all declare the same version, and that version is declared
    /// canonically. Editing the canonical without re-syncing a skill fails here.
    #[test]
    fn managed_core_identical_across_canonical_and_all_skills() {
        let canonical = normalize_protocol(&read(WORKFLOW_CANONICAL_PROTOCOL_PATH));
        let mut prev_version: Option<String> = None;
        for ws in workflow_skills() {
            let skill = read(ws.path);
            let (version, core) =
                extract_workflow_core(&skill).unwrap_or_else(|e| panic!("{}: {e}", ws.path));
            assert_eq!(
                core, canonical,
                "{}: managed workflow core drifted from {WORKFLOW_CANONICAL_PROTOCOL_PATH} — re-sync the skill",
                ws.path
            );
            if let Some(prev) = &prev_version {
                assert_eq!(
                    &version, prev,
                    "{}: protocol version disagrees with another skill",
                    ws.path
                );
            }
            prev_version = Some(version.clone());
            assert!(
                canonical.contains(&format!("WORKFLOW_PROTOCOL_VERSION = {version}")),
                "{}: canonical must declare WORKFLOW_PROTOCOL_VERSION = {version}",
                ws.path
            );
            // (6) the platform marker appears OUTSIDE the managed core.
            assert!(
                skill.contains(ws.platform_marker),
                "{}: must reference its platform marker '{}'",
                ws.path,
                ws.platform_marker
            );
            assert!(
                !core.contains(ws.platform_marker),
                "{}: platform marker '{}' leaked into the managed core",
                ws.path,
                ws.platform_marker
            );
        }
        assert!(
            prev_version.is_some(),
            "expected at least one workflow skill"
        );
    }

    /// (6) No platform-specific mechanic leaks into the shared canonical core.
    /// Since every skill's core is asserted equal to canonical, a clean canonical
    /// proves a clean core in all skills.
    #[test]
    fn no_platform_token_leaks_into_canonical_core() {
        let canonical = normalize_protocol(&read(WORKFLOW_CANONICAL_PROTOCOL_PATH));
        for tok in PLATFORM_TOKENS {
            assert!(
                !canonical.contains(tok),
                "platform token '{tok}' must not appear in the canonical workflow core"
            );
        }
    }

    /// The platform-shared phase body (everything between the managed core and the
    /// platform integration section) must be IDENTICAL across a skill's OMP and
    /// OpenCode files. Editing one platform's body without the other fails here —
    /// this is what keeps the *semantic* workflow in sync, not just the core.
    #[test]
    fn phase_body_is_identical_across_platforms() {
        let mut bodies: BTreeMap<&str, BTreeMap<&str, String>> = BTreeMap::new();
        for ws in workflow_skills() {
            let body =
                workflow_phase_body(&read(ws.path)).unwrap_or_else(|e| panic!("{}: {e}", ws.path));
            bodies
                .entry(ws.skill)
                .or_default()
                .insert(ws.platform, body);
        }
        for (skill, per_platform) in bodies {
            let omp = per_platform.get("omp").expect("omp body");
            let oc = per_platform.get("opencode").expect("opencode body");
            assert_eq!(
                omp, oc,
                "{skill}: phase body drifted between OMP and OpenCode"
            );
            assert!(!omp.is_empty(), "{skill}: phase body is empty");
        }
    }

    /// (5) Marker corruption is detected: missing or one-sided markers error.
    #[test]
    fn extract_workflow_core_rejects_corrupt_markers() {
        assert!(extract_workflow_core("no markers here").is_err());
        assert!(
            extract_workflow_core("<!-- ctl:workflow-core:start version=1 -->\nbody, no end")
                .is_err()
        );
        assert!(extract_workflow_core("only an end <!-- ctl:workflow-core:end -->").is_err());
    }

    /// Provenance guard: ctl adapts ideas, it never vendors third-party skill
    /// trees as an active control plane. No source-named skill dir may exist.
    #[test]
    fn no_third_party_skills_vendored() {
        for forbidden in [
            ".omp/skills/mattpocock",
            ".opencode/skills/mattpocock",
            ".trellis",
            "vendor/skills",
        ] {
            assert!(
                !manifest_root().join(forbidden).exists(),
                "third-party skills must not be vendored: {forbidden}"
            );
        }
    }

    /// The L0 external-reference status is recorded in the protocol and NOTICE.
    #[test]
    fn docs_record_l0_external_reference_status() {
        let canon = read(WORKFLOW_CANONICAL_PROTOCOL_PATH);
        assert!(
            canon.contains("L0 reference"),
            "canonical protocol must record L0 external-reference status"
        );
        let notice = read(".omp/skills/NOTICE.md");
        assert!(
            notice.contains("Pocock"),
            "NOTICE must record the workflow-skills provenance (Matt Pocock / Trellis PR #335)"
        );
    }
}
