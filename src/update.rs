//! Self-update from GitHub Releases.
//!
//! Like Claude Code's auto-updater: the interactive UI checks once in the
//! background, silently installs a newer release in place of the running
//! binary and only says something when an update was installed (or could not
//! be installed for lack of permissions). Downloads go through curl / wget,
//! the same tools install.sh uses, so ttop itself needs no TLS stack.

use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

const REPO: &str = "HiroGitea/ttop";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

pub enum Outcome {
    UpToDate,
    Updated { version: String },
    /// A newer release exists but the binary's directory is not writable.
    NeedsPermission { version: String, dir: PathBuf },
}

/// Auto-update runs only in binaries built by the release workflow
/// (`TTOP_RELEASE_BUILD` set at compile time), unless the user opts out with
/// `TTOP_DISABLE_AUTOUPDATER=1`.
pub fn auto_update_enabled() -> bool {
    let disabled = std::env::var("TTOP_DISABLE_AUTOUPDATER")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    option_env!("TTOP_RELEASE_BUILD").is_some() && !disabled
}

/// Check the latest release and, if it is newer, replace the running binary.
pub fn update() -> Result<Outcome> {
    let tag = latest_tag()?;
    let version = tag.trim_start_matches('v').to_string();
    if !is_newer(&version, CURRENT) {
        return Ok(Outcome::UpToDate);
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    install(&tag, &version, &exe)
}

fn latest_tag() -> Result<String> {
    let body = fetch(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    v["tag_name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("no tag_name in release metadata"))
}

fn install(tag: &str, version: &str, exe: &Path) -> Result<Outcome> {
    let asset = asset_name()?;
    let dir = exe.parent().context("executable has no parent directory")?;
    // The new binary is written next to the old one so the final rename is atomic.
    let tmp = dir.join(format!(".ttop-update-{}", std::process::id()));
    let file = match fs::OpenOptions::new().write(true).create_new(true).mode(0o755).open(&tmp) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            return Ok(Outcome::NeedsPermission { version: version.to_string(), dir: dir.to_path_buf() });
        }
        Err(e) => return Err(e).with_context(|| format!("creating {}", tmp.display())),
    };
    let res = write_verified(file, &tmp, tag, version, &asset)
        .and_then(|()| fs::rename(&tmp, exe).with_context(|| format!("replacing {}", exe.display())));
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res.map(|()| Outcome::Updated { version: version.to_string() })
}

fn write_verified(mut file: fs::File, tmp: &Path, tag: &str, version: &str, asset: &str) -> Result<()> {
    let base = format!("https://github.com/{REPO}/releases/download/{tag}");
    let bin = fetch(&format!("{base}/{asset}"))?;
    let sums = String::from_utf8(fetch(&format!("{base}/SHA256SUMS"))?)?;
    let want = expected_sha256(&sums, asset).with_context(|| format!("no checksum for {asset}"))?;
    if sha256_hex(&bin) != want {
        bail!("checksum mismatch for {asset}");
    }
    file.write_all(&bin)?;
    file.sync_all()?;
    drop(file);
    fs::set_permissions(tmp, fs::Permissions::from_mode(0o755))?;

    // Refuse to install a binary that does not run or reports another version.
    let out = Command::new(tmp).arg("--version").output()?;
    let reported = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() || reported.trim() != format!("ttop {version}") {
        bail!("downloaded binary reports `{}`, expected ttop {version}", reported.trim());
    }
    Ok(())
}

fn asset_name() -> Result<String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", arch @ ("x86_64" | "aarch64")) => Ok(format!("ttop-linux-{arch}")),
        (os, arch) => bail!("no prebuilt binary for {os}/{arch}"),
    }
}

/// Download `url` with curl, falling back to wget. Output is captured, so
/// nothing reaches the terminal while the TUI is running.
fn fetch(url: &str) -> Result<Vec<u8>> {
    let out = match Command::new("curl").args(["-fsSL", "--max-time", "60", url]).output() {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            Command::new("wget").args(["-q", "-T", "60", "-O", "-", url]).output()
        }
        r => r,
    }
    .context("curl or wget is required")?;
    if !out.status.success() {
        bail!("download failed: {url}");
    }
    Ok(out.stdout)
}

fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.trim_start_matches('v').split('.');
    let v = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    it.next().is_none().then_some(v)
}

/// True if `latest` is a plain x.y.z version greater than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// Look up `asset` in a `sha256sum` style listing.
fn expected_sha256(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut it = line.split_whitespace();
        let (hash, name) = (it.next()?, it.next()?);
        (name.trim_start_matches('*') == asset).then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        // pre-releases and malformed tags never trigger an update
        assert!(!is_newer("0.3.0-rc1", "0.2.0"));
        assert!(!is_newer("0.3", "0.2.0"));
        assert!(!is_newer("latest", "0.2.0"));
    }

    #[test]
    fn checksum_lookup() {
        let sums = "AAAA  ttop-linux-aarch64\nbbbb *ttop-linux-x86_64\n";
        assert_eq!(expected_sha256(sums, "ttop-linux-aarch64").as_deref(), Some("aaaa"));
        assert_eq!(expected_sha256(sums, "ttop-linux-x86_64").as_deref(), Some("bbbb"));
        assert_eq!(expected_sha256(sums, "ttop-linux-riscv64"), None);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
