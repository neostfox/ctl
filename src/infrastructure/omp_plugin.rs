//! Single-source generation of the publishable `@velo-ai/omp` OMP plugin package.
//!
//! The plugin is assembled from the SAME `.omp/` source that `ctl init` embeds
//! (`skills::all_embedded_files`): the governance hook + skills + spec guides are
//! copied verbatim, and a `package.json` is generated declaring the OMP extension
//! entry (the hook). The package is **pure hooks + skills** — the npm binary
//! distribution was retired (B-lite; see
//! `.ctl/spec/alignment/2026-07-04-binary-distribution-shrink.md`), so `ctl`
//! itself is installed separately (`cargo install` or a GitHub release) and the
//! hook resolves it via the one blessed chain CTL_BIN → `~/.cargo/bin` → PATH.
//!
//! `ctl skills sync` writes the package; `--check` (and the cargo drift test
//! `omp_plugin_package_is_in_sync_on_disk`) re-derive it and fail if the committed
//! `npm-omp/` is stale. Content is sourced from disk (`.omp/<rel>`) at runtime so
//! a stale binary cannot silently emit stale package content.

use crate::infrastructure::skill_sync::SyncOutcome;
use crate::infrastructure::skills::all_embedded_files;
use anyhow::{anyhow, Result};
use std::path::Path;

/// Directory of the committed, generated plugin package.
const PLUGIN_DIR: &str = "npm-omp";
/// npm package name to publish.
const PLUGIN_NAME: &str = "@velo-ai/omp";
/// OMP extension entry point, relative to the package root.
const HOOK_ENTRY: &str = "./hooks/pre/ctl-context.ts";

/// Generated `package.json` (hand-formatted for stable, reviewable output).
/// Pure hooks + skills: no binary dependency — `ctl` is installed separately
/// and resolved via CTL_BIN → `~/.cargo/bin` → PATH.
fn package_json() -> String {
    format!(
        r#"{{
  "name": "{PLUGIN_NAME}",
  "version": "{ver}",
  "description": "OMP plugin for the ctl control plane: governance hook + skills. Pure integration package — install ctl separately (cargo install, or a GitHub release); the hook resolves it via CTL_BIN, ~/.cargo/bin, then PATH.",
  "license": "MIT",
  "repository": {{
    "type": "git",
    "url": "https://github.com/neostfox/ctl"
  }},
  "keywords": [
    "omp",
    "oh-my-pi",
    "ctl",
    "control-plane",
    "governance",
    "plugin"
  ],
  "omp": {{
    "extensions": [
      "{HOOK_ENTRY}"
    ]
  }},
  "files": [
    "hooks/",
    "skills/",
    "spec/",
    "README.md"
  ]
}}
"#,
        ver = env!("CARGO_PKG_VERSION"),
    )
}

/// Generated `README.md` (static — version lives in `package.json`).
fn readme() -> String {
    format!(
        r#"# {PLUGIN_NAME}

OMP plugin for the **ctl** control plane: the governance pre-hook and the
control-plane skills. Pure integration package — it does **not** bundle the
`ctl` binary.

> **Generated file — do not edit.** This package is produced from the canonical
> `.omp/` source by `ctl skills sync`. Edit `.omp/` (and the generator in
> `src/infrastructure/omp_plugin.rs`) instead; CI fails if `{PLUGIN_DIR}/` drifts.

## Install

1. Install `ctl` itself: `cargo install --path .` from the ctl repo, or download
   a binary from the GitHub releases page. The hook resolves it via the one
   blessed chain **CTL_BIN → `~/.cargo/bin` → PATH** — set `CTL_BIN` to pin a
   specific binary. `ctl init --platform omp` pins `CTL_BIN` into
   `~/.omp/agent/.env` automatically (OMP merges that file into its process
   env at startup), so resolution does not depend on which shell launched
   `omp`.
2. Install the plugin. The extension hook only loads for **npm-installed** or
   **linked** plugins (not for `omp plugin install github:…` marketplace
   installs):

```sh
# Local development against this repo's generated package:
omp plugin link ./{PLUGIN_DIR}

# Or, once published:
npm i {PLUGIN_NAME}
```
"#,
    )
}

