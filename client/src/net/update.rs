//! Self-update from the node: `teresa update`, and the offer on launch.
//!
//! The node serves prebuilt binaries (see the server's `api::releases`), so a
//! teammate picks up a new feature without a Rust toolchain. The download is
//! checked against the manifest's SHA-256 and swapped in beside the running
//! executable, so the replacement is a rename on the same filesystem.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use teresa_proto::{platform_key, ClientRelease, ReleaseAsset};
use tokio::io::AsyncWriteExt;

use crate::config::ClientConfig;

/// Build stamp of this binary: set by the deploy script through the
/// `TERESA_BUILD` environment variable, `dev` for a plain `cargo build`.
pub const BUILD: &str = match option_env!("TERESA_BUILD") {
    Some(build) => build,
    None => "dev",
};

/// This machine's platform key, matching the manifest's.
#[must_use]
pub fn platform() -> String {
    platform_key(std::env::consts::OS, std::env::consts::ARCH)
}

/// Whether `release` is something this binary should move to: a different
/// build, available for this platform.
#[must_use]
pub fn is_update(release: &ClientRelease) -> bool {
    release.build != BUILD && release.platforms.contains_key(&platform())
}

/// Talks to the node's release routes.
#[derive(Debug, Clone)]
pub struct Updater {
    base: String,
    http: reqwest::Client,
}

impl Updater {
    /// Builds an updater for the configured node.
    ///
    /// # Panics
    /// Never in practice: the reqwest builder only fails over TLS backend
    /// misconfiguration, and the backend is compiled in.
    #[must_use]
    pub fn new(config: &ClientConfig) -> Self {
        Self {
            base: config.http_base(),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .build()
                .expect("http client builds"),
        }
    }

    /// The build the node offers, or `None` if it hosts no client builds.
    pub async fn latest(&self, timeout: Duration) -> anyhow::Result<Option<ClientRelease>> {
        let response = self
            .http
            .get(format!("{}/client/latest", self.base))
            .timeout(timeout)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let release = response.error_for_status()?.json().await?;
        Ok(Some(release))
    }

    /// Downloads this platform's binary from `release` and puts it in place
    /// of the running executable. Returns the executable's path, for a
    /// relaunch — asked for *before* the swap, since afterwards Linux reports
    /// the old inode as `… (deleted)`.
    pub async fn install(&self, release: &ClientRelease) -> anyhow::Result<PathBuf> {
        let platform = platform();
        let asset = release
            .platforms
            .get(&platform)
            .with_context(|| format!("the node has no build for {platform}"))?;

        let exe = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .context("cannot locate the running teresa")?;
        let dir = exe.parent().context("teresa has no parent directory")?;
        let staged = dir.join(format!(".teresa-update.{}", std::process::id()));

        let result = async {
            self.download(&platform, asset, &staged).await?;
            replace(&staged, &exe).map_err(|e| writable_hint(e, dir))
        }
        .await;
        if result.is_err() {
            let _ = std::fs::remove_file(&staged);
        }
        result.map(|()| exe)
    }

    async fn download(
        &self,
        platform: &str,
        asset: &ReleaseAsset,
        dest: &Path,
    ) -> anyhow::Result<()> {
        let response = self
            .http
            .get(format!("{}/client/download/{platform}", self.base))
            .send()
            .await?
            .error_for_status()?;

        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| writable_hint(e, dest.parent().unwrap_or(dest)))?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            hasher.update(&chunk);
            size += chunk.len() as u64;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        drop(file);

        let sha = format!("{:x}", hasher.finalize());
        if size != asset.size || !sha.eq_ignore_ascii_case(&asset.sha256) {
            anyhow::bail!("download corrupted (size or checksum mismatch) — try again");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }
}

/// Swaps `staged` in as `exe`.
#[cfg(not(windows))]
fn replace(staged: &Path, exe: &Path) -> std::io::Result<()> {
    // Atomic; the running process keeps its (now unlinked) inode.
    std::fs::rename(staged, exe)
}

/// Swaps `staged` in as `exe`. A running `.exe` cannot be overwritten or
/// deleted on Windows, but it can be renamed out of the way; the leftover is
/// removed by [`cleanup_previous`] on the next launch.
#[cfg(windows)]
fn replace(staged: &Path, exe: &Path) -> std::io::Result<()> {
    let old = previous_path(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old)?;
    std::fs::rename(staged, exe).inspect_err(|_| {
        let _ = std::fs::rename(&old, exe);
    })
}

#[cfg(windows)]
fn previous_path(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// Removes the binary a Windows update renamed aside. Best effort; a no-op
/// elsewhere.
pub fn cleanup_previous() {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(previous_path(&exe));
    }
}

/// Turns "permission denied" into what to do about it.
fn writable_hint(error: std::io::Error, dir: &Path) -> anyhow::Error {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        anyhow::anyhow!(
            "cannot write to {} — run the update with the rights that installed teresa there (e.g. sudo teresa update)",
            dir.display()
        )
    } else {
        error.into()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn release(build: &str, platform: &str) -> ClientRelease {
        ClientRelease {
            build: build.to_owned(),
            platforms: BTreeMap::from([(
                platform.to_owned(),
                ReleaseAsset {
                    file: "teresa-x".to_owned(),
                    sha256: String::new(),
                    size: 0,
                },
            )]),
        }
    }

    #[test]
    fn a_different_build_for_this_platform_is_an_update() {
        assert!(is_update(&release("some-other-build", &platform())));
    }

    #[test]
    fn the_same_build_is_not_an_update() {
        assert!(!is_update(&release(BUILD, &platform())));
    }

    #[test]
    fn a_build_for_another_platform_is_not_an_update() {
        assert!(!is_update(&release("some-other-build", "plan9-mips")));
    }
}
