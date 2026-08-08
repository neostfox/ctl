//! Single-source generation for the ctl skills.
//!
//! Each generated skill has ONE source at `.agent/skills/<skill>/source.md`:
//! frontmatter + an optional shared phase **body** + optional per-platform
//! **integration** sections delimited by `<!-- integration:<platform> -->`.
//! `ctl skills sync` composes each platform's `SKILL.md` from:
//!
//! ```text
//! <frontmatter>            (shared, from source.md)
//! # <title> (<Label>)      (generated preamble)
//! [<managed core>]         (canonical .agent/protocols/<core>.md, wrapped — core skills only)
//! [<reference note>]       (only for cores with a reference part)
//! [<phase body>]           (shared, from source.md; empty for control-guard)
//! [## <Label> Integration] (per-platform, from source.md; absent for plain skills)
//! ```
//!
//! A skill belongs to a **family** that names an optional managed **core**:
//! - `workflow` — grill/PRD/tasks, embedding the workflow-core (with a reference part).
//! - `control-guard` — the governance core (no reference part).
//! - `plain` — no core (ctl-spec, ctl-cognitive, ctl-meta); the source body is the
//!   whole skill, generated per platform.
//!
//! **Not every skill is generated.** ctl-review and ctl-diagnose are
//! single-platform (OMP), core-less skills with no cross-platform divergence, so
//! generation would be pure indirection — they are hand-authored directly under
//! `.omp/skills/`. Generation pays for itself only where a managed core must stay
//! in sync across copies or platforms diverge.
//!
//! The managed core stays byte-identical to the canonical protocol and the body
//! stays identical across platforms BY CONSTRUCTION (one source). `--check`
//! re-derives and fails on any on-disk divergence (the generated files are
//! committed).

use anyhow::{anyhow, Result};
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

/// Every platform key, for skills that ship on all of them.
const ALL_KEYS: &[&str] = &["omp", "opencode", "claude"];

/// The spec for a managed core: the canonical protocol a core skill embeds
/// verbatim, its marker pair, version declaration, optional reference-part
/// marker, and the noun used in the generated preamble. Plain skills have no
/// core (`Family::core` is `None`). Adding a new managed core means adding a
/// `CoreSpec` + `Family` here; the drift tests in `skills.rs` independently
/// re-assert an embedded core equals canonical.
struct CoreSpec {
    canonical_path: &'static str,
    start_prefix: &'static str,
    end_marker: &'static str,
    version_decl: &'static str,
    reference_marker: Option<&'static str>,
    protocol_noun: &'static str,
}

/// A family groups skills by which core (if any) they embed.
struct Family {
    name: &'static str,
    core: Option<&'static CoreSpec>,
}

const WORKFLOW_CORE: CoreSpec = CoreSpec {
    canonical_path: ".agent/protocols/workflow-skills.md",
    start_prefix: "<!-- ctl:workflow-core:start version=",
    end_marker: "<!-- ctl:workflow-core:end -->",
    version_decl: "WORKFLOW_PROTOCOL_VERSION",
    reference_marker: Some("<!-- ctl:workflow-core-reference:start -->"),
    protocol_noun: "ctl workflow protocol",
};

const CONTROL_GUARD_CORE: CoreSpec = CoreSpec {
    canonical_path: ".agent/protocols/control-guard.md",
    start_prefix: "<!-- ctl:control-guard-core:start version=",
    end_marker: "<!-- ctl:control-guard-core:end -->",
    version_decl: "CONTROL_GUARD_PROTOCOL_VERSION",
    reference_marker: None,
    protocol_noun: "control-guard protocol",
};

const WORKFLOW_FAMILY: Family = Family {
    name: "workflow",
    core: Some(&WORKFLOW_CORE),
};

const CONTROL_GUARD_FAMILY: Family = Family {
    name: "control-guard",
    core: Some(&CONTROL_GUARD_CORE),
};

const PLAIN_FAMILY: Family = Family {
    name: "plain",
    core: None,
};

/// A skill generated from a single `.agent/skills/<name>/source.md`. `title` is
/// the generated H1; `platforms` are the platform keys it ships to.
struct SkillSpec {
    name: &'static str,
    title: &'static str,
    family: &'static Family,
    platforms: &'static [&'static str],
}

