use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// Default gate execution timeout (EXEC-002).
const GATE_TIMEOUT_SECS: u64 = 60;

/// Grace period between SIGTERM and SIGKILL when terminating a timed-out process
/// tree (Unix). Windows uses `taskkill /F` (immediate force).
const GRACE_MS: u64 = 1000;

/// Maximum captured output per stream (EXEC-002).
const OUTPUT_CAP: usize = 64 * 1024;

/// A gate template defines a fixed command to run.
/// No arbitrary shell — only predefined templates are allowed (EXEC-001).
#[derive(Debug, Clone)]
pub struct GateTemplate {
    pub id: &'static str,
    pub description: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
}

/// All available gate templates. This is the authoritative list.
pub static GATE_TEMPLATES: &[GateTemplate] = &[
    GateTemplate {
        id: "cargo_fmt_check",
        description: "Check code formatting with cargo fmt",
        command: "cargo",
        args: &["fmt", "--check"],
    },
    GateTemplate {
        id: "cargo_check",
        description: "Check compilation with cargo check",
        command: "cargo",
        args: &["check"],
    },
    GateTemplate {
        id: "cargo_test",
        description: "Run tests with cargo test",
        command: "cargo",
        args: &["test"],
    },
    GateTemplate {
        id: "cargo_clippy",
        description: "Run clippy lint checks",
        command: "cargo",
        args: &["clippy", "--", "-D", "warnings"],
    },
];

/// Result of running a gate.
#[derive(Debug)]
pub struct GateRunResult {
    pub gate_id: String,
    pub passed: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// True when the gate exceeded the timeout and its process tree was
    /// terminated (and confirmed reaped) before this result was produced. A
    /// timed-out gate is never `passed`. If the process tree could NOT be
    /// confirmed terminated, `run_gate` returns an `Err` instead (execution
    /// containment failure) — it is never reported as an ordinary failed gate.
    pub timed_out: bool,
}

/// Find a gate template by ID.
pub fn find_template(id: &str) -> Option<&'static GateTemplate> {
    GATE_TEMPLATES.iter().find(|t| t.id == id)
}

/// Run a gate with EXEC-002 controls:
/// - Only allowlisted command templates (EXEC-001)
/// - Timeout: 60s default
/// - Environment: full inherit with proxy/auth denylist (EXEC-003: no network deps)
/// - Output cap: 64KB per stream, truncated if exceeded
/// - Explicit working directory
/// - On timeout, the spawned process tree is terminated before this returns, but
///   the strength of that guarantee is platform-dependent. On Unix the child
///   leads its own process group, so `kill(-pgid)` signals the whole tree
///   (TERM→KILL); on Windows `taskkill /T` makes a BEST-EFFORT sweep of
///   descendants (no Job Object, so a grandchild spawned during the sweep is not
///   guaranteed reaped). On BOTH platforms ctl confirms containment by reaping
///   the ROOT it manages — never by a blocking wait or a group probe. When the
///   managed root cannot be confirmed reaped, returns `Err` (execution
///   containment failure) rather than an ordinary failed gate.
pub fn run_gate(gate_id: &str, working_dir: &Path) -> Result<GateRunResult> {
    let template =
        find_template(gate_id).ok_or_else(|| anyhow!("Unknown gate template: {}", gate_id))?;
    let args: Vec<String> = template.args.iter().map(|s| s.to_string()).collect();
    let sup = supervise(
        template.command,
        &args,
        working_dir,
        Duration::from_secs(GATE_TIMEOUT_SECS),
    )
    .map_err(|e| anyhow!("Gate '{}': {}", gate_id, e))?;

    Ok(GateRunResult {
        gate_id: gate_id.to_string(),
        passed: sup.success,
        exit_code: sup.exit_code,
        stdout: cap_output(&sup.stdout),
        stderr: cap_output(&sup.stderr),
        timed_out: sup.timed_out,
    })
}

