//! Client self-update: the node hosts prebuilt `teresa` binaries, so a
//! teammate picks up a new feature with `teresa update` instead of a Rust
//! toolchain and `cargo install`.
//!
//! The deploy script writes `manifest.json` into the node's releases
//! directory; the server only ever reads it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// `GET /api/v1/client/latest` — the client build the node is handing out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientRelease {
    /// Build stamp, e.g. `2026-10-15.bd0f6dc`. Clients compare it for
    /// equality only: any difference from their own stamp is an update.
    pub build: String,
    /// One binary per platform key (see [`platform_key`]).
    pub platforms: BTreeMap<String, ReleaseAsset>,
}

/// One downloadable binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    /// File name inside the releases directory.
    pub file: String,
    /// Hex SHA-256 of the file, checked by the client before installing.
    pub sha256: String,
    /// Size in bytes.
    pub size: u64,
}

/// The platform key for an OS and CPU, in `std::env::consts` spelling:
/// `macos-aarch64`, `linux-x86_64`, `windows-x86_64`, …
#[must_use]
pub fn platform_key(os: &str, arch: &str) -> String {
    format!("{os}-{arch}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_the_deploy_script_writes_parses() {
        let raw = r#"{
            "build": "2026-10-15.bd0f6dc",
            "platforms": {
                "macos-aarch64": {"file": "teresa-macos-aarch64", "sha256": "ab", "size": 7}
            }
        }"#;
        let release: ClientRelease = serde_json::from_str(raw).unwrap();
        assert_eq!(release.build, "2026-10-15.bd0f6dc");
        assert_eq!(
            release.platforms[&platform_key("macos", "aarch64")].file,
            "teresa-macos-aarch64"
        );
    }
}