/// Every skill generated from source. Adding a skill means adding a row here.
/// `platforms` preserves the intentional cross-platform asymmetry (e.g.
/// ctl-cognitive omits opencode). ctl-review/ctl-diagnose are intentionally
/// absent — hand-authored OMP-only skills (see the module doc).
fn generated_skills() -> &'static [SkillSpec] {
    &[
        SkillSpec {
            name: "control-guard",
            title: "Control Guard",
            family: &CONTROL_GUARD_FAMILY,
            platforms: ALL_KEYS,
        },
        SkillSpec {
            name: "ctl-grill-with-spec",
            title: "ctl-grill-with-spec",
            family: &WORKFLOW_FAMILY,
            platforms: ALL_KEYS,
        },
        SkillSpec {
            name: "ctl-to-prd",
            title: "ctl-to-prd",
            family: &WORKFLOW_FAMILY,
            platforms: ALL_KEYS,
        },
        SkillSpec {
            name: "ctl-to-tasks",
            title: "ctl-to-tasks",
            family: &WORKFLOW_FAMILY,
            platforms: ALL_KEYS,
        },
        SkillSpec {
            name: "ctl-spec",
            title: "ctl-spec",
            family: &PLAIN_FAMILY,
            platforms: ALL_KEYS,
        },
        SkillSpec {
            name: "ctl-cognitive",
            title: "ctl-cognitive",
            family: &PLAIN_FAMILY,
            platforms: &["omp", "claude"],
        },
        SkillSpec {
            name: "ctl-meta",
            title: "ctl-meta",
            family: &PLAIN_FAMILY,
            platforms: ALL_KEYS,
        },
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

    // Split off the integration sections. Plain skills have none — the whole
    // post-frontmatter text is the body.
    let marker = "<!-- integration:";
    let (body, integrations) = match after_fm.find(marker) {
        Some(i) => {
            let body = after_fm[..i].trim().to_string();
            let mut integrations = std::collections::HashMap::new();
            for chunk in after_fm.split(marker).skip(1) {
                let close = chunk
                    .find("-->")
                    .ok_or_else(|| anyhow!("malformed integration marker"))?;
                let key = chunk[..close].trim().to_string();
                let content = chunk[close + 3..].trim().to_string();
                integrations.insert(key, content);
            }
            (body, integrations)
        }
        None => (after_fm.trim().to_string(), std::collections::HashMap::new()),
    };
    Ok(Source {
        frontmatter,
        body,
        integrations,
    })
}

/// Read a core's canonical content and its declared version. For a core with a
/// reference marker, the embedded core is everything before it (the phase map /
/// frameworks / provenance after it are reference-only); otherwise the whole
/// canonical file is the core.
fn read_core(project_root: &Path, spec: &CoreSpec) -> Result<(String, String)> {
    let raw = std::fs::read_to_string(project_root.join(spec.canonical_path))
        .map_err(|e| anyhow!("canonical protocol unreadable: {e}"))?
        .replace("\r\n", "\n");
    let version = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix(&format!("{} = ", spec.version_decl)))
        .ok_or_else(|| anyhow!("canonical does not declare {}", spec.version_decl))?
        .trim()
        .to_string();
    let embedded = match spec.reference_marker {
        Some(m) => raw
            .split_once(m)
            .map(|(embedded, _)| embedded)
            .ok_or_else(|| anyhow!("canonical core missing reference marker {m}"))?
            .trim_end()
            .to_string(),
        None => raw.trim_end().to_string(),
    };
    Ok((embedded, version))
}

/// A managed core read from disk, bundled with its spec for composition.
struct RenderedCore<'a> {
    spec: &'a CoreSpec,
    content: String,
    version: String,
}