/// Outcome of supervising one child process to completion or timeout.
struct Supervised {
    success: bool,
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

/// Spawn `command` (Unix: in its own process group) and supervise it to
/// completion or `timeout`. On timeout the ENTIRE process tree is terminated and
/// reaped before returning. Returns `Err` ONLY when termination cannot be
/// confirmed (execution containment failure) — a normal timeout with confirmed
/// termination returns `Ok` with `timed_out = true` and `success = false`.
fn supervise(
    command: &str,
    args: &[String],
    working_dir: &Path,
    timeout: Duration,
) -> Result<Supervised> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let allowed_env = build_allowed_env();
    let mut cmd = Command::new(command);
    cmd.args(args)
        .current_dir(working_dir)
        .env_clear()
        .envs(&allowed_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Unix: give the child its own process group so the whole tree can be
    // signalled at once via `kill -- -<pgid>`. Descendants inherit the group.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("failed to execute: {}", e))?;
    let pid = child.id();

    // Drain stdout/stderr on threads so a chatty gate cannot deadlock on a full
    // pipe while we poll for completion.
    let mut out_pipe = child.stdout.take().expect("stdout piped");
    let mut err_pipe = child.stderr.take().expect("stderr piped");
    let out_h = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out_pipe.read_to_end(&mut b);
        b
    });
    let err_h = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err_pipe.read_to_end(&mut b);
        b
    });

    let deadline = Instant::now() + timeout;
    let poll = Duration::from_millis(50);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = out_h.join().unwrap_or_default();
                let stderr = err_h.join().unwrap_or_default();
                return Ok(Supervised {
                    success: status.success(),
                    exit_code: status.code().unwrap_or(-1),
                    stdout,
                    stderr,
                    timed_out: false,
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    // Terminate the whole tree and CONFIRM by reaping the root.
                    // Reaping the managed child is authoritative; it does not
                    // depend on a process-group probe that a mis-grouped child
                    // could fool. Crucially we never call a blocking
                    // `child.wait()` here: if the root cannot be reaped, a
                    // surviving child would hang the supervisor forever (this is
                    // exactly what stalled CI before).
                    let confirmed = terminate_and_reap(&mut child, pid);
                    if !confirmed {
                        // Root could not be confirmed reaped. Surviving
                        // descendants may hold the pipes open and block
                        // read_to_end forever — do NOT join the drain threads on
                        // this error path; leaking them (and a possible zombie)
                        // is the acceptable cost of never hanging.
                        return Err(anyhow!(
                            "execution containment failure: gate timed out after {}s, but \
                             process-tree termination could not be confirmed",
                            timeout.as_secs()
                        ));
                    }
                    // Tree is dead → pipes closed → joins return promptly.
                    let stdout = out_h.join().unwrap_or_default();
                    let stderr = err_h.join().unwrap_or_default();
                    return Ok(Supervised {
                        success: false,
                        exit_code: -1,
                        stdout,
                        stderr,
                        timed_out: true,
                    });
                }
                std::thread::sleep(poll);
            }
            Err(e) => return Err(anyhow!("failed while waiting on child: {}", e)),
        }
    }
}

/// Poll-reap the managed direct child until it exits or `budget` elapses.
/// Reaping the ROOT is the authoritative confirmation that the process ctl
/// manages is gone — unlike a process-group probe, it cannot be fooled by a
/// child that never joined the expected group, and unlike `Child::wait` it never
/// blocks indefinitely. Returns `true` once the child is reaped.
fn reaped_within(child: &mut std::process::Child, budget: Duration) -> bool {
    use std::time::Instant;
    let deadline = Instant::now() + budget;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return false,
        }
    }
}

/// Terminate the timed-out process tree, then confirm the managed ROOT is reaped.
/// `taskkill /T /F` force-terminates the process AND its descendants (best
/// effort: no Job Object, so a grandchild spawned mid-sweep may briefly outlive
/// this). Its exit code is unreliable when descendants are a moving target, so we
/// confirm by reaping the ROOT — the process ctl manages — not by trusting
/// taskkill or polling a reuse-prone PID listing.
#[cfg(windows)]
fn terminate_and_reap(child: &mut std::process::Child, pid: u32) -> bool {
    use std::process::{Command, Stdio};
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    reaped_within(child, Duration::from_millis(500))
}

