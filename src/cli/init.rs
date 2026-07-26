use super::*;

pub(super) fn cmd_project_update(force: bool, skip: bool, dry_run: bool) -> Result<()> {
    let project_root = std::env::current_dir()?;
    let summary = crate::infrastructure::project_update::update_project(
        &project_root,
        crate::infrastructure::project_update::UpdateOptions {
            force,
            skip_conflicts: skip,
            dry_run,
        },
    )?;
    if dry_run {
        println!("[dry-run] Project update preview:");
    } else {
        println!("Project update complete:");
    }
    println!(
        "  added: {} · updated: {} · unchanged: {} · conflicts: {} · skipped: {}",
        summary.added, summary.updated, summary.unchanged, summary.conflicts, summary.skipped
    );
    for file in summary.files {
        println!("  {}  {}", file.action, file.path);
    }
    if summary.conflicts > 0 && !force && !skip {
        println!("Conflicts were preserved; review `.new` files or rerun with --force/--skip.");
    }
    Ok(())
}

pub(super) fn cmd_init(
    explicit: &[PlatformArg],
    claude: bool,
    opencode: bool,
    omp: bool,
    all: bool,
    yes: bool,
    dry_run: bool,
) -> Result<()> {
    use std::io::IsTerminal;
    let project_root = std::env::current_dir()?;

    // Explicit flags win. In scripted mode, reuse detected integrations; a fresh
    // project defaults to all supported platforms so onboarding is complete.
    let selection = match resolve_platform_selection(explicit, claude, opencode, omp, all) {
        Ok(selection) => selection,
        Err(_) if yes => {
            let detected = detected_platform_selection(&project_root);
            if detected.any() {
                detected
            } else {
                PlatformSelection::all()
            }
        }
        Err(_) if std::io::stdin().is_terminal() => prompt_platform(&project_root)?,
        Err(_) => {
            return Err(anyhow::anyhow!(
                "ctl init: choose at least one platform with --platform <name>, --claude, \
                 --opencode, --omp, or --all; use --yes for scripted onboarding."
            ));
        }
    };

    if dry_run {
        println!(
            "[dry-run] Would initialize the local task ledger and inject the {} integration.",
            platform_label(selection)
        );
        return Ok(());
    }
    ControlApp::init(&project_root)?;
    println!("Initialized local task ledger.");

    // Write default config if not present
    let config_path = project_root.join(".ctl").join("config.toml");
    if !config_path.exists() {
        let default_config = r#"# ctl Control Plane Configuration
# Customize which decay risks are enabled and their severity.

[risk]
# Production code decay risks (R1-R6). Set false to disable.
R1_cognitive_overload = true
R2_change_propagation = true
R3_knowledge_duplication = true
R4_accidental_complexity = true
R5_dependency_disorder = true
R6_domain_distortion = true

# Test decay risks (T1-T6). Set false to disable.
T1_test_obscurity = true
T2_test_brittleness = true
T3_test_duplication = true
T4_mock_abuse = true
T5_coverage_illusion = true
T6_architecture_mismatch = true

[severity]
# Override severity: "critical", "warning", "suggestion"
# R1 = "warning"

[scope]
# Glob patterns to exclude from analysis
# ignore = ["**/*.generated.*", "**/vendor/**"]
"#;
        std::fs::write(&config_path, default_config)?;
        println!("Created default .ctl/config.toml");
    }

    // Inject the chosen platform integration(s).
    inject_platform(selection, &project_root)?;
    let tracked = crate::infrastructure::project_update::record_initial_manifest(&project_root)?;
    if tracked > 0 {
        println!("Recorded {} managed template baseline file(s).", tracked);
    }

    // Self-check: the hooks just installed exec `ctl` from a separate process
    // (no shell). Warn now if that process would not resolve a runnable binary,
    // instead of letting the gate fail closed on the user's first write.
    report_ctl_reachability();

    // OMP-specific: verify the integration files are coherent and surface the
    // one prerequisite `ctl init` cannot check itself — that the plugin is
    // actually installed/linked (the hook only loads from an installed/linked
    // plugin; a marketplace install does not load it).
    if selection.omp {
        // The OMP hook runs inside the omp process, whose env is fixed at
        // launch — pin CTL_BIN in the agent .env OMP merges at startup so
        // resolution stops depending on which shell launched omp.
        pin_ctl_bin_in_omp_env();
        report_omp_integration(&project_root);
    }

    println!("Control-plane active for {}.", platform_label(selection));
    println!("Next steps:");
    println!("  1. Open your configured coding agent and describe the work.");
    println!("  2. Inspect task state with `ctl board`.");
    println!("  3. For a small scoped change, start with `ctl task quick --write-allow <path>`.");
    Ok(())
}