/// Compose one platform's SKILL.md text. Deterministic; the single source of
/// truth for what a generated skill file looks like. `core` is `None` for plain
/// (core-less) families, which emit no managed-core block.
fn compose(
    src: &Source,
    spec: &SkillSpec,
    platform: &GenPlatform,
    core: Option<&RenderedCore>,
) -> String {
    let integration = src
        .integrations
        .get(platform.key)
        .map(String::as_str)
        .unwrap_or("");
    let mut out = String::new();
    out.push_str(&src.frontmatter);
    out.push('\n');
    out.push_str(&format!("# {} ({})\n\n", spec.title, platform.label));
    if let Some(rc) = core {
        let cs = rc.spec;
        out.push_str(&format!(
            "The **managed core** below is the platform-neutral {}, byte-checked by CI \
             against `{}` across platforms. Do not edit it here — it is generated from \
             `.agent/skills/{}/source.md` by `ctl skills sync`. {}-specific mechanics live \
             after the core.\n\n",
            cs.protocol_noun, cs.canonical_path, spec.name, platform.label
        ));
        out.push_str(&format!("{}{} -->\n", cs.start_prefix, rc.version));
        out.push_str(&rc.content);
        out.push('\n');
        out.push_str(cs.end_marker);
        out.push_str("\n\n");
        if cs.reference_marker.is_some() {
            out.push_str(&format!(
                "*The phase map, frameworks, and provenance are reference material in `{}` \
                 — not embedded here. The auto-loaded control-guard carries the pipeline \
                 routing; this skill's body covers its own phase.*\n\n",
                cs.canonical_path
            ));
        }
    }
    if !src.body.is_empty() {
        out.push_str(&src.body);
        out.push_str("\n\n");
    }
    if !integration.is_empty() {
        out.push_str(&format!(
            "## {} Integration (platform-specific)\n\n",
            platform.label
        ));
        out.push_str(integration);
        out.push('\n');
    }
    out
}

/// One difference found by `sync`/`check`.
pub struct SyncOutcome {
    pub written: Vec<String>,
    pub stale: Vec<String>,
}

/// Generate every generated skill's `SKILL.md` (and any `references/` files)
/// for its declared platforms from its single `.agent/skills/<skill>/source.md`.
/// With `check = true`, write nothing and report which files are out of date.
pub fn sync(project_root: &Path, check: bool) -> Result<SyncOutcome> {
    let mut written = Vec::new();
    let mut stale = Vec::new();
    // Cache (content, version) by canonical path — one file read per managed core.
    let mut core_cache: std::collections::HashMap<&'static str, (String, String)> =
        std::collections::HashMap::new();
    for spec in generated_skills() {
        let rendered: Option<RenderedCore<'_>> = match spec.family.core {
            None => None,
            Some(cs) => {
                let (content, version) = if let Some(v) =
                    core_cache.get(cs.canonical_path).cloned()
                {
                    v
                } else {
                    let v = read_core(project_root, cs)?;
                    core_cache.insert(cs.canonical_path, v.clone());
                    v
                };
                Some(RenderedCore {
                    spec: cs,
                    content,
                    version,
                })
            }
        };
        let src_path = project_root.join(format!(".agent/skills/{}/source.md", spec.name));
        let raw = std::fs::read_to_string(&src_path)
            .map_err(|e| anyhow!("{}: {e}", src_path.display()))?;
        let src = parse_source(&raw).map_err(|e| anyhow!("{}: {e}", spec.name))?;

        // Progressive-disclosure references: `.agent/skills/<name>/references/*.md`
        // are copied verbatim to each platform skill dir. Supplementary depth,
        // loaded on demand; the SKILL.md stays correct without them. Designed to
        // be dropped wholesale once models no longer need the context economy.
        let refs_dir = project_root.join(format!(".agent/skills/{}/references", spec.name));
        let mut ref_names: Vec<String> = Vec::new();
        if refs_dir.is_dir() {
            for entry in std::fs::read_dir(&refs_dir)? {
                let path = entry?.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    ref_names.push(path.file_name().unwrap().to_string_lossy().into_owned());
                }
            }
            ref_names.sort();
        }

        for platform in PLATFORMS {
            if !spec.platforms.contains(&platform.key) {
                continue;
            }
            let composed = compose(&src, spec, platform, rendered.as_ref());
            sync_one(
                project_root,
                &format!("{}/{}/SKILL.md", platform.dir, spec.name),
                &composed,
                check,
                &mut written,
                &mut stale,
            )?;
            for rname in &ref_names {
                let ref_src =
                    project_root.join(format!(".agent/skills/{}/references/{}", spec.name, rname));
                let content = std::fs::read_to_string(&ref_src)
                    .map_err(|e| anyhow!("{}: {e}", ref_src.display()))?
                    .replace("\r\n", "\n");
                sync_one(
                    project_root,
                    &format!("{}/{}/references/{}", platform.dir, spec.name, rname),
                    &content,
                    check,
                    &mut written,
                    &mut stale,
                )?;
            }
        }
    }
    Ok(SyncOutcome { written, stale })
}