/// Terminate the timed-out process tree, then confirm the managed ROOT is reaped.
/// The child leads its own process group (pgid == pid, via `process_group(0)`),
/// so `kill(-pid, sig)` signals the whole tree at once — TERM for a grace window,
/// then KILL. We signal the group AND the child directly (belt-and-suspenders, in
/// case the child somehow never joined the group), and confirm by reaping the
/// root rather than probing the group. The syscall is used directly because the
/// external `kill` binary's parsing of a bare negative pid is not portable.
#[cfg(unix)]
fn terminate_and_reap(child: &mut std::process::Child, pid: u32) -> bool {
    let p = pid as libc::pid_t;
    // SAFETY: `kill(2)` only delivers a signal; it has no memory-safety effects.
    unsafe {
        libc::kill(-p, libc::SIGTERM);
        libc::kill(p, libc::SIGTERM);
    }
    if reaped_within(child, Duration::from_millis(GRACE_MS)) {
        // Root exited on TERM; KILL the group so no descendant lingers.
        unsafe {
            libc::kill(-p, libc::SIGKILL);
        }
        return true;
    }
    unsafe {
        libc::kill(-p, libc::SIGKILL);
        libc::kill(p, libc::SIGKILL);
    }
    reaped_within(child, Duration::from_millis(500))
}

/// Build the execution environment with a denylist approach (EXEC-002, EXEC-003).
///
/// Instead of allowlisting specific vars (which is fragile across platforms and
/// toolchain versions), we pass through the full environment and only strip
/// variables that could enable unauthorized network access. This prevents
/// transient linker failures on Windows where the MSVC toolchain needs many
/// env vars (LIB, INCLUDE, SystemRoot, APPDATA, PATHEXT, etc.) that are
/// difficult to enumerate completely.
fn build_allowed_env() -> HashMap<String, String> {
    filter_allowed_env(std::env::vars())
}

/// Strip network/proxy/token vars (EXEC-002/003) from an environment iterator.
///
/// Pure over its input so it can be unit-tested with a synthetic environment —
/// the test must NOT mutate the real process env (`std::env::set_var`), which is
/// not thread-safe and data-races every parallel test reading `std::env::vars`
/// (each gate-spawning test calls `build_allowed_env`), sporadically aborting
/// the whole test binary.
fn filter_allowed_env<I>(vars: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    /// Network-related env var prefixes whose presence could bypass EXEC-003.
    const BLOCKED_PREFIXES: &[&str] = &[
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "FTP_PROXY",
        "ftp_proxy",
        "NO_PROXY",
        "no_proxy",
        // Auth tokens that could be used to access network resources
        "AUTH_TOKEN",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "GITLAB_TOKEN",
        "CARGO_REGISTRY_TOKEN",
        "CARGO_REGISTRY_HTTP_", // CARGO_REGISTRY_HTTP_*, but CARGO_HOME etc are fine
        "NETRC",
        "SSH_AUTH_SOCK",
    ];

    vars.into_iter()
        .filter(|(key, _)| {
            !BLOCKED_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix))
        })
        .collect()
}

/// Cap output to OUTPUT_CAP bytes, truncating with a marker if exceeded.
fn cap_output(raw: &[u8]) -> String {
    if raw.len() <= OUTPUT_CAP {
        String::from_utf8_lossy(raw).into_owned()
    } else {
        let mut s = String::from_utf8_lossy(&raw[..OUTPUT_CAP]).into_owned();
        s.push_str("\n... [output truncated at 64KB]");
        s
    }
}

