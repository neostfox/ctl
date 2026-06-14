use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

/// Default gate execution timeout (EXEC-002).
const GATE_TIMEOUT_SECS: u64 = 60;

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
pub fn run_gate(gate_id: &str, working_dir: &Path) -> Result<GateRunResult> {
    let template =
        find_template(gate_id).ok_or_else(|| anyhow!("Unknown gate template: {}", gate_id))?;

    let allowed_env = build_allowed_env();
    let gate_id_owned = gate_id.to_string();
    let command = template.command.to_string();
    let args: Vec<String> = template.args.iter().map(|s| s.to_string()).collect();
    let working_dir_owned = working_dir.to_path_buf();

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::process::Command::new(&command)
            .args(&args)
            .current_dir(&working_dir_owned)
            .env_clear()
            .envs(&allowed_env)
            .output();
        let _ = tx.send(result);
    });

    let output = match rx.recv_timeout(Duration::from_secs(GATE_TIMEOUT_SECS)) {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return Err(anyhow!("Failed to execute gate '{}': {}", gate_id_owned, e)),
        Err(_) => {
            return Err(anyhow!(
                "Gate '{}' timed out after {}s",
                gate_id_owned,
                GATE_TIMEOUT_SECS
            ))
        }
    };

    let stdout = cap_output(&output.stdout);
    let stderr = cap_output(&output.stderr);

    Ok(GateRunResult {
        gate_id: gate_id_owned,
        passed: output.status.success(),
        exit_code: output.status.code().unwrap_or(-1),
        stdout,
        stderr,
    })
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
}
