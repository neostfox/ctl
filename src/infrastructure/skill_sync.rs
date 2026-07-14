//! Single-source generation for the workflow skills.
//!
//! Each workflow skill has ONE source at `.agent/skills/<skill>/source.md`:
//! frontmatter + the shared phase **body** + per-platform **integration**
//! sections delimited by `<!-- integration:<platform> -->`. `ctl skills sync`
//! composes each platform's `SKILL.md` from:
//!
//! ```text
//! <frontmatter>            (shared, from source.md)
//! # <skill> (<Label>)      (generated preamble)
//! <managed core>           (canonical .agent/protocols/workflow-skills.md, wrapped)
//! <phase body>             (shared, from source.md)
//! ## <Label> Integration … (per-platform, from source.md)
//! ```
//!
//! The managed core stays byte-identical to the canonical protocol and the body
//! stays identical across platforms BY CONSTRUCTION (one source), so the manual
//! mirror + drift-authoring is gone. `--check` re-derives and fails on any
//! on-disk divergence (the generated files are committed).

use anyhow::{anyhow, bail, Result};
use std::path::Path;

/// A target platform for generation.
struct GenPlatform {
    key: &'static str,
    label: &'static str,
    dir: &'static str,
}

const PLATFORMS: &[GenPlatform] = &[
    GenPlatform {
        key: "omp",
        label: "OMP",
        dir: ".omp/skills",
    },
    GenPlatform {
        key: "opencode",
        label: "opencode",
        dir: ".opencode/skills",
    },
    GenPlatform {
        key: "claude",
        label: "Claude Code",
        dir: ".claude/skills",
    },
];

/// Canonical workflow protocol + its managed-core markers.
const CANONICAL_PATH: &str = ".agent/protocols/workflow-skills.md";
const CORE_START_PREFIX: &str = "<!-- ctl:workflow-core:start version=";
const CORE_END_MARKER: &str = "<!-- ctl:workflow-core:end -->";
const VERSION_DECL: &str = "WORKFLOW_PROTOCOL_VERSION";
const REFERENCE_MARKER: &str = "<!-- ctl:workflow-core-reference:start -->";

/// The workflow skills generated from a single source. control-guard is NOT here
/// (its frontmatter is per-platform and it has no phase body — it stays
/// hand-authored under `control_guard_protocol_sync`).
pub fn generated_skills() -> &'static [&'static str] {
    &[
        "ctl-grill-with-spec",
        "ctl-to-prd",
        "ctl-to-tasks",
        "ctl-tdd-loop",
        "ctl-handoff",
    ]
}

/// A parsed `source.md`: shared frontmatter + body, and per-platform integration.
struct Source {
    frontmatter: String,
    body: String,
    integrations: std::collections::HashMap<String, String>,
}

fn parse_source(text: &str) -> Result<Source> {
    let text = text.replace("\r\n", "\n");
    // Frontmatter: leading `---\n ... \n---`.
    let rest = text
        .strip_prefix("---\n")
        .ok_or_else(|| anyhow!("source must start with a `---` frontmatter block"))?;
    let end = rest
        .find("\n---\n")
        .ok_or_else(|| anyhow!("frontmatter is not terminated by a `---` line"))?;
    let frontmatter = format!("---\n{}\n---\n", &rest[..end]);
    let after_fm = &rest[end + 5..];

    // Split off the integration sections.
    let mut integrations = std::collections::HashMap::new();
    let marker = "<!-- integration:";
    let body = match after_fm.find(marker) {
        Some(i) => after_fm[..i].to_string(),
        None => bail!("source has no `<!-- integration:<platform> -->` sections"),
    };
    for chunk in after_fm.split(marker).skip(1) {
        let close = chunk
            .find("-->")
            .ok_or_else(|| anyhow!("malformed integration marker"))?;
        let key = chunk[..close].trim().to_string();
        let content = chunk[close + 3..].to_string();
        integrations.insert(key, content.trim().to_string());
    }
    Ok(Source {
        frontmatter,
        body: body.trim().to_string(),
        integrations,
    })
}