/// Human label for the selected platforms (used in init output).
pub(super) fn platform_label(selection: PlatformSelection) -> String {
    let mut labels = Vec::new();
    if selection.claude {
        labels.push("Claude Code (.claude/)");
    }
    if selection.opencode {
        labels.push("opencode (.opencode/)");
    }
    if selection.omp {
        labels.push("OMP (.omp/)");
    }
    labels.join(" + ")
}

/// Inject the integration files for the selected platform(s), reporting each.
pub(super) fn inject_platform(selection: PlatformSelection, project_root: &Path) -> Result<()> {
    use crate::infrastructure::skills;
    let injected = |n: usize| -> String {
        if n > 0 {
            format!("injected {n} file(s)")
        } else {
            "already present".to_string()
        }
    };
    if selection.omp {
        let n = skills::inject_all(project_root)?;
        println!("  .omp/      {} (skills + hooks + settings)", injected(n));
    }
    if selection.claude {
        let n = skills::inject_claude(project_root)?;
        println!(
            "  .claude/   {} (hooks + settings + workflow skills)",
            injected(n)
        );
    }
    if selection.opencode {
        let n = skills::inject_opencode(project_root)?;
        println!(
            "  .opencode/ {} (gate plugin + agents + skills)",
            injected(n)
        );
    }
    Ok(())
}

#[cfg(test)]
/// How a platform hook — a separate Node/Python process spawned later with the
/// default environment — would resolve the `ctl` binary it must exec (no shell).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum CtlReach {
    /// A runnable binary was found via the named resolution step.
    Resolved { how: &'static str },
    /// `ctl` is on PATH only as a non-exe shim (Windows npm `.cmd`/`.ps1`) and no
    /// real binary resolves — `execFile`/`subprocess` (no shell) cannot run it.
    OnlyShim,
    /// Nothing resolves anywhere.
    NotFound,
}

/// The environment inputs a hook sees, injected so the resolution is unit-testable
/// without touching the real environment or filesystem.
pub(super) struct CtlProbe<'a> {
    pub(super) windows: bool,
    pub(super) bin_name: &'a str,
    pub(super) ctl_bin: Option<String>,
    pub(super) home: Option<String>,
    pub(super) path_dirs: Vec<String>,
    pub(super) exists: &'a dyn Fn(&Path) -> bool,
}

