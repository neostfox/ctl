use super::*;

/// Remove single- and double-quoted spans before segment classification.
/// Backslash escapes are honored inside double quotes; an unterminated quote
/// drops the tail — the conservative direction (less text to misclassify,
/// never more).
pub(super) fn strip_quoted(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                }
            }
            '"' => {
                let mut esc = false;
                for q in chars.by_ref() {
                    if esc {
                        esc = false;
                        continue;
                    }
                    match q {
                        '\\' => esc = true,
                        '"' => break,
                        _ => {}
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Classify a bash command into an action category.
/// Classify a single (non-compound) command segment.
pub(super) fn classify_bash_segment(segment: &str) -> &'static str {
    let cmd = segment.trim();
    if cmd.starts_with("git commit") || cmd.starts_with("git add") {
        "git_commit"
    } else if cmd.starts_with("git push") {
        "git_push"
    } else if cmd.starts_with("cargo add") || cmd.starts_with("cargo install") {
        // `cargo install --path <dir>` is a self-install of a local crate (the
        // dev-loop's binary reinstall) — build-tier, not a dependency change.
        // Registry installs (`cargo install <crate>`) stay deps: supply chain.
        if cmd.starts_with("cargo install") && cmd.contains("--path") {
            "cargo_build"
        } else {
            "cargo_deps"
        }
    } else if cmd.starts_with("cargo check")
        || cmd.starts_with("cargo test")
        || cmd.starts_with("cargo build")
        || cmd.starts_with("cargo fmt")
        || cmd.starts_with("cargo clippy")
    {
        "cargo_build"
    } else if is_bash_write_segment(cmd) {
        "bash_write"
    } else {
        "bash_other"
    }
}

/// Best-effort detection of a shell segment that writes files. ctl cannot
/// path-scope shell writes, so a positive result only RECLASSIFIES the segment
/// so the gate can require an active task (closing the "idle bash write" gap) —
/// it is NOT a `write_allow` boundary. Conservative by design: a redirect
/// appended to a git/cargo command is not reclassified, and obfuscation
/// (eval/base64/here-doc/quoted `>`) can still hide a write. Mirrors the
/// operator-splitting caveat on [`classify_bash`].
pub(super) fn is_bash_write_segment(cmd: &str) -> bool {
    let first = cmd.split_whitespace().next().unwrap_or("");
    if matches!(
        first,
        "tee" | "cp" | "mv" | "dd" | "install" | "truncate" | "ln"
    ) || cmd.starts_with("sed -i")
        || cmd.starts_with("sed --in-place")
    {
        return true;
    }
    // gh7 / issue #7: git checkout/restore with a path marker (--, --ours,
    // --theirs) is the merge-resolution file write — the exact dogfood bypass
    // (`git checkout --ours pnpm-workspace.yaml`). git restore always targets
    // files (no branch-switch semantics), so any positional flags it as a write.
    if (cmd.starts_with("git checkout") || cmd.starts_with("git restore"))
        && (cmd.contains(" --ours") || cmd.contains(" --theirs") || cmd.contains(" -- "))
    {
        return true;
    }
    if cmd.starts_with("git restore") && cmd.split_whitespace().count() > 2 {
        return true;
    }
    if has_opaque_wrapper(cmd) {
        return true;
    }
    segment_has_file_redirect(cmd)
}

/// True if the segment invokes an opaque command wrapper — `eval`, `bash -c`,
/// `sh -c` (and the -lc login-shell variants). These execute a STRING whose
/// contents quote-stripping hides, so the classifier cannot see whether a
/// write hides inside. Used to fail-closed (deny under an active task): an
/// opaque command that might write anywhere cannot be confirmed in-scope.
/// (gh7 hardening / issue #7.)
pub(super) fn has_opaque_wrapper(seg: &str) -> bool {
    let first = seg.split_whitespace().next().unwrap_or("");
    first == "eval"
        || seg.starts_with("bash -c")
        || seg.starts_with("sh -c")
        || seg.starts_with("bash -lc")
        || seg.starts_with("sh -lc")
}

/// True if any segment of `command` (split on shell operators) is an opaque
/// wrapper. The wrapper verb sits OUTSIDE quotes, so this scans the original
/// command text (not strip_quoted output).
pub(super) fn command_has_opaque_wrapper(command: &str) -> bool {
    command
        .split([';', '\n', '&', '|', '(', ')', '`'])
        .any(|seg| has_opaque_wrapper(seg.trim()))
}

/// True if the segment contains an output redirection to a FILE (`>` / `>>`),
/// excluding fd-duplication forms (`>&`, `2>&1`) which target no file.
pub(super) fn segment_has_file_redirect(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'>' {
            let after = if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                i + 2
            } else {
                i + 1
            };
            let dup = after < bytes.len() && bytes[after] == b'&';
            if !dup && !cmd[after..].trim_start().is_empty() {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Best-effort extraction of the file paths a bash command writes to. Returns
/// every target that could be statically identified from the common file-
/// mutating patterns (redirection, cp/mv/install/rm/tee/mkdir/truncate/sed -i,
/// git checkout/restore, npm pkg set). An EMPTY result means the classifier
/// could not extract a target (undecidable) — the gate then falls back to
/// observe-mode allow-and-record. NOT a security boundary: obfuscation (eval,
/// env vars, brace expansion, command substitution, here-docs) can hide the
/// real target. Mirrors the caveat on [`classify_bash`] (gh7 / issue #7 F1).
pub(super) fn extract_bash_write_targets(command: &str) -> Vec<String> {
    let stripped = strip_quoted(command);
    let mut targets: Vec<String> = Vec::new();
    for segment in stripped.split([';', '\n', '&', '|', '(', ')', '`']) {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        // Output redirection `> file` / `>> file` (fd-duplication `>&` excluded).
        if let Some(t) = extract_redirect_target(seg) {
            targets.push(t);
        }
        let tokens: Vec<&str> = seg.split_whitespace().collect();
        let first = tokens.first().copied().unwrap_or("");
        let positional: Vec<&str> = tokens.iter().skip(1).copied().collect();
        match first {
            "cp" | "mv" | "install" => {
                // Last positional (non-flag) arg is the destination.
                if let Some(dst) = positional.iter().rev().find(|t| !t.starts_with('-')) {
                    targets.push(dst.to_string());
                }
            }
            "rm" | "tee" | "mkdir" | "truncate" => {
                // Every positional (non-flag) arg is a write/remove target.
                for t in positional.iter().filter(|t| !t.starts_with('-')) {
                    targets.push(t.to_string());
                }
            }
            "sed" => {
                // `sed -i EXPR FILE` — last positional is the file.
                if let Some(f) = positional.iter().rev().find(|t| !t.starts_with('-')) {
                    targets.push(f.to_string());
                }
            }
            "git" => {
                let sub = positional.first().copied().unwrap_or("");
                if sub == "checkout" || sub == "restore" {
                    // Paths after `--`, or bare positionals; --ours/--theirs are flags.
                    let mut after_dd = false;
                    for t in positional.iter().skip(1) {
                        if *t == "--" {
                            after_dd = true;
                        } else if after_dd {
                            targets.push(t.to_string());
                        } else if matches!(*t, "--ours" | "--theirs") {
                            // flag — the path(s) follow it
                        } else if !t.starts_with('-') {
                            targets.push(t.to_string());
                        }
                    }
                }
            }
            "npm"
                if positional.first().map(|s| *s == "pkg").unwrap_or(false)
                    && positional.get(1).map(|s| *s == "set").unwrap_or(false) =>
            {
                // `npm pkg set ...` mutates package.json.
                targets.push("package.json".to_string());
            }
            _ => {}
        }
    }
    targets
}

/// Extract the file path after the first `>` / `>>` file redirection in a
/// segment. fd-duplication (`>&`, `2>&1`) targets no file and is skipped.
pub(super) fn extract_redirect_target(seg: &str) -> Option<String> {
    let bytes = seg.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'>' {
            let after = if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                i + 2
            } else {
                i + 1
            };
            if after < bytes.len() && bytes[after] == b'&' {
                i = after;
                continue;
            }
            let rest = seg[after..].trim_start();
            let file: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            if !file.is_empty() {
                return Some(file);
            }
        }
        i += 1;
    }
    None
}

/// Classify a bash command for gating. Compound commands (`a && b`, `a; b`,
/// `a | b`, `$(a)`) are classified by their MOST restrictive segment — otherwise
/// `cp x y && git push` would slip through as `bash_other` (the loophole the
/// architecture review found). Best-effort only: a caller with shell access can
/// still obscure intent (eval, base64, env-indirection); this discourages and
/// audits casual composition, it is not a hard security boundary.
pub(super) fn classify_bash(command: &str) -> &'static str {
    let (mut push, mut deps, mut commit, mut write, mut build) =
        (false, false, false, false, false);
    // Quoted spans are DATA, not commands — a commit MESSAGE containing
    // "(cargo install /" segment-split into a cargo_deps classification and
    // denied a legitimate commit (live false positive, decisions.jsonl,
    // 2026-07-05). Strip quotes before splitting. Caveat, consistent with the
    // best-effort stance above: bash expands `$(..)`/backticks INSIDE double
    // quotes, so a substitution hidden in quotes now escapes classification —
    // a determined caller already could via eval/base64; casual composition
    // is still caught.
    let cleaned = strip_quoted(command);
    // Split on shell control/grouping operators so a restricted action inside a
    // compound or substitution (`$(..)`, backticks) becomes its own segment.
    for segment in cleaned.split([';', '\n', '&', '|', '(', ')', '`']) {
        match classify_bash_segment(segment) {
            "git_push" => push = true,
            "cargo_deps" => deps = true,
            "git_commit" => commit = true,
            "bash_write" => write = true,
            "cargo_build" => build = true,
            _ => {}
        }
    }
    // Most restrictive wins: step-up actions, then commit window, then a file
    // write (needs an active task), then build, else other.
    if push {
        "git_push"
    } else if deps {
        "cargo_deps"
    } else if commit {
        "git_commit"
    } else if write {
        "bash_write"
    } else if build {
        "cargo_build"
    } else {
        "bash_other"
    }
}

/// M6 shared-`.git` hardening: detect a destructive git verb in a (possibly
/// compound) command. Returns the offending verb for disclosure, or `None`.
///
/// These verbs rewrite refs/HEAD/the index or delete working-tree files — run
/// against a repository whose `.git` is shared by a live agent run's worktree,
/// they can corrupt or strand that run (e.g. `git branch -D` removes a ref a
/// run's worktree is on; `git clean -x` can delete the `.ctl/runs/*/worktree`
/// directories themselves). Detected independently of [`classify_bash`] so the
/// caller can apply it as an *overlay* — deny only when a run is active,
/// otherwise fall through to normal gating (so `git reset && git push` is still
/// push-gated when nothing is running).
///
/// Split on the same shell operators as [`classify_bash`] so a verb buried in a
/// compound/substitution (`x && git reset`, `$(git clean)`) is still caught;
/// best-effort, not a hard boundary (eval/indirection can still obscure intent).
/// `switch` is included as the modern synonym of `checkout`. Deliberately NOT
/// gated yet (each needs its own analysis): `merge`, `stash`, `cherry-pick`,
/// `gc`, `worktree` mutation.
pub(super) fn detect_shared_git_op(command: &str) -> Option<&'static str> {
    command
        .split([';', '\n', '&', '|', '(', ')', '`'])
        .find_map(shared_git_segment)
}