/// List all available gate templates.
pub fn list_templates() -> Vec<&'static GateTemplate> {
    GATE_TEMPLATES.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_template_known() {
        assert!(find_template("cargo_fmt_check").is_some());
        assert!(find_template("cargo_check").is_some());
        assert!(find_template("cargo_test").is_some());
        assert!(find_template("cargo_clippy").is_some());
    }

    #[test]
    fn test_find_template_unknown() {
        assert!(find_template("nonexistent_gate").is_none());
    }

    #[test]
    fn test_list_templates_count() {
        assert_eq!(list_templates().len(), 4);
    }

    #[test]
    fn test_run_gate_unknown() {
        let dir = std::env::current_dir().unwrap();
        assert!(run_gate("nonexistent_gate", &dir).is_err());
    }

    #[test]
    fn test_run_gate_known() {
        let dir = std::env::current_dir().unwrap();
        let result = run_gate("cargo_check", &dir);
        assert!(
            result.is_ok(),
            "cargo_check gate should execute: {:?}",
            result
        );
        let r = result.unwrap();
        assert_eq!(r.gate_id, "cargo_check");
        // cargo_check on a valid project should pass
        assert!(r.passed, "cargo_check should pass: stderr: {}", r.stderr);
    }

    #[test]
    fn test_cap_output_under_limit() {
        let data = b"hello world";
        let result = cap_output(data);
        assert_eq!(result, "hello world");
        assert!(!result.contains("truncated"));
    }

    #[test]
    fn test_cap_output_over_limit() {
        let data = vec![b'x'; OUTPUT_CAP + 100];
        let result = cap_output(&data);
        assert!(result.contains("truncated"));
        assert!(result.len() > OUTPUT_CAP);
    }

    #[test]
    fn test_build_allowed_env_has_path() {
        let env = build_allowed_env();
        assert!(env.contains_key("PATH") || env.contains_key("Path"));
    }

    #[test]
    fn test_build_allowed_env_blocks_proxy_vars() {
        // Filter a SYNTHETIC environment — never mutate the real process env
        // (set_var is not thread-safe and would data-race parallel tests).
        let synthetic = [
            (
                "HTTPS_PROXY".to_string(),
                "http://evil.proxy:1234".to_string(),
            ),
            ("GITHUB_TOKEN".to_string(), "ghp_secret".to_string()),
            ("CARGO_REGISTRY_HTTP_TIMEOUT".to_string(), "5".to_string()),
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("CARGO_HOME".to_string(), "/home/u/.cargo".to_string()),
        ];
        let env = filter_allowed_env(synthetic);
        assert!(
            !env.contains_key("HTTPS_PROXY"),
            "proxy vars must be blocked"
        );
        assert!(
            !env.contains_key("GITHUB_TOKEN"),
            "token vars must be blocked"
        );
        assert!(
            !env.contains_key("CARGO_REGISTRY_HTTP_TIMEOUT"),
            "CARGO_REGISTRY_HTTP_* must be blocked"
        );
        assert!(env.contains_key("PATH"), "benign vars must pass through");
        assert!(
            env.contains_key("CARGO_HOME"),
            "CARGO_HOME must NOT be caught by the CARGO_REGISTRY_HTTP_ prefix"
        );
    }

    // ── process-tree termination ──

    use std::time::Instant;

    /// A command that runs ~`secs` seconds as a DIRECT child.
    #[cfg(windows)]
    fn sleeper(secs: u32) -> (&'static str, Vec<String>) {
        // `ping -n N` sends N pings ~1s apart ≈ (N-1) seconds.
        (
            "ping",
            vec!["-n".into(), (secs + 1).to_string(), "127.0.0.1".into()],
        )
    }
    #[cfg(unix)]
    fn sleeper(secs: u32) -> (&'static str, Vec<String>) {
        ("sleep", vec![secs.to_string()])
    }

    /// A command that runs the real sleeper as a GRANDCHILD (under a shell).
    #[cfg(windows)]
    fn nested_sleeper(secs: u32) -> (&'static str, Vec<String>) {
        (
            "cmd",
            vec!["/c".into(), format!("ping -n {} 127.0.0.1 >nul", secs + 1)],
        )
    }
    #[cfg(unix)]
    fn nested_sleeper(secs: u32) -> (&'static str, Vec<String>) {
        ("sh", vec!["-c".into(), format!("sleep {}", secs)])
    }

    fn quick(code: i32) -> (&'static str, Vec<String>) {
        #[cfg(windows)]
        {
            ("cmd", vec!["/c".into(), format!("exit {}", code)])
        }
        #[cfg(unix)]
        {
            ("sh", vec!["-c".into(), format!("exit {}", code)])
        }
    }

    #[test]
    fn supervise_quick_success_is_not_timeout() {
        let (c, a) = quick(0);
        let dir = std::env::current_dir().unwrap();
        let s = supervise(c, &a, &dir, Duration::from_secs(10)).unwrap();
        assert!(s.success && !s.timed_out, "exit={}", s.exit_code);
    }

    #[test]
    fn supervise_quick_failure_reaped_not_timeout() {
        let (c, a) = quick(3);
        let dir = std::env::current_dir().unwrap();
        let s = supervise(c, &a, &dir, Duration::from_secs(10)).unwrap();
        assert!(!s.success && !s.timed_out);
        assert_eq!(s.exit_code, 3);
    }

    #[test]
    fn supervise_timeout_kills_direct_child_promptly() {
        let (c, a) = sleeper(30);
        let dir = std::env::current_dir().unwrap();
        let start = Instant::now();
        let s = supervise(c, &a, &dir, Duration::from_secs(1)).unwrap();
        assert!(s.timed_out && !s.success);
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "did not return promptly: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn supervise_timeout_kills_grandchild_promptly() {
        let (c, a) = nested_sleeper(30);
        let dir = std::env::current_dir().unwrap();
        let start = Instant::now();
        let s = supervise(c, &a, &dir, Duration::from_secs(1)).unwrap();
        assert!(s.timed_out);
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "did not return promptly: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn supervise_timeout_stops_side_effects() {
        let dir = std::env::temp_dir();
        let marker = dir.join(format!("ctl-ptt-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let mp = marker.to_string_lossy().to_string();

        #[cfg(windows)]
        let (c, a) = (
            "cmd",
            vec![
                "/c".to_string(),
                format!(
                    "for /L %i in (1,1,100000) do (echo x>>\"{}\" & ping -n 1 -w 30 127.0.0.1 >nul)",
                    mp
                ),
            ],
        );
        #[cfg(unix)]
        let (c, a) = (
            "sh",
            vec![
                "-c".to_string(),
                format!("while true; do echo x >> '{}'; sleep 0.03; done", mp),
            ],
        );

        let s = supervise(c, &a, &dir, Duration::from_secs(1)).unwrap();
        assert!(s.timed_out);
        // After return, the tree is dead → the file must stop growing.
        let size1 = std::fs::metadata(&marker).map(|m| m.len()).unwrap_or(0);
        std::thread::sleep(Duration::from_millis(600));
        let size2 = std::fs::metadata(&marker).map(|m| m.len()).unwrap_or(0);
        let _ = std::fs::remove_file(&marker);
        assert_eq!(
            size1, size2,
            "side effects continued after termination ({size1} → {size2})"
        );
    }

    #[test]
    fn supervise_repeated_timeouts_no_hang() {
        let dir = std::env::current_dir().unwrap();
        for _ in 0..3 {
            let (c, a) = sleeper(30);
            let s = supervise(c, &a, &dir, Duration::from_secs(1)).unwrap();
            assert!(s.timed_out);
        }
    }

    /// A process that ignores SIGTERM must still be SIGKILLed (Unix only).
    #[cfg(unix)]
    #[test]
    fn supervise_kills_term_ignoring_process() {
        let dir = std::env::current_dir().unwrap();
        let (c, a) = (
            "sh",
            vec!["-c".to_string(), "trap '' TERM; sleep 30".to_string()],
        );
        let start = Instant::now();
        let s = supervise(c, &a, &dir, Duration::from_secs(1)).unwrap();
        assert!(s.timed_out);
        assert!(start.elapsed() < Duration::from_secs(15));
    }
}