pub(super) fn nonempty(o: &Option<String>) -> Option<&str> {
    o.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Resolve the exact `ctl` path a hook would exec, mirroring the one blessed
/// chain every adapter now uses (B-lite; see
/// `.ctl/spec/alignment/2026-07-04-binary-distribution-shrink.md`):
/// CTL_BIN → `~/.cargo/bin` → real exe on PATH. npm probing was retired with
/// the npm binary distribution. Returns `(resolved_path, how, saw_shim)`.
/// Pure: all env/fs access is injected via `CtlProbe`.
pub(super) fn resolve_ctl_path(p: &CtlProbe) -> (Option<std::path::PathBuf>, &'static str, bool) {
    // 1. explicit operator override
    if let Some(bin) = nonempty(&p.ctl_bin) {
        if (p.exists)(Path::new(bin)) {
            return (Some(Path::new(bin).to_path_buf()), "CTL_BIN", false);
        }
    }
    // 2. cargo install — the blessed install location
    if let Some(home) = nonempty(&p.home) {
        let cargo = Path::new(home).join(".cargo").join("bin").join(p.bin_name);
        if (p.exists)(&cargo) {
            return (Some(cargo), "cargo", false);
        }
    }
    // 3. PATH — require a REAL exe (`bin_name`), not just a shim. On Windows,
    //    execFile/subprocess (no shell) cannot run a bare `ctl`/`.cmd`/`.ps1`
    //    (a stale npm-era shim may still linger on PATH — flag it).
    let mut shim_seen = false;
    for dir in &p.path_dirs {
        let d = Path::new(dir);
        let exe = d.join(p.bin_name);
        if (p.exists)(&exe) {
            return (Some(exe), "PATH", false);
        }
        if p.windows {
            for shim in ["ctl", "ctl.cmd", "ctl.ps1"] {
                if (p.exists)(&d.join(shim)) {
                    shim_seen = true;
                }
            }
        }
    }
    (None, "", shim_seen)
}

#[cfg(test)]
/// `CtlReach` derived from `resolve_ctl_path` (pure). `#[cfg(test)]` (here and on
/// `CtlReach`) is intentional, not a smell: `report_ctl_reachability` execs via
/// `resolve_ctl_path` directly, so this derivation has no release-build caller.
/// Gating it keeps dead code out of the release binary instead of silencing a
/// warning; drop the gate if a non-test caller ever needs it.
pub(super) fn resolve_ctl_for_hook(p: &CtlProbe) -> CtlReach {
    let (path, how, shim) = resolve_ctl_path(p);
    match path {
        Some(_) => CtlReach::Resolved { how },
        None if shim => CtlReach::OnlyShim,
        None => CtlReach::NotFound,
    }
}

/// Build a `CtlProbe` from the real process environment.
pub(super) fn real_ctl_probe() -> CtlProbe<'static> {
    let windows = cfg!(windows);
    let path_dirs = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .map(|x| x.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let home = if windows {
        std::env::var("USERPROFILE").ok()
    } else {
        std::env::var("HOME").ok()
    };
    CtlProbe {
        windows,
        bin_name: if windows { "ctl.exe" } else { "ctl" },
        ctl_bin: std::env::var("CTL_BIN").ok(),
        home,
        path_dirs,
        exists: &|p| p.is_file(),
    }
}

/// Exec the resolved ctl (`--version`) to confirm it actually runs, not just
/// exists. Catches wrong-arch / corrupt / non-executable binaries that `is_file`
/// passes but the hook's `execFile` would fail on — the gap behind "ctl init
/// passed but the gate still reports binary-not-found". Bounded to 5s so a
/// broken binary cannot hang `ctl init`.
pub(super) fn exec_ctl_version(bin: &Path) -> Result<String> {
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    let mut child = std::process::Command::new(bin)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                use std::io::Read;
                let mut s = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut s);
                }
                return Ok(s.trim().to_string());
            }
            Ok(Some(_)) => anyhow::bail!("non-zero exit"),
            Ok(None) if start.elapsed() < TIMEOUT => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("timed out");
            }
            Err(e) => {
                // try_wait errored — the child may still be running. Kill +
                // reap before bailing so a wedged probe can never orphan a ctl
                // process that outlives `ctl init`.
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("{e}");
            }
        }
    }
}

/// B-lite version-skew check: every EXISTING candidate on the chain is exec'd
/// (`--version`) and compared against the resolved one. Silent version skew —
/// a stale binary shadowing a fresh install — was the single worst binary pit;
/// this turns it into a visible warning at `ctl init` time.
pub(super) fn report_version_skew(resolved: &Path, resolved_ver: &str, p: &CtlProbe) {
    let mut candidates: Vec<(&str, std::path::PathBuf)> = Vec::new();
    if let Some(bin) = nonempty(&p.ctl_bin) {
        candidates.push(("CTL_BIN", Path::new(bin).to_path_buf()));
    }
    if let Some(home) = nonempty(&p.home) {
        candidates.push((
            "cargo",
            Path::new(home).join(".cargo").join("bin").join(p.bin_name),
        ));
    }
    for dir in &p.path_dirs {
        let exe = Path::new(dir).join(p.bin_name);
        if (p.exists)(&exe) {
            candidates.push(("PATH", exe));
            break;
        }
    }
    for (label, cand) in candidates {
        if cand == resolved || !(p.exists)(&cand) {
            continue;
        }
        if let Ok(ver) = exec_ctl_version(&cand) {
            if ver != resolved_ver {
                println!(
                    "  ⚠️  version skew: {} at {} reports \"{}\" but the hooks resolve \"{}\" — \
                     the older one runs stale governance rules. Reinstall or remove it.",
                    label,
                    cand.display(),
                    ver,
                    resolved_ver
                );
            }
        }
    }
}