/// Classify one (non-compound) segment as a destructive git op, if it is one.
pub(super) fn shared_git_segment(segment: &str) -> Option<&'static str> {
    let rest = segment.trim().strip_prefix("git ")?.trim_start();
    let sub = rest.split_whitespace().next().unwrap_or("");
    match sub {
        "checkout" => Some("git checkout"),
        "switch" => Some("git switch"),
        "reset" => Some("git reset"),
        "rebase" => Some("git rebase"),
        "clean" => Some("git clean"),
        // Plain `git branch` (list) and `git branch <name>` (create) are safe;
        // only delete/move/force flags rewrite or remove refs other worktrees
        // may track.
        "branch"
            if rest.split_whitespace().skip(1).any(|t| {
                matches!(
                    t,
                    "-d" | "-D" | "--delete" | "-m" | "-M" | "--move" | "-f" | "--force"
                )
            }) =>
        {
            Some("git branch -D")
        }
        _ => None,
    }
}

/// Check if a path is within any of the allowed scopes.
///
/// Routes the target through `classify_write_target` first: `Path::starts_with`
/// is component-wise but does NOT collapse `ParentDir`, so a target like
/// `src/../etc/passwd` would otherwise match the `src` scope lexically and
/// be reported as in-scope. `Protected` targets still match here because a
/// granted `ctl apply` exception can authorize a protected path; only
/// `Suspicious` (traversal/UNC/empty) and `OutOfRepo` short-circuit.
pub(super) fn path_in_scope(project_root: &Path, path: &str, scopes: &[String]) -> bool {
    if matches!(
        classify_write_target(project_root, path),
        WriteTarget::Suspicious(_) | WriteTarget::OutOfRepo
    ) {
        return false;
    }
    let resolved = if Path::new(path).is_relative() {
        project_root.join(path)
    } else {
        Path::new(path).to_path_buf()
    };
    scopes
        .iter()
        .any(|s| resolved.starts_with(project_root.join(s)))
}

