//! Local session storage: `~/.config/teresa/session.toml`.
//!
//! Holds the bearer token and enough identity to greet the user before the
//! server answers. Never the password — the client forgets that the moment
//! the register/login call returns.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::ClientConfig;

/// A remembered login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// The bearer token, exactly as the server minted it.
    pub token: String,
    /// Handle, for the pre-connect UI.
    pub username: String,
    /// Display name, for the pre-connect UI.
    pub display_name: String,
    /// Which server this session belongs to, so switching servers in the
    /// config invalidates the session instead of sending the token to the
    /// wrong host.
    pub server: String,
}

impl Session {
    /// The default session file path.
    #[must_use]
    pub fn default_path() -> PathBuf {
        ClientConfig::state_dir().join("session.toml")
    }

    /// Loads the session, if one is stored and it belongs to `server`.
    #[must_use]
    pub fn load(path: &Path, server: &str) -> Option<Self> {
        let raw = std::fs::read_to_string(path).ok()?;
        let session: Self = toml::from_str(&raw).ok()?;
        if session.server == server {
            Some(session)
        } else {
            // A session for some other host is not an error, just not usable —
            // and must NOT be presented to the host we are talking to now.
            None
        }
    }

    /// Stores the session with owner-only permissions.
    ///
    /// # Errors
    /// Fails if the directory cannot be created or the file cannot be written.
    ///
    /// # Panics
    /// Never in practice: serialising this struct to TOML cannot fail.
    pub fn store(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self).expect("session serialises");
        std::fs::write(path, body)?;

        // 0600: the token is a credential. Windows has no mode bits; there the
        // profile directory's ACL is the protection, which matches what every
        // other credential-storing CLI does.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Deletes the stored session — logout.
    ///
    /// # Errors
    /// Fails only on a real filesystem error; a missing file is already the
    /// desired state.
    pub fn clear(path: &Path) -> anyhow::Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        Session {
            token: "secret-token".into(),
            username: "neo".into(),
            display_name: "Neo".into(),
            server: "127.0.0.1".into(),
        }
    }

    #[test]
    fn stores_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");

        sample().store(&path).unwrap();
        assert_eq!(Session::load(&path, "127.0.0.1"), Some(sample()));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");

        sample().store(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "session file must be 0600");
    }

    #[test]
    fn a_session_for_another_server_is_not_offered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        sample().store(&path).unwrap();

        assert_eq!(Session::load(&path, "10.0.0.9"), None);
    }

    #[test]
    fn clear_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        sample().store(&path).unwrap();

        Session::clear(&path).unwrap();
        assert!(Session::load(&path, "127.0.0.1").is_none());
        Session::clear(&path).unwrap(); // second time: nothing to do, no error
    }

    #[test]
    fn corrupt_files_read_as_no_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "not valid toml [[[").unwrap();
        assert!(Session::load(&path, "127.0.0.1").is_none());
    }
}
