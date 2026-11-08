//! Updating itself: the .exe asks GitHub for the latest release and, if it is newer, swaps itself for the
//! release's .exe after checking it against the SHA-256 digest GitHub publishes for it.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::applog;
use crate::system::{self, VERSION};

pub const GITHUB_REPO: &str = "Zorakidd/Update-check";
/// A release's .exe is a few megabytes; anything near this is not one.
const DOWNLOAD_LIMIT: usize = 100 * 1024 * 1024;

fn releases_url() -> String {
    std::env::var("UPDATE_CHECK_RELEASES_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest"))
}

/// The numbers of a version such as "v0.2.0", without trailing zeros, so 0.1 and 0.1.0 are the same.
pub fn parse_version(text: &str) -> Option<Vec<u64>> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = Vec::new();
    for part in text.split('.') {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        parts.push(part.parse().ok()?);
    }
    while parts.len() > 1 && parts.last() == Some(&0) {
        parts.pop();
    }
    Some(parts)
}

pub fn is_newer(tag: &str, installed: &str) -> bool {
    matches!((parse_version(tag), parse_version(installed)), (Some(tag), Some(installed)) if tag > installed)
}

pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: Option<u64>,
    pub digest: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Installed,
    UpToDate,
    Failed,
}

/// Replace the running .exe with a newer release. `Installed` means the caller should restart.
pub fn self_update(data_dir: &Path) -> Outcome {
    if !system::is_release() {
        return Outcome::UpToDate;
    }
    let exe = system::exe_path();
    // Left behind by the previous update, and locked until that process has exited.
    let _ = std::fs::remove_file(with_suffix(&exe, ".old"));

    let marker = data_dir.join("last-update.txt");
    let result = (|| -> Result<Option<String>, String> {
        let Some((tag, asset)) = find_update(VERSION)? else { return Ok(None) };
        if std::fs::read_to_string(&marker).is_ok_and(|installed| installed.trim() == tag) {
            // Without this, a release whose .exe reports an older version is downloaded on every start.
            applog::error("updater", format!("Not reinstalling {tag}: it was installed before but reports version {VERSION}"));
            return Ok(None);
        }
        install(&asset, &exe)?;
        let _ = std::fs::write(&marker, &tag);
        Ok(Some(tag))
    })();
    match result {
        Ok(Some(tag)) => {
            applog::info("updater", format!("Updated from {VERSION} to {tag}"));
            Outcome::Installed
        }
        Ok(None) => Outcome::UpToDate,
        Err(error) => {
            applog::warning("updater", format!("Update failed: {error}"));
            Outcome::Failed
        }
    }
}

/// The latest release and its .exe, if it is newer than `installed`.
fn find_update(installed: &str) -> Result<Option<(String, Asset)>, String> {
    let response = crate::http::get(
        &releases_url(),
        &[("Accept", "application/vnd.github+json")],
        Duration::from_secs(5),
        4 * 1024 * 1024,
    )?;
    if response.status == 404 {
        // The repository has no release yet, so there is nothing to update to.
        return Ok(None);
    }
    if response.status != 200 {
        return Err(format!("GitHub answered with HTTP status {}", response.status));
    }
    let release: Value =
        serde_json::from_slice(&response.body).map_err(|_| "GitHub's answer isn't the expected JSON".to_owned())?;
    parse_release(&release, installed)
}

fn parse_release(release: &Value, installed: &str) -> Result<Option<(String, Asset)>, String> {
    let tag = release.get("tag_name").and_then(Value::as_str).unwrap_or_default().to_owned();
    if !is_newer(&tag, installed) {
        return Ok(None);
    }
    let assets = release.get("assets").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    for asset in assets {
        let name = asset.get("name").and_then(Value::as_str).unwrap_or_default();
        if !name.to_lowercase().ends_with(".exe") {
            continue;
        }
        let url = asset
            .get("browser_download_url")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{name} has no download address"))?;
        return Ok(Some((
            tag,
            Asset {
                name: name.to_owned(),
                url: url.to_owned(),
                size: asset.get("size").and_then(Value::as_u64),
                digest: asset.get("digest").and_then(Value::as_str).map(str::to_owned),
            },
        )));
    }
    applog::warning("updater", format!("Release {tag} has no .exe attached"));
    Ok(None)
}

fn with_suffix(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn install(asset: &Asset, exe: &Path) -> Result<(), String> {
    let new = with_suffix(exe, ".new");
    let old = with_suffix(exe, ".old");
    let response = crate::http::get(&asset.url, &[], Duration::from_secs(60), DOWNLOAD_LIMIT)?;
    if response.status != 200 {
        return Err(format!("the download of {} answered with HTTP status {}", asset.name, response.status));
    }
    verify(&response.body, asset)?;
    std::fs::write(&new, &response.body).map_err(|error| error.to_string())?;
    // Windows won't overwrite or delete a running .exe, but it does allow renaming it.
    if let Err(error) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(error.to_string());
    }
    if let Err(error) = std::fs::rename(&new, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(error.to_string());
    }
    Ok(())
}