/// M-c: the first OTHER active task (non-archived, phase `in_progress` or
/// `review`) whose `write_allow` contains `path`. A write landing inside another
/// active task's claimed scope is hard-denied even when it sits within the
/// governing task's own scope — two active tasks must never write the same
/// region. This is the single-path specialization of
/// `schedule::detect_write_scope_overlap`; `ctl schedule validate` applies the
/// set-vs-set form across a whole plan. "Active" matches the `ctl board`
/// definition (in_progress | review), keeping one notion of active across
/// M-a / M-b / M-c.
pub(super) fn first_overlapping_active_task(
    project_root: &Path,
    path: &str,
    governing_task_id: &str,
) -> Result<Option<String>> {
    let app = ControlApp::open(project_root, false)?;
    let reports = app.generate_status_report()?;
    for report in &reports {
        let phase = report.get("phase").and_then(|v| v.as_str()).unwrap_or("");
        let is_archived = report
            .get("is_archived")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let task_id = report
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if is_archived || task_id == governing_task_id || !matches!(phase, "in_progress" | "review")
        {
            continue;
        }
        let state = app.replay_task(&task_id)?;
        let scopes: Vec<String> = state.write_allow.iter().cloned().collect();
        if path_in_scope(project_root, path, &scopes) {
            return Ok(Some(task_id));
        }
    }
    Ok(None)
}

