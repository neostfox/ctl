//! `ctl update` — in-place self-updater (ADR 0002).
//!
//! This is the ONLY place in ctl core that performs network I/O. It is an
//! operator-invoked maintenance command: it resolves the latest release from
//! GitHub, downloads + sha256-verifies the release archive over HTTPS, extracts
//! it with the system `tar`, and replaces the running binary. It produces no
//! canonical events and never runs on the governed task/run/gate path — the
//! event ledger stays pure and offline.
//!
//! TLS uses ureq's `native-tls` backend (OS-native: schannel / Security.framework
//! / OpenSSL), so no C/asm toolchain is required to build (unlike rustls' `ring`).

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

const REPO: &str = "neostfox/ctl";
const USER_AGENT: &str = concat!("ctl-update/", env!("CARGO_PKG_VERSION"));

/// The compiled-in version of this binary (no leading `v`).
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Release-asset naming for the running build's platform.
struct Target {
    /// The release asset file name, e.g. `ctl-x86_64-pc-windows-msvc.zip`.
    asset: String,
    /// The binary file name inside the archive.
    bin_name: &'static str,
}

fn detect_target() -> Result<Target> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    // Mirrors scripts/install.sh + install.ps1: Windows ships only x86_64 (.zip),
    // Linux/macOS ship x86_64 + aarch64 (.tar.gz).
    let (triple, ext, bin_name) = match (os, arch) {
        ("windows", _) => ("x86_64-pc-windows-msvc", "zip", "ctl.exe"),
        ("linux", "x86_64") => ("x86_64-unknown-linux-gnu", "tar.gz", "ctl"),
        ("linux", "aarch64") => ("aarch64-unknown-linux-gnu", "tar.gz", "ctl"),
        ("macos", "x86_64") => ("x86_64-apple-darwin", "tar.gz", "ctl"),
        ("macos", "aarch64") => ("aarch64-apple-darwin", "tar.gz", "ctl"),
        _ => bail!("ctl update: unsupported platform '{os}-{arch}' (no published release asset)"),
    };
    Ok(Target {
        asset: format!("ctl-{triple}.{ext}"),
        bin_name,
    })
}

/// Parse a `vX.Y.Z` / `X.Y.Z` tag into comparable numeric components. Unparseable
/// suffixes (pre-release tags) make the parse fail, and we fall back to string
/// inequality at the call site.
fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Is `latest` strictly newer than `current`? Numeric semver compare when both
/// parse; otherwise a conservative "differs" check (any difference => offer it).
fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => l > c,
        _ => latest.trim_start_matches('v') != current.trim_start_matches('v'),
    }
}

/// An agent pinned to the native-tls provider. ureq enables the `native-tls`
/// *capability* via the crate feature, but its runtime TLS provider still
/// defaults to Rustls (which we deliberately do not compile) — so the provider
/// must be selected explicitly or every HTTPS call panics.
fn agent() -> ureq::Agent {
    use ureq::tls::{TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .build(),
        )
        .build()
        .into()
}

/// GET a URL and return the response body bytes. The only network primitive.
fn http_get_bytes(url: &str) -> Result<Vec<u8>> {
    let resp = agent()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .with_context(|| format!("ctl update: request failed: {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("ctl update: GET {url} returned HTTP {}", status.as_u16());
    }
    let mut buf = Vec::new();
    resp.into_body()
        .into_reader()
        .read_to_end(&mut buf)
        .with_context(|| format!("ctl update: reading body: {url}"))?;
    Ok(buf)
}

/// Resolve the latest release tag via the GitHub API.
fn resolve_latest_tag() -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let bytes = http_get_bytes(&url)?;
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).context("ctl update: GitHub API response was not JSON")?;
    json.get("tag_name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("ctl update: no tag_name in GitHub latest-release response"))
}

fn download_base(version: &str) -> String {
    if version == "latest" {
        format!("https://github.com/{REPO}/releases/latest/download")
    } else {
        format!("https://github.com/{REPO}/releases/download/{version}")
    }
}

/// Verify `bytes` against an expected `sha256` hex digest (case-insensitive).
fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hasher.finalize();
    let actual_hex = actual
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    // A `.sha256` file is typically `<hex>  <filename>`; take the first field.
    let expected = expected_hex
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    if expected.is_empty() {
        bail!("ctl update: empty expected checksum");
    }
    if expected != actual_hex {
        bail!("ctl update: checksum mismatch (expected {expected}, got {actual_hex})");
    }
    Ok(())
}

/// Extract `archive` into `dest` using the system `tar` (bsdtar on Windows reads
/// `.zip`; GNU/bsd tar auto-detect gzip for `.tar.gz`).
fn extract(archive: &Path, dest: &Path) -> Result<()> {
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
        .context("ctl update: failed to run `tar` (is it on PATH?)")?;
    if !status.success() {
        bail!("ctl update: `tar` failed to extract {}", archive.display());
    }
    Ok(())
}

/// On Windows, a previous update renamed the running binary to `<exe>.old`; it is
/// locked while that old process runs, so clean it up best-effort on a later run.
fn cleanup_stale(exe: &Path) {
    let backup = backup_path(exe);
    let _ = std::fs::remove_file(backup);
}

fn backup_path(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_os_string();
    s.push(".old");
    PathBuf::from(s)
}