/// Print a reachability line after `ctl init`: PASS (with the resolved path and
/// a real `--version` run) when a hook would exec a runnable `ctl`, or a WARNING
/// and remediation when it would fail closed. Existence alone is not enough —
/// the resolved binary is exec'd to prove it runs. Also reports version skew
/// across the remaining chain candidates and python availability (the .claude
/// hooks are python scripts — without python the gate never fires).
pub(super) fn report_ctl_reachability() {
    let probe = real_ctl_probe();
    let (path, how, shim) = resolve_ctl_path(&probe);
    match path {
        Some(bin) => match exec_ctl_version(&bin) {
            Ok(ver) => {
                println!("  ctl reachable by gate hooks (resolved via {how}; runs: {ver}).");
                report_version_skew(&bin, &ver, &probe);
            }
            Err(e) => {
                println!(
                    "  ⚠️  `ctl` resolved via {how} at {} but FAILED to run ({e}); gate hooks \
                     will FAIL CLOSED (block writes/bash).\n     \
                     Fix: set CTL_BIN to a runnable ctl(.exe), or reinstall \
                     (`cargo install --path .`, or a GitHub release binary).",
                    bin.display()
                );
            }
        },
        None if shim => {
            println!(
                "  ⚠️  `ctl` is on PATH only as a shim (.cmd/.ps1), not a real executable.\n     \
                 Gate hooks exec `ctl` without a shell and will FAIL CLOSED (block writes/bash).\n     \
                 Fix: set CTL_BIN to the real ctl(.exe), or put a real ctl on PATH (e.g. `cargo install --path .`)."
            );
        }
        None => {
            println!(
                "  ⚠️  No runnable `ctl` binary found for the gate hooks; they will FAIL CLOSED \
                 until one is.\n     \
                 Fix: install `ctl` (`cargo install --path .`, or a GitHub release binary) or set CTL_BIN to its path."
            );
        }
    }
    // The .claude hooks run as `python <hook>.py` — no python, no gate.
    if std::process::Command::new("python")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        println!(
            "  ⚠️  `python` not found on PATH — the .claude hooks (gate/context/wrap-up) \
             cannot run, so Claude Code governance never fires. Install Python 3."
        );
    }
}

/// For `--platform omp`/`all`: verify the OMP integration is coherent and warn
/// about the one prerequisite `ctl init` cannot check itself — that the plugin
/// is actually installed/linked. OMP loads the hook ONLY from an npm-installed
/// or `omp plugin link`-ed plugin; a marketplace install (`omp plugin install
/// github:…`) does NOT load the extension, so governance silently never fires.
pub(super) fn report_omp_integration(project_root: &Path) {
    let hook = project_root
        .join(".omp")
        .join("hooks")
        .join("pre")
        .join("ctl-context.ts");
    if !hook.is_file() {
        println!(
            "  ⚠️  OMP governance hook not found at {} — the gate will not fire. \
             Re-run `ctl init --platform omp`.",
            hook.display()
        );
    }
    println!(
        "  ℹ️  OMP loads the hook only from an installed/linked plugin — run \
         `omp plugin link ./npm-omp` (dev) or `npm i @velo-ai/omp`, NOT \
         `omp plugin install github:…` (marketplace installs do not load the hook)."
    );
}