/// The full set of generated target files, as `(relative_path, expected_content)`
/// pairs. Copied files take content from `.omp/<rel>`; generated files use the
/// templates above. Returned in a deterministic order.
fn targets(project_root: &Path) -> Result<Vec<(String, String)>> {
    let omp_dir = project_root.join(".omp");
    let mut out = Vec::new();

    // Copied verbatim from the canonical .omp/ source (single source of truth).
    for file in all_embedded_files() {
        let rel = file.relative_path; // e.g. "hooks/pre/ctl-context.ts"
        let src = omp_dir.join(rel);
        let content = std::fs::read_to_string(&src)
            .map_err(|e| anyhow!("plugin source unreadable ({}): {e}", src.display()))?
            .replace("\r\n", "\n");
        out.push((rel.to_string(), content));
    }

    // Generated manifest + readme.
    out.push(("package.json".to_string(), package_json()));
    out.push(("README.md".to_string(), readme()));

    Ok(out)
}

/// Generate the `npm-omp/` plugin package from `.omp/`. With `check = true`, write
/// nothing and report which files are out of date (the CI/drift contract). Mirrors
/// `skill_sync::sync` and returns the same `SyncOutcome`.
pub fn sync(project_root: &Path, check: bool) -> Result<SyncOutcome> {
    let plugin_dir = project_root.join(PLUGIN_DIR);
    let mut written = Vec::new();
    let mut stale = Vec::new();

    for (rel, expected) in targets(project_root)? {
        let dest = plugin_dir.join(&rel);
        let current = std::fs::read_to_string(&dest)
            .map(|s| s.replace("\r\n", "\n"))
            .unwrap_or_default();
        if current == expected {
            continue;
        }
        let label = format!("{PLUGIN_DIR}/{rel}");
        if check {
            stale.push(label);
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, &expected)?;
            written.push(label);
        }
    }

    Ok(SyncOutcome { written, stale })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_json_declares_extension_and_no_binary_dependency() {
        let pkg = package_json();
        assert!(pkg.contains("\"name\": \"@velo-ai/omp\""));
        assert!(
            pkg.contains(HOOK_ENTRY),
            "omp.extensions points at the hook"
        );
        // B-lite: pure hooks+skills — the npm binary distribution is retired,
        // so the plugin must NOT depend on any binary package.
        assert!(
            !pkg.contains("\"dependencies\""),
            "no dependency block at all"
        );
        assert!(!pkg.contains("@velo-ai/ctl"), "no binary dependency");
        let ver = env!("CARGO_PKG_VERSION");
        assert_eq!(pkg.matches(ver).count(), 1, "package version only");
    }

    #[test]
    fn targets_include_the_hook_and_manifest() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let rels: Vec<String> = targets(&root)
            .unwrap()
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        assert!(rels.iter().any(|r| r == "hooks/pre/ctl-context.ts"));
        assert!(rels.iter().any(|r| r == "package.json"));
        assert!(rels.iter().any(|r| r == "README.md"));
    }

    #[test]
    fn omp_plugin_package_is_in_sync_on_disk() {
        // The committed npm-omp/ must equal what `sync --check` derives — i.e.
        // nobody edited a generated file and forgot to run `ctl skills sync`.
        // Mirrors `skill_sync::tests::every_workflow_skill_is_in_sync_on_disk`.
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let outcome = sync(&root, true).expect("plugin sync check runs");
        assert!(
            outcome.stale.is_empty(),
            "npm-omp/ is stale — run `ctl skills sync`: {:?}",
            outcome.stale
        );
    }
}
