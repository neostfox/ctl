//! Embedded control-plane skills and hooks, compiled into the binary via `include_str!`.
//! `cmd_init` writes these into the target project's `.omp/` directory
//! so the AI model automatically loads the control plane on every session.

pub struct EmbeddedFile {
    pub relative_path: &'static str, // e.g. "skills/control-guard/SKILL.md"
    pub content: &'static str,
}

pub fn all_embedded_files() -> Vec<EmbeddedFile> {
    vec![
        // Skills
        EmbeddedFile {
            relative_path: "skills/control-guard/SKILL.md",
            content: include_str!("../../.omp/skills/control-guard/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-new/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-new/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-apply/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-apply/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-close/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-close/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-abort/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-abort/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-health/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-health/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-status/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-status/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-spec-before/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-spec-before/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-spec-update/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-spec-update/SKILL.md"),
        },
        EmbeddedFile {
            relative_path: "skills/ctl-spec-bootstrap/SKILL.md",
            content: include_str!("../../.omp/skills/ctl-spec-bootstrap/SKILL.md"),
        },
        // Hooks (OMP native extension format)
        EmbeddedFile {
            relative_path: "hooks/pre/ctl-context.js",
            content: include_str!("../../.omp/hooks/pre/ctl-context.js"),
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