/// Where the OMP runtime reads its agent-level `.env`, mirroring oh-my-pi's
/// `packages/utils/src/dirs.ts`: `PI_CODING_AGENT_DIR` overrides the agent dir
/// outright; otherwise the config root (`PI_CONFIG_DIR`, default `~/.omp`)
/// plus `agent`. Profile/XDG redirection is deliberately not mirrored — init
/// prints the path it wrote so an advanced setup can relocate the entry.
pub(super) fn omp_agent_env_file(
    agent_dir_override: Option<&str>,
    config_dir: Option<&str>,
    home: &Path,
) -> std::path::PathBuf {
    if let Some(d) = agent_dir_override.map(str::trim).filter(|s| !s.is_empty()) {
        return Path::new(d).join(".env");
    }
    let root = match config_dir.map(str::trim).filter(|s| !s.is_empty()) {
        Some(d) if Path::new(d).is_absolute() => Path::new(d).to_path_buf(),
        Some(d) => home.join(d),
        None => home.join(".omp"),
    };
    root.join("agent").join(".env")
}

/// Upsert `KEY="value"` into dotenv-style content, preserving every other
/// line. Key matching mirrors OMP's `parseEnvFile`: everything before the
/// first `=`, trimmed. OMP's parser is last-wins, so ALL existing `KEY`
/// lines collapse into the one new line — a surviving duplicate below the
/// pin would silently override it. Returns the new content plus the
/// previously effective value: the LAST occurrence, quotes stripped.
pub(super) fn upsert_env_line(content: &str, key: &str, value: &str) -> (String, Option<String>) {
    let new_line = format!("{key}=\"{value}\"");
    let mut old: Option<String> = None;
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for raw in content.lines() {
        let line = raw.trim_end_matches('\r');
        let is_key = !line.trim_start().starts_with('#')
            && line.split_once('=').is_some_and(|(k, _)| k.trim() == key);
        if is_key {
            let val = line.split_once('=').map(|(_, v)| v.trim()).unwrap_or("");
            old = Some(val.trim_matches(|c| c == '"' || c == '\'').to_string());
            if !replaced {
                lines.push(new_line.clone());
                replaced = true;
            }
        } else {
            lines.push(line.to_string());
        }
    }
    if !replaced {
        lines.push(new_line);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    (out, old)
}

/// True for a binary living in a cargo `target/{debug,release}` tree — a
/// transient dev build that must not be pinned as the durable CTL_BIN.
pub(super) fn is_cargo_target_build(bin: &Path) -> bool {
    // Recognize a `target/{debug,release}/<bin>` artifact regardless of host
    // path separator: normalize backslashes so a Windows-style path is
    // identified on any platform (a unit test asserts the `C:\\...` form;
    // production sees the host-native form). Match the segment pair
    // `/target/debug/` or `/target/release/` so "targeted" does not match.
    let s = bin.to_string_lossy().replace('\\', "/");
    s.contains("/target/debug/") || s.contains("/target/release/")
}

/// Pin `CTL_BIN` in the OMP agent `.env` (runs on `ctl init --platform
/// omp|all`). OMP merges that file into its process env at startup (keys not
/// already set), so the governance hook resolves ctl regardless of which
/// shell launched `omp` — a launch env missing PATH/CTL_BIN was a real
/// fail-closed source. Best-effort: a failure warns and never fails init.
pub(super) fn pin_ctl_bin_in_omp_env() {
    // Prefer the chain's answer (stable blessed locations); fall back to the
    // binary running init — the manual-install case the pin exists for.
    let (resolved, _, _) = resolve_ctl_path(&real_ctl_probe());
    let Some(bin) = resolved.or_else(|| std::env::current_exe().ok()) else {
        return;
    };
    if is_cargo_target_build(&bin) {
        println!(
            "  ⚠️  CTL_BIN not pinned in the OMP .env: {} is a transient cargo target build. \
             Install a durable binary (cargo install --path ., or a GitHub release) and re-run ctl init.",
            bin.display()
        );
        return;
    }
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").ok()
    } else {
        std::env::var("HOME").ok()
    };
    let Some(home) = home.filter(|h| !h.trim().is_empty()) else {
        return;
    };
    let env_file = omp_agent_env_file(
        std::env::var("PI_CODING_AGENT_DIR").ok().as_deref(),
        std::env::var("PI_CONFIG_DIR").ok().as_deref(),
        Path::new(&home),
    );
    let existing = std::fs::read_to_string(&env_file).unwrap_or_default();
    let value = bin.display().to_string();
    let (updated, old) = upsert_env_line(&existing, "CTL_BIN", &value);
    if updated == existing {
        println!(
            "  .omp env: CTL_BIN already pinned in {}",
            env_file.display()
        );
        return;
    }
    let write = env_file
        .parent()
        .map(std::fs::create_dir_all)
        .unwrap_or(Ok(()))
        .and_then(|()| std::fs::write(&env_file, &updated));
    match write {
        Ok(()) => match old.filter(|o| o != &value) {
            Some(o) => println!(
                "  .omp env: CTL_BIN pinned to {} in {} (was: {})",
                value,
                env_file.display(),
                o
            ),
            None => println!(
                "  .omp env: CTL_BIN pinned to {} in {} — the OMP hook now resolves ctl \
                     regardless of the shell omp was launched from",
                value,
                env_file.display()
            ),
        },
        Err(e) => println!(
            "  ⚠️  could not pin CTL_BIN in {} ({e}) — set CTL_BIN there manually if the \
             OMP gate reports binary-not-found.",
            env_file.display()
        ),
    }
}