/// Replace the running binary at `exe` with the freshly extracted `new_bin`.
fn self_replace(exe: &Path, new_bin: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new_bin, std::fs::Permissions::from_mode(0o755))
            .context("ctl update: chmod the new binary")?;
        // On Unix renaming over a running executable is allowed: the live process
        // keeps the old inode, new invocations get the new file.
        std::fs::rename(new_bin, exe).context("ctl update: replacing the binary")?;
    }
    #[cfg(windows)]
    {
        // Windows refuses to overwrite a running .exe, but it allows renaming it.
        // Move the live binary aside, then put the new one in its place. The
        // `.old` file stays locked until this process exits; the next run's
        // cleanup_stale() removes it.
        let backup = backup_path(exe);
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(exe, &backup).context("ctl update: renaming the running binary aside")?;
        if let Err(e) = std::fs::rename(new_bin, exe) {
            // Roll back so the operator is not left without a binary.
            let _ = std::fs::rename(&backup, exe);
            return Err(e).context("ctl update: installing the new binary");
        }
    }
    Ok(())
}

/// Entry point for `ctl update`.
pub fn run(version: Option<String>, check: bool) -> Result<()> {
    let exe = std::env::current_exe().context("ctl update: cannot locate the running binary")?;
    cleanup_stale(&exe);

    let current = current_version();

    // Resolve which tag we are targeting.
    let tag = match &version {
        Some(v) => v.clone(),
        None => resolve_latest_tag()?,
    };

    if check {
        if is_newer(&tag, current) {
            println!("ctl {current} -> {tag} available. Run `ctl update` to install.");
        } else {
            println!("ctl {current} is up to date (latest: {tag}).");
        }
        return Ok(());
    }

    if version.is_none() && !is_newer(&tag, current) {
        println!("ctl {current} is already the latest ({tag}); nothing to do.");
        return Ok(());
    }

    let target = detect_target()?;
    let base = download_base(version.as_deref().unwrap_or("latest"));
    let asset_url = format!("{base}/{}", target.asset);

    println!("ctl update: downloading {asset_url}");
    let archive_bytes = http_get_bytes(&asset_url)?;

    // Checksum: required when published. The release tooling ships `<asset>.sha256`.
    match http_get_bytes(&format!("{asset_url}.sha256")) {
        Ok(sha_bytes) => {
            let expected = String::from_utf8_lossy(&sha_bytes);
            verify_sha256(&archive_bytes, &expected)?;
            println!("ctl update: checksum verified");
        }
        Err(e) => {
            bail!(
                "ctl update: could not fetch checksum ({e}); refusing to install unverified binary"
            );
        }
    }

    // Stage in a temp dir next to the binary so the final rename is same-volume.
    let parent = exe
        .parent()
        .ok_or_else(|| anyhow!("ctl update: binary has no parent directory"))?;
    let stage = parent.join(format!(".ctl-update-{}", std::process::id()));
    std::fs::create_dir_all(&stage).context("ctl update: creating staging dir")?;
    let cleanup_stage = || {
        let _ = std::fs::remove_dir_all(&stage);
    };

    let result = (|| -> Result<()> {
        let archive_path = stage.join(&target.asset);
        std::fs::write(&archive_path, &archive_bytes).context("ctl update: writing archive")?;
        extract(&archive_path, &stage)?;
        let new_bin = stage.join(target.bin_name);
        if !new_bin.exists() {
            bail!(
                "ctl update: '{}' not found in the downloaded archive",
                target.bin_name
            );
        }
        self_replace(&exe, &new_bin)?;
        Ok(())
    })();

    cleanup_stage();
    result?;

    println!(
        "ctl update: updated {current} -> {tag} at {}",
        exe.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parse_and_compare() {
        assert_eq!(parse_semver("v0.0.5"), Some((0, 0, 5)));
        assert_eq!(parse_semver("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("0.1"), None);
        assert_eq!(parse_semver("0.0.5-rc1"), None);
        assert!(is_newer("v0.0.6", "0.0.5"));
        assert!(is_newer("0.1.0", "0.0.9"));
        assert!(!is_newer("0.0.5", "0.0.5"));
        assert!(!is_newer("v0.0.4", "0.0.5"));
        // Unparseable tags fall back to "differs".
        assert!(is_newer("nightly", "0.0.5"));
        assert!(!is_newer("v0.0.5", "0.0.5"));
    }

    #[test]
    fn target_asset_naming_is_platform_correct() {
        // Whatever platform the test runs on, the asset name is well-formed and
        // the binary name matches the archive kind.
        let t = detect_target().expect("supported test platform");
        assert!(t.asset.starts_with("ctl-"));
        if cfg!(windows) {
            assert!(t.asset.ends_with(".zip"));
            assert_eq!(t.bin_name, "ctl.exe");
        } else {
            assert!(t.asset.ends_with(".tar.gz"));
            assert_eq!(t.bin_name, "ctl");
        }
    }

    #[test]
    fn sha256_verification() {
        // sha256("") and a `<hex>  file` formatted expectation.
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_sha256(b"", empty).is_ok());
        assert!(verify_sha256(b"", &format!("{empty}  ctl.tar.gz")).is_ok());
        assert!(verify_sha256(b"not empty", empty).is_err());
        assert!(verify_sha256(b"", "").is_err());
    }

    #[test]
    fn backup_path_appends_old() {
        let p = backup_path(Path::new("/usr/local/bin/ctl"));
        assert_eq!(p, PathBuf::from("/usr/local/bin/ctl.old"));
    }
}
