//! Embedded control-plane skills and hooks, compiled into the binary via `include_str!`.
//! `cmd_init` writes these into the target project's `.omp/` directory
//! so the AI model automatically loads the control plane on every session.

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

/// Deterministic drift check for the control-guard protocol: the canonical
/// source and the managed-core block embedded in every platform skill must be
/// byte-identical (normalized) and declare the same version. Drift fails CI
/// (this runs under `cargo test`). There is no generator — skills are authored
/// directly; this only refuses to let them diverge. Adding a platform means
/// adding its skill to `PLATFORM_SKILLS` below.
#[cfg(test)]
mod control_guard_protocol_sync {
    use std::path::PathBuf;

    const CANONICAL: &str = ".agent/protocols/control-guard.md";
    /// (label, skill path, the platform entry point it must reference outside the core).
    const PLATFORM_SKILLS: &[(&str, &str, &str)] = &[
        (
            "OMP",
            ".omp/skills/control-guard/SKILL.md",
            ".omp/hooks/pre/ctl-context.ts",
        ),
        (
            "opencode",
            ".opencode/skills/control-guard/SKILL.md",
            ".opencode/plugins/ctl-gate.ts",
        ),
    ];

    const START_PREFIX: &str = "<!-- ctl:control-guard-core:start version=";
    const END_MARKER: &str = "<!-- ctl:control-guard-core:end -->";

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

    fn read(rel: &str) -> String {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("missing {rel}: {e}"))
    }

    /// LF endings, trailing per-line whitespace stripped, leading/trailing blank
    /// lines trimmed — tolerant of insertion whitespace, strict on content.
    fn normalize(s: &str) -> String {
        s.replace("\r\n", "\n")
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
            .trim_matches('\n')
            .to_string()
    }

    /// Extract (version, normalized core). Panics unless exactly one well-formed
    /// managed block exists — catching missing, duplicate, or one-sided markers.
    fn extract_core(skill: &str, label: &str) -> (String, String) {
        let starts = skill.matches(START_PREFIX).count();
        let ends = skill.matches(END_MARKER).count();
        assert_eq!(
            starts, 1,
            "{label}: expected exactly one start marker, found {starts}"
        );
        assert_eq!(
            ends, 1,
            "{label}: expected exactly one end marker, found {ends}"
        );

        let start_idx = skill.find(START_PREFIX).unwrap();
        let end_idx = skill.find(END_MARKER).unwrap();
        assert!(
            start_idx < end_idx,
            "{label}: end marker precedes start marker"
        );

        let after = &skill[start_idx + START_PREFIX.len()..];
        let line_len = after.find('\n').unwrap_or(after.len());
        let version = after[..line_len]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches("-->")
            .trim()
            .to_string();
        assert!(
            !version.is_empty(),
            "{label}: could not parse version from start marker"
        );

        let body_start = start_idx + START_PREFIX.len() + line_len + 1;
        (version, normalize(&skill[body_start..end_idx]))
    }

    #[test]
    fn managed_core_is_identical_across_canonical_and_all_skills() {
        let canonical = normalize(&read(CANONICAL));

        let mut prev_version: Option<String> = None;
        for (label, path, entry_point) in PLATFORM_SKILLS {
            let skill = read(path);
            let (version, core) = extract_core(&skill, label);

            // (3)/(9) the core must equal the canonical source exactly. If the
            // canonical changes but a skill isn't re-synced, this fails.
            assert_eq!(
                core, canonical,
                "{label}: managed core drifted from {CANONICAL} — re-sync the skill"
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
}