pub(super) fn cmd_skills(command: &SkillsCommands) -> Result<()> {
    match command {
        SkillsCommands::Sync { check } => {
            let project_root = std::env::current_dir()?;
            // Two single-source generators run under one command: the per-platform
            // workflow SKILL.md files, and the `@velo-ai/omp` plugin package
            // assembled from `.omp/`. Their outcomes are merged so `--check` gates
            // both in CI.
            let mut outcome = crate::infrastructure::skill_sync::sync(&project_root, *check)?;
            let plugin = crate::infrastructure::omp_plugin::sync(&project_root, *check)?;
            outcome.written.extend(plugin.written);
            outcome.stale.extend(plugin.stale);
            if *check {
                if outcome.stale.is_empty() {
                    println!("All generated files are in sync with their source.");
                } else {
                    for s in &outcome.stale {
                        println!("  stale: {s}");
                    }
                    return Err(anyhow::anyhow!(
                        "{} generated file(s) out of date — run `ctl skills sync`",
                        outcome.stale.len()
                    ));
                }
            } else if outcome.written.is_empty() {
                println!("All generated files already up to date.");
            } else {
                for s in &outcome.written {
                    println!("  wrote: {s}");
                }
                println!("Regenerated {} file(s) from source.", outcome.written.len());
            }
            Ok(())
        }
    }
}

/// Interactive platform picker. Multiple entries may be separated by commas or spaces.
pub(super) fn prompt_platform(project_root: &Path) -> Result<PlatformSelection> {
    use std::io::Write as _;

    let detected = detected_platform_selection(project_root);
    println!("Select one or more agent platforms to wire up:");
    println!("  1) claude    Claude Code  (.claude/)");
    println!("  2) opencode  opencode     (.opencode/)");
    println!("  3) omp       OMP          (.omp/)");
    println!("  4) all       all supported platforms");
    if detected.any() {
        println!(
            "Detected existing integrations: {}",
            platform_label(detected)
        );
    }
    print!("Choice(s) [Enter keeps detected, or all on a fresh project]: ");
    std::io::stdout().flush().ok();

    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let input = line.trim();
    if input.is_empty() {
        return Ok(if detected.any() {
            detected
        } else {
            PlatformSelection::all()
        });
    }

    let mut selection = PlatformSelection::default();
    for token in input.split([',', ' ', ';']) {
        let token = token.trim().to_lowercase();
        if token.is_empty() {
            continue;
        }
        match token.as_str() {
            "1" | "claude" => selection.claude = true,
            "2" | "opencode" => selection.opencode = true,
            "3" | "omp" => selection.omp = true,
            "4" | "all" => selection = PlatformSelection::all(),
            other => {
                return Err(anyhow::anyhow!(
                    "Unrecognized platform '{}'. Choose claude, opencode, omp, all, or 1-4.",
                    other
                ));
            }
        }
    }
    if !selection.any() {
        return Err(anyhow::anyhow!("No platform selected."));
    }
    Ok(selection)
}