/// Check if a path targets the spec directory (always writable for updates).
///
/// Routes the target through `classify_write_target` first: `Path::starts_with`
/// is component-wise but does NOT collapse `ParentDir`, so a target like
/// `.ctl/spec/../tasks/events.jsonl` would otherwise match the spec prefix
/// lexically and bypass the protected-path hard deny (the only hard deny in
/// observe mode). Only an `InRepo` target (no `..`, no UNC, no absolute
/// escape, not protected) can be treated as a spec path.
pub(super) fn is_spec_path(project_root: &Path, path: &str) -> bool {
    if !matches!(
        classify_write_target(project_root, path),
        WriteTarget::InRepo
    ) {
        return false;
    }
    let resolved = if Path::new(path).is_relative() {
        project_root.join(path)
    } else {
        Path::new(path).to_path_buf()
    };
    resolved.starts_with(project_root.join(".ctl").join("spec"))
}
/// Gate-side classification of a Write/Edit target path (observe mode).
///
/// Unlike `PathNormalizer::normalize` (the structural boundary check run at
/// create/revise time, which requires an existing, canonicalizable parent),
/// this is a lexical check for the hook path: targets arrive absolute from the
/// host and may name files that do not exist yet. Protection is checked
/// explicitly here and stays a hard deny (unless a `ctl apply` exception is
/// granted): create/revise no longer reject protected paths — they may be
/// declared in write_allow — so this gate is the single enforcement point.
pub(super) enum WriteTarget {
    /// Inside the repo, not protected.
    InRepo,
    /// Outside the project root entirely (e.g. a home-dir memory file).
    OutOfRepo,
    /// Under a protected root (ledgers, schemas, manifests) — hard deny.
    Protected(String),
    /// Traversal/UNC/empty — the gate refuses to classify it. Hard deny.
    Suspicious(&'static str),
}

pub(super) fn classify_write_target(project_root: &Path, target: &str) -> WriteTarget {
    use std::path::Component;
    if target.starts_with("\\\\") || target.starts_with("//") {
        return WriteTarget::Suspicious("UNC path");
    }
    let p = Path::new(target);
    let rel = if p.is_absolute() {
        match p.strip_prefix(project_root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => return WriteTarget::OutOfRepo,
        }
    } else {
        p.to_path_buf()
    };
    let mut normalized = std::path::PathBuf::new();
    for comp in rel.components() {
        match comp {
            Component::ParentDir => return WriteTarget::Suspicious(".. traversal"),
            Component::RootDir | Component::Prefix(_) => {
                return WriteTarget::Suspicious("absolute path component")
            }
            Component::CurDir => continue,
            Component::Normal(c) => normalized.push(c),
        }
    }
    if normalized.as_os_str().is_empty() {
        return WriteTarget::Suspicious("empty path");
    }
    let norm = PathNormalizer::new(project_root.to_path_buf());
    if norm.is_protected(&normalized) {
        return WriteTarget::Protected(normalized.to_string_lossy().into_owned());
    }
    // Canonical-resolve through symlinks and re-check `is_protected` on the
    // canonical target. The lexical check above misses a symlink INSIDE an
    // in-scope dir that redirects a write at a protected path (e.g. an agent
    // with `write_allow=["src"]` creates `src/ln -> .git/config`, then writes
    // `src/ln` — the lexical path is `src/ln`, the canonical landing is
    // `.git/config`). `canonical_for_gate` follows the leaf symlink (and
    // rejects any symlink in a non-leaf component via `normalize`'s ancestry
    // walk). Failure (escapes root, no existing ancestor canonicalizes) is
    // treated as Suspicious — fail-closed.
    let rel_str = normalized.to_string_lossy().into_owned();
    match norm.canonical_for_gate(&rel_str) {
        Ok(canon_rel) if norm.is_protected(&canon_rel) => {
            WriteTarget::Protected(canon_rel.to_string_lossy().into_owned())
        }
        Ok(_) => WriteTarget::InRepo,
        Err(_) => WriteTarget::Suspicious("cannot canonicalize target (symlink/escape)"),
    }
}