/// Reject a truncated, corrupted or non-executable download, and one that can't be checked.
pub fn verify(content: &[u8], asset: &Asset) -> Result<(), String> {
    let name = if asset.name.is_empty() { "download" } else { &asset.name };
    if asset.size.is_some_and(|size| size != content.len() as u64) || !content.starts_with(b"MZ") {
        return Err(format!("{name} is incomplete or not a Windows executable"));
    }
    // GitHub publishes a "sha256:<hex>" digest for each release asset. A download that can't be checked
    // against one isn't run.
    let Some(expected) = asset.digest.as_deref().and_then(|digest| digest.strip_prefix("sha256:")) else {
        return Err(format!("{name} comes without a SHA-256 digest to check it against"));
    };
    let actual: String = Sha256::digest(content).iter().map(|byte| format!("{byte:02x}")).collect();
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(format!("{name} doesn't match the release's SHA-256 digest"));
    }
    Ok(())
}

/// Start the newly installed .exe with the same arguments. The caller exits right after.
pub fn restart() {
    let _ = std::process::Command::new(system::exe_path()).args(std::env::args_os().skip(1)).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exe() -> Vec<u8> {
        let mut content = b"MZ".to_vec();
        content.extend([0u8; 100]);
        content
    }

    fn asset_for(content: &[u8]) -> Asset {
        let digest: String = Sha256::digest(content).iter().map(|byte| format!("{byte:02x}")).collect();
        Asset {
            name: "UpdateCheck.exe".to_owned(),
            url: String::new(),
            size: Some(content.len() as u64),
            digest: Some(format!("sha256:{digest}")),
        }
    }

    #[test]
    fn only_newer_numeric_versions_count_as_updates() {
        for (tag, installed, newer) in [
            ("v0.2.0", "0.1.0", true),
            ("v0.10.0", "0.9.0", true),
            ("v0.1.0", "0.1.0", false),
            ("v0.1", "0.1.0", false),
            ("v0.0.9", "0.1.0", false),
            ("nightly", "0.1.0", false),
            ("v1..2", "0.1.0", false),
        ] {
            assert_eq!(is_newer(tag, installed), newer, "{tag} vs {installed}");
        }
    }

    #[test]
    fn verify_accepts_matching_download() {
        assert_eq!(verify(&exe(), &asset_for(&exe())), Ok(()));
    }

    #[test]
    fn verify_rejects_truncated_download() {
        let asset = asset_for(&exe());
        assert!(verify(&exe()[..50], &asset).unwrap_err().contains("incomplete"));
    }

    #[test]
    fn verify_rejects_non_executable() {
        let content = b"<html>".to_vec();
        assert!(verify(&content, &asset_for(&content)).unwrap_err().contains("not a Windows executable"));
    }

    #[test]
    fn verify_rejects_digest_mismatch() {
        let mut asset = asset_for(&exe());
        asset.digest = Some(format!("sha256:{}", "0".repeat(64)));
        assert!(verify(&exe(), &asset).unwrap_err().contains("doesn't match"));
    }

    #[test]
    fn verify_rejects_a_download_without_a_digest() {
        let mut asset = asset_for(&exe());
        asset.digest = None;
        assert!(verify(&exe(), &asset).unwrap_err().contains("without a SHA-256 digest"));
        asset.digest = Some("md5:abc".to_owned());
        assert!(verify(&exe(), &asset).unwrap_err().contains("without a SHA-256 digest"));
    }

    #[test]
    fn the_first_exe_of_a_newer_release_is_picked() {
        let release = json!({
            "tag_name": "v9.0.0",
            "assets": [
                {"name": "notes.txt", "browser_download_url": "https://example.com/notes.txt"},
                {"name": "UpdateCheck.EXE", "browser_download_url": "https://example.com/u.exe", "size": 5, "digest": "sha256:ab"},
            ],
        });
        let (tag, asset) = parse_release(&release, "0.2.0").unwrap().unwrap();
        assert_eq!(tag, "v9.0.0");
        assert_eq!(asset.url, "https://example.com/u.exe");
        assert_eq!(asset.size, Some(5));
        assert!(parse_release(&release, "9.0").unwrap().is_none());
        assert!(parse_release(&json!({"tag_name": "v9.0.0", "assets": []}), "0.2.0").unwrap().is_none());
    }
}