/// Read the canonical core content and its declared version.
fn read_core(project_root: &Path) -> Result<(String, String)> {
    let raw = std::fs::read_to_string(project_root.join(CANONICAL_PATH))
        .map_err(|e| anyhow!("canonical protocol unreadable: {e}"))?
        .replace("\r\n", "\n");
    let version = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix(&format!("{VERSION_DECL} = ")))
        .ok_or_else(|| anyhow!("canonical does not declare {VERSION_DECL}"))?
        .trim()
        .to_string();
    // The embedded core is everything before the reference marker; the phase
    // map, frameworks, and provenance after it are reference-only (the
    // auto-loaded control-guard carries the pipeline; each skill's body covers
    // its own phase).
    let embedded = raw
        .split_once(REFERENCE_MARKER)
        .map(|(embedded, _)| embedded)
        .ok_or_else(|| anyhow!("canonical workflow core missing reference marker"))?
        .trim_end()
        .to_string();
    Ok((embedded, version))
}

/// Compose one platform's SKILL.md text. Deterministic; the single source of
/// truth for what a generated skill file looks like.
fn compose(src: &Source, skill: &str, platform: &GenPlatform, core: &str, version: &str) -> String {
    let integration = src
        .integrations
        .get(platform.key)
        .map(String::as_str)
        .unwrap_or("");
    let mut out = String::new();
    out.push_str(&src.frontmatter);
    out.push('\n');
    out.push_str(&format!("# {skill} ({})\n\n", platform.label));
    out.push_str(&format!(
        "The **managed core** below is the platform-neutral ctl workflow protocol, \
         byte-checked by CI against `{CANONICAL_PATH}` across platforms. Do not edit it \
         here — it is generated from `.agent/skills/{skill}/source.md` by `ctl skills sync`. \
         {}-specific mechanics live after the core.\n\n",
        platform.label
    ));
    out.push_str(&format!("{CORE_START_PREFIX}{version} -->\n"));
    out.push_str(core);
    out.push('\n');
    out.push_str(CORE_END_MARKER);
    out.push_str("\n\n*The phase map, frameworks, and provenance are reference material in `");
    out.push_str(CANONICAL_PATH);
    out.push_str("` — not embedded here. The auto-loaded control-guard carries the pipeline routing; this skill's body covers its own phase.*\n");
    if !src.body.is_empty() {
        out.push('\n');
        out.push_str(&src.body);
        out.push('\n');
    }
    out.push_str(&format!(
        "\n## {} Integration (platform-specific)\n\n",
        platform.label
    ));
    out.push_str(integration);
    out.push('\n');
    out
}

/// One difference found by `sync`/`check`.
pub struct SyncOutcome {
    pub written: Vec<String>,
    pub stale: Vec<String>,
}

/// Generate every workflow skill's `SKILL.md` for every platform. With
/// `check = true`, write nothing and report which files are out of date.
pub fn sync(project_root: &Path, check: bool) -> Result<SyncOutcome> {
    let (core, version) = read_core(project_root)?;
    let mut written = Vec::new();
    let mut stale = Vec::new();
    for skill in generated_skills() {
        let src_path = project_root.join(format!(".agent/skills/{skill}/source.md"));
        let raw = std::fs::read_to_string(&src_path)
            .map_err(|e| anyhow!("{}: {e}", src_path.display()))?;
        let src = parse_source(&raw).map_err(|e| anyhow!("{skill}: {e}"))?;
        for platform in PLATFORMS {
            let composed = compose(&src, skill, platform, &core, &version);
            let rel = format!("{}/{skill}/SKILL.md", platform.dir);
            let dest = project_root.join(&rel);
            let current = std::fs::read_to_string(&dest)
                .map(|s| s.replace("\r\n", "\n"))
                .unwrap_or_default();
            if current == composed {
                continue;
            }
            if check {
                stale.push(rel);
            } else {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&dest, &composed)?;
                written.push(rel);
            }
        }
    }
    Ok(SyncOutcome { written, stale })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_source_splits_frontmatter_body_and_integrations() {
        let text = "---\nname: x\ndescription: \"d\"\n---\n\nbody line\n\n<!-- integration:omp -->\nomp text\n\n<!-- integration:claude -->\nclaude text\n";
        let s = parse_source(text).unwrap();
        assert!(s.frontmatter.starts_with("---\nname: x"));
        assert_eq!(s.body, "body line");
        assert_eq!(s.integrations.get("omp").unwrap(), "omp text");
        assert_eq!(s.integrations.get("claude").unwrap(), "claude text");
    }

    #[test]
    fn every_workflow_skill_is_in_sync_on_disk() {
        // The committed SKILL.md files must equal what `sync --check` derives —
        // i.e. nobody hand-edited a generated file. Mirrors the CI check.
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let outcome = sync(&root, true).expect("sync check runs");
        assert!(
            outcome.stale.is_empty(),
            "generated skills are stale — run `ctl skills sync`: {:?}",
            outcome.stale
        );
    }
}