/// Compare `content` to the file at `rel` (CRLF-normalized); on mismatch, record
/// it as stale (`check`) or write it. The single write/check primitive for
/// `sync`, reused for both SKILL.md and references.
fn sync_one(
    project_root: &Path,
    rel: &str,
    content: &str,
    check: bool,
    written: &mut Vec<String>,
    stale: &mut Vec<String>,
) -> Result<()> {
    let dest = project_root.join(rel);
    let current = std::fs::read_to_string(&dest)
        .map(|s| s.replace("\r\n", "\n"))
        .unwrap_or_default();
    if current == content {
        return Ok(());
    }
    if check {
        stale.push(rel.to_string());
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, content)?;
        written.push(rel.to_string());
    }
    Ok(())
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
    fn parse_source_allows_no_integration_sections() {
        // A plain skill: whole post-frontmatter text is the body.
        let text = "---\nname: x\ndescription: \"d\"\n---\n\nwhole body\nmore body\n";
        let s = parse_source(text).unwrap();
        assert!(s.integrations.is_empty());
        assert_eq!(s.body, "whole body\nmore body");
    }

    #[test]
    fn every_generated_skill_is_in_sync_on_disk() {
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

    #[test]
    fn generated_skills_exclude_the_hand_authored_omp_only_skills() {
        // ctl-review/ctl-diagnose are hand-authored (OMP-only, no core, no
        // divergence) — they must NOT be in the generated set.
        let names: Vec<&str> = generated_skills().iter().map(|s| s.name).collect();
        for required in [
            "control-guard",
            "ctl-grill-with-spec",
            "ctl-to-prd",
            "ctl-to-tasks",
            "ctl-spec",
            "ctl-cognitive",
            "ctl-meta",
        ] {
            assert!(names.contains(&required), "{required} must be generated");
        }
        for hand_authored in ["ctl-review", "ctl-diagnose"] {
            assert!(
                !names.contains(&hand_authored),
                "{hand_authored} is hand-authored, not generated"
            );
        }
        // Plain families carry no core; core families do.
        assert!(PLAIN_FAMILY.core.is_none());
        assert!(WORKFLOW_FAMILY.core.is_some());
        assert!(CONTROL_GUARD_FAMILY.core.is_some());
        // Platform asymmetry is intentional and preserved.
        let cognitive = generated_skills().iter().find(|s| s.name == "ctl-cognitive").unwrap();
        assert_eq!(cognitive.platforms, &["omp", "claude"]);
    }

    #[test]
    fn progressive_disclosure_references_are_synced() {
        // References under .agent/skills/<name>/references/ are copied verbatim
        // to each platform the skill ships to. Locks the pattern so it cannot
        // silently regress (and so the drop-block stays clean to remove later).
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for (skill, rname) in [
            ("control-guard", "command-reference.md"),
            ("ctl-grill-with-spec", "first-principles.md"),
        ] {
            let src = std::fs::read_to_string(
                root.join(format!(".agent/skills/{skill}/references/{rname}")),
            )
            .unwrap()
            .replace("\r\n", "\n");
            for dir in [".omp/skills", ".claude/skills", ".opencode/skills"] {
                let dest = root.join(format!("{dir}/{skill}/references/{rname}"));
                let on_disk =
                    std::fs::read_to_string(&dest).unwrap().replace("\r\n", "\n");
                assert_eq!(on_disk, src, "{dest:?} drifted from source");
            }
        }
    }
}
