#![allow(clippy::disallowed_types)] // serde needed for GitHub API deserialization

use color_eyre::{eyre::eyre, Result};
use owo_colors::OwoColorize;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const GITHUB_API_URL: &str = "https://api.github.com/repos/bearcove/dodeca/releases/latest";

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// Get the current platform target triple
fn get_platform_target() -> Result<&'static str> {
    // Determine target triple based on compile-time cfg
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Ok("x86_64-unknown-linux-gnu");

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Ok("aarch64-unknown-linux-gnu");

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Ok("x86_64-apple-darwin");

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok("aarch64-apple-darwin");

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Ok("x86_64-pc-windows-msvc");

    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return Ok("aarch64-pc-windows-msvc");

    #[allow(unreachable_code)]
    Err(eyre!("Unsupported platform for self-update"))
}

/// Parse version string (removes 'v' prefix if present)
fn parse_version(version: &str) -> &str {
    version.strip_prefix('v').unwrap_or(version)
}

/// Compare version strings (simple lexicographic comparison)
fn is_newer_version(current: &str, latest: &str) -> bool {
    let current = parse_version(current);
    let latest = parse_version(latest);

    // Parse as semver-like strings
    let current_parts: Vec<u32> = current
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let latest_parts: Vec<u32> = latest
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();

    // Compare version parts
    for i in 0..std::cmp::max(current_parts.len(), latest_parts.len()) {
        let current_part = current_parts.get(i).copied().unwrap_or(0);
        let latest_part = latest_parts.get(i).copied().unwrap_or(0);

        if latest_part > current_part {
            return true;
        } else if latest_part < current_part {
            return false;
        }
    }

    false
}

