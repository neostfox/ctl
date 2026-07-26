use super::*;

pub(super) fn cmd_memory(command: &MemoryCommands) -> Result<()> {
    match command {
        MemoryCommands::Verify { json } => cmd_memory_verify(*json),
    }
}

/// Scan `~/.ctl/memory/*.md` for project-path pollution — content that would
/// leak one repo's specifics into every project session (global memory is shared
/// across all repos). Warns, never blocks. [ROADMAP #1/S]
///
/// Signals (advisory — a human triages; false positives are expected and fine):
/// a source path (`src/`), a code file extension, a build/test command
/// (`cargo test`, `npm run`, ...), or an absolute/local filesystem path.
pub(super) fn cmd_memory_verify(json: bool) -> Result<()> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    let memory_dir = match &home {
        Some(h) => Path::new(h).join(".ctl").join("memory"),
        None => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"has_memory_dir": false, "findings": 0, "items": []})
                );
            } else {
                println!("No home directory (USERPROFILE/HOME) — cannot locate ~/.ctl/memory/.");
            }
            return Ok(());
        }
    };
    if !memory_dir.exists() {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "has_memory_dir": false,
                    "memory_dir": memory_dir.display().to_string(),
                    "findings": 0,
                    "items": [],
                })
            );
        } else {
            println!(
                "No global memory directory at {} — nothing to scan.",
                memory_dir.display()
            );
        }
        return Ok(());
    }

    let mut findings: Vec<(std::path::PathBuf, usize, Vec<&'static str>, String)> = Vec::new();
    let mut files_scanned = 0u32;
    scan_memory_dir(&memory_dir, &mut findings, &mut files_scanned)?;

    if json {
        let items: Vec<serde_json::Value> = findings
            .iter()
            .map(|(f, line, sigs, excerpt)| {
                serde_json::json!({
                    "file": f.to_string_lossy(),
                    "line": line,
                    "signals": sigs,
                    "excerpt": excerpt,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "has_memory_dir": true,
                "memory_dir": memory_dir.display().to_string(),
                "files_scanned": files_scanned,
                "findings": findings.len(),
                "items": items,
            })
        );
        return Ok(());
    }

    if findings.is_empty() {
        println!(
            "No project-path pollution detected (scanned {} memory file(s)).",
            files_scanned
        );
    } else {
        println!(
            "POLLUTION warnings ({} across {} memory file(s)):",
            findings.len(),
            files_scanned
        );
        for (f, line, sigs, excerpt) in &findings {
            println!(
                "  {}:{}  [{}]  {}",
                f.display(),
                line,
                sigs.join(", "),
                excerpt
            );
        }
        println!("\nAdvisory only — global memory is shared across all projects; review whether these references are project-specific.");
    }
    Ok(())
}

fn scan_memory_dir(
    dir: &Path,
    findings: &mut Vec<(std::path::PathBuf, usize, Vec<&'static str>, String)>,
    files_scanned: &mut u32,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.metadata()?.is_dir() {
            scan_memory_dir(&path, findings, files_scanned)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            *files_scanned += 1;
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for (i, line) in content.lines().enumerate() {
                let sigs = pollution_signals(line);
                if !sigs.is_empty() {
                    let excerpt = ellipsize_inline(line.trim(), 80);
                    findings.push((path.clone(), i + 1, sigs, excerpt));
                }
            }
        }
    }
    Ok(())
}

/// Pollution signals on a single line. Returns the label of every signal that
/// fires (advisory — overlap is fine). Kept deliberately permissive: this warns,
/// it does not block, so false positives are acceptable and human-triaged.
fn pollution_signals(line: &str) -> Vec<&'static str> {
    let mut sigs = Vec::new();
    let lower = line.to_ascii_lowercase();
    if line.contains("src/") || line.contains("\\src\\") {
        sigs.push("source path");
    }
    const CODE_EXTS: &[&str] = &[
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".go", ".py", ".java", ".rb", ".vue", ".svelte",
    ];
    if CODE_EXTS.iter().any(|e| line.contains(e)) {
        sigs.push("code extension");
    }
    const BUILD_CMDS: &[&str] = &[
        "cargo run",
        "cargo build",
        "cargo test",
        "cargo bench",
        "cargo fmt",
        "npm run",
        "npm test",
        "npm install",
        "npm ci",
        "pnpm ",
        "yarn ",
        "pip install",
        "pytest",
        "jest",
        "go build",
        "go test",
        "dotnet ",
    ];
    if BUILD_CMDS.iter().any(|c| lower.contains(c)) {
        sigs.push("build/test command");
    }
    const ABS_MARKERS: &[&str] = &[
        "/home/",
        "/users/",
        "/usr/local/",
        "c:\\",
        "d:\\",
        "c:/",
        "d:/",
    ];
    if ABS_MARKERS.iter().any(|m| lower.contains(m)) {
        sigs.push("absolute path");
    }
    sigs
}

/// Trim `s` to at most `max` chars (by char count), appending `...` on truncation.
fn ellipsize_inline(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max - 3).collect();
    format!("{}...", truncated)
}

#[cfg(test)]
mod memory_verify_tests {
    use super::*;

    #[test]
    fn pollution_signals_detection() {
        assert!(pollution_signals("edit src/auth/token.rs").contains(&"source path"));
        assert!(pollution_signals("see foo.rs in the tree").contains(&"code extension"));
        assert!(pollution_signals("run cargo test --release").contains(&"build/test command"));
        assert!(pollution_signals("the file is at /home/x/y").contains(&"absolute path"));
        assert!(pollution_signals("C:\\Users\\x\\proj\\src").contains(&"absolute path"));
        // clean general guidance triggers nothing
        assert!(
            pollution_signals("Prefer small, independently-shippable tasks.").is_empty(),
            "generic guidance should not pollute"
        );
    }

    #[test]
    fn scan_flags_polluted_lines_in_temp_dir() {
        let tmp =
            std::env::temp_dir().join(format!("ctl-memory-verify-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("general.md"),
            "Prefer small tasks.\nThis is fine.\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("polluted.md"),
            "# Notes\nIn src/auth we did X.\nRun cargo test then.\nPath: /home/x/y.rs\n",
        )
        .unwrap();

        let mut findings = Vec::new();
        let mut scanned = 0u32;
        scan_memory_dir(&tmp, &mut findings, &mut scanned).unwrap();
        assert_eq!(scanned, 2);
        assert_eq!(findings.len(), 3, "three polluted lines (lines 2,3,4)");
        let lines: Vec<usize> = findings.iter().map(|(_, l, _, _)| *l).collect();
        assert!(lines.contains(&2) && lines.contains(&3) && lines.contains(&4));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ellipsize_truncates_long_lines() {
        assert_eq!(ellipsize_inline("short", 10), "short");
        let long = "x".repeat(100);
        let e = ellipsize_inline(&long, 10);
        assert_eq!(e.chars().count(), 10);
        assert!(e.ends_with("..."));
    }
}