/// Resolve a task's gates: explicit `--gates` win; otherwise derive the project
/// default floor recorded in `.ctl/config.toml` by /ctl-spec. Errors
/// when neither is available — a task must declare at least one gate (the app
/// layer enforces the same non-empty invariant), and ctl hardcodes no floor.
pub(super) fn resolve_task_gates(project_root: &Path, explicit: &[String]) -> Result<Vec<String>> {
    if !explicit.is_empty() {
        return Ok(explicit.to_vec());
    }
    let derived = read_project_default_gates(project_root);
    if derived.is_empty() {
        return Err(anyhow::anyhow!(
            "No --gates given and no project default gate floor found \
             (.ctl/config.toml [project].default_gates). Pass --gates, or run \
             /ctl-spec to record the project gate floor."
        ));
    }
    Ok(derived)
}

/// Read `[project].default_gates` from `.ctl/config.toml`, if present.
///
/// ctl does not hardcode a project gate floor; the ctl-spec skill
/// derives one per project and records it here. Returns an empty vec when the
/// file, the `[project]` table, or the key is absent. The config has no general
/// TOML reader (it is otherwise written/consumed as raw text), so parsing is a
/// minimal, targeted extraction of the `default_gates = [...]` string array.
/// Whatever it returns is still validated against the gate-template list by the
/// caller (`create_task`), so a malformed entry surfaces as an "unknown gate"
/// error rather than silent misbehavior.
pub(super) fn read_project_default_gates(project_root: &Path) -> Vec<String> {
    let path = project_root.join(".ctl").join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(content) => parse_project_default_gates(&content),
        Err(_) => Vec::new(),
    }
}

/// Extract the `default_gates` string array under the `[project]` table.
/// Supports the single-line form the skill writes
/// (`default_gates = ["cargo_check", "cargo_test"]`) and a multi-line array.
/// Tolerant of trailing comments after the closing `]`. Non-string / unquoted
/// tokens are dropped (the caller validates the remainder).
pub(super) fn parse_project_default_gates(content: &str) -> Vec<String> {
    let mut in_project = false;
    let mut collecting = false;
    let mut buf = String::new();
    for raw in content.lines() {
        let line = raw.trim();
        if !collecting && line.starts_with('[') && line.ends_with(']') {
            in_project = line == "[project]";
            continue;
        }
        if !in_project {
            continue;
        }
        if collecting {
            if let Some(close) = line.find(']') {
                buf.push(' ');
                buf.push_str(&line[..close]);
                return extract_quoted_tokens(&buf);
            }
            buf.push(' ');
            buf.push_str(line);
            continue;
        }
        // Look for `default_gates = [ ... ]`.
        let after_key = match line.strip_prefix("default_gates") {
            Some(rest) => rest.trim_start(),
            None => continue,
        };
        let after_eq = match after_key.strip_prefix('=') {
            Some(rest) => rest.trim_start(),
            None => continue,
        };
        let Some(open) = after_eq.find('[') else {
            continue;
        };
        let tail = &after_eq[open + 1..];
        if let Some(close) = tail.find(']') {
            return extract_quoted_tokens(&tail[..close]);
        }
        buf.push_str(tail);
        collecting = true;
    }
    Vec::new()
}

/// Split a comma-separated list of double-quoted tokens into the inner strings.
pub(super) fn extract_quoted_tokens(inner: &str) -> Vec<String> {
    inner
        .split(',')
        .filter_map(|tok| {
            let t = tok.trim().strip_prefix('"')?.strip_suffix('"')?;
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        })
        .collect()
}