/// Fetch the latest release from GitHub
async fn fetch_latest_release() -> Result<GitHubRelease> {
    let client = reqwest::Client::builder()
        .user_agent(format!("dodeca/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    let response = client.get(GITHUB_API_URL).send().await?;

    if !response.status().is_success() {
        return Err(eyre!(
            "Failed to fetch latest release: HTTP {}",
            response.status()
        ));
    }

    let release: GitHubRelease = response.json().await?;
    Ok(release)
}

/// Download a file from a URL
async fn download_file(url: &str, dest: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent(format!("dodeca/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(eyre!("Failed to download file: HTTP {}", response.status()));
    }

    let bytes = response.bytes().await?;
    fs::write(dest, bytes)?;

    Ok(())
}

/// Extract a .tar.xz archive
fn extract_tar_xz(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    use std::process::Command;

    // Use system tar command for simplicity (works on Unix and modern Windows)
    let status = Command::new("tar")
        .arg("xf")
        .arg(archive_path)
        .arg("-C")
        .arg(dest_dir)
        .status()?;

    if !status.success() {
        return Err(eyre!("Failed to extract archive"));
    }

    Ok(())
}

/// Get the current executable path
fn get_current_exe() -> Result<PathBuf> {
    env::current_exe().map_err(|e| eyre!("Failed to get current executable path: {}", e))
}

/// Get the directory containing the executable
fn get_exe_dir() -> Result<PathBuf> {
    let exe = get_current_exe()?;
    exe.parent()
        .ok_or_else(|| eyre!("Failed to get executable directory"))
        .map(|p| p.to_path_buf())
}

/// Replace the current binary with a new one (handles running binary issue)
fn replace_binary(new_binary: &Path, current_binary: &Path) -> Result<()> {
    // On Unix, we can rename the running binary (it stays in memory)
    // On Windows, we need a different approach

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        // Backup the old binary
        let backup = current_binary.with_extension("old");
        if backup.exists() {
            fs::remove_file(&backup)?;
        }
        fs::rename(current_binary, &backup)?;

        // Copy new binary
        fs::copy(new_binary, current_binary)?;

        // Set executable permissions
        let mut perms = fs::metadata(current_binary)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(current_binary, perms)?;

        println!("  {} Old binary backed up to: {}", "✓".green(), backup.display());
    }

    #[cfg(windows)]
    {
        // On Windows, we can't replace a running binary
        // Instead, we create a .new file and tell the user to restart
        let new_path = current_binary.with_extension("exe.new");
        fs::copy(new_binary, &new_path)?;

        println!("  {} New binary saved to: {}", "✓".green(), new_path.display());
        println!("\n{}", "NOTE:".yellow().bold());
        println!("  Windows cannot replace a running binary.");
        println!("  The update has been downloaded to: {}", new_path.display());
        println!("  Please rename it to replace the current binary after exit.");
    }

    Ok(())
}

/// Perform the self-update
pub async fn self_update() -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");

    println!("{}", "Checking for updates...".cyan().bold());
    println!("  Current version: {}", current_version.yellow());

    // Fetch latest release
    let release = fetch_latest_release().await?;
    let latest_version = parse_version(&release.tag_name);

    println!("  Latest version:  {}", latest_version.yellow());

    // Check if update is needed
    if !is_newer_version(current_version, latest_version) {
        println!("\n{}", "Already up to date!".green().bold());
        return Ok(());
    }

    println!("\n{}", "Update available!".green().bold());

    // Get platform target
    let target = get_platform_target()?;
    println!("  Platform: {}", target.cyan());

    // Find the appropriate asset
    let archive_name = format!("dodeca-{}.tar.xz", target);
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == archive_name)
        .ok_or_else(|| eyre!("No release asset found for platform: {}", target))?;

    println!("  Archive:  {}", asset.name.cyan());
    println!("\n{}", "Downloading update...".cyan().bold());

    // Create temp directory
    let temp_dir = env::temp_dir().join(format!("dodeca-update-{}", latest_version));
    fs::create_dir_all(&temp_dir)?;

    // Download archive
    let archive_path = temp_dir.join(&archive_name);
    download_file(&asset.browser_download_url, &archive_path).await?;
    println!("  {} Downloaded: {}", "✓".green(), archive_path.display());

    // Extract archive
    println!("\n{}", "Extracting archive...".cyan().bold());
    extract_tar_xz(&archive_path, &temp_dir)?;
    println!("  {} Extracted to: {}", "✓".green(), temp_dir.display());

    // Find the binary in extracted files
    let binary_name = if cfg!(windows) { "ddc.exe" } else { "ddc" };
    let new_binary = temp_dir.join(binary_name);

    if !new_binary.exists() {
        return Err(eyre!("Binary not found in extracted archive: {}", new_binary.display()));
    }

    // Replace the current binary
    println!("\n{}", "Installing update...".cyan().bold());
    let current_binary = get_current_exe()?;
    replace_binary(&new_binary, &current_binary)?;

    // Copy plugins directory if it exists
    let new_plugins = temp_dir.join("plugins");
    if new_plugins.exists() {
        let exe_dir = get_exe_dir()?;
        let target_plugins = exe_dir.join("plugins");

        // Remove old plugins directory
        if target_plugins.exists() {
            fs::remove_dir_all(&target_plugins)?;
        }

        // Copy new plugins
        copy_dir_all(&new_plugins, &target_plugins)?;
        println!("  {} Plugins updated: {}", "✓".green(), target_plugins.display());
    }

    // Cleanup
    fs::remove_dir_all(&temp_dir)?;

    println!("\n{}", "Update complete!".green().bold());
    println!("  Updated from {} to {}", current_version.yellow(), latest_version.yellow());

    #[cfg(not(windows))]
    println!("\n{}", "You can now use the updated version.".cyan());

    #[cfg(windows)]
    println!("\n{}", "Please restart ddc to use the updated version.".cyan());

    Ok(())
}

/// Recursively copy a directory
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("v0.2.12"), "0.2.12");
        assert_eq!(parse_version("0.2.12"), "0.2.12");
        assert_eq!(parse_version("v1.0.0"), "1.0.0");
    }

    #[test]
    fn test_is_newer_version() {
        // Newer versions
        assert!(is_newer_version("0.2.12", "0.2.13"));
        assert!(is_newer_version("0.2.12", "0.3.0"));
        assert!(is_newer_version("0.2.12", "1.0.0"));
        assert!(is_newer_version("0.2.12", "v0.2.13"));

        // Same version
        assert!(!is_newer_version("0.2.12", "0.2.12"));
        assert!(!is_newer_version("0.2.12", "v0.2.12"));

        // Older versions
        assert!(!is_newer_version("0.2.12", "0.2.11"));
        assert!(!is_newer_version("0.2.12", "0.1.0"));
        assert!(!is_newer_version("1.0.0", "0.9.9"));
    }

    #[test]
    fn test_get_platform_target() {
        // This should succeed on supported platforms
        let result = get_platform_target();
        assert!(result.is_ok());
        let target = result.unwrap();
        assert!(target.contains("-"));
    }
}
