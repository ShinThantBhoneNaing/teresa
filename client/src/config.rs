//! Client configuration: `~/.config/teresa/config.toml`.
//!
//! Written with defaults on first run so the file a user opens to edit always
//! exists and shows every knob. CLI flags override the file; the file
//! overrides nothing (there is deliberately no env layer here — a terminal
//! client's environment is the user's shell, and surprising it is rude).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Everything the client can be told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // each bool is an independent user switch
pub struct ClientConfig {
    /// Server hostname or IP.
    pub server: String,
    /// Server port.
    pub port: u16,
    /// Whether to speak TLS (`https`/`wss`). Off for localhost tinkering.
    pub tls: bool,
    /// Theme name; see the `themes` module for the roster.
    pub theme: String,
    /// Master switch for the bell and title flash.
    pub notifications: bool,
    /// Ring the terminal bell on mentions and DMs.
    pub bell: bool,
    /// Flash the terminal title on unseen activity.
    pub title_flash: bool,
    /// Show `alice is typing…` lines from others.
    pub typing_indicators: bool,
    /// 12- or 24-hour clock in timestamps.
    pub clock_24h: bool,
    /// Log file for troubleshooting. Absent by default: chat clients have no
    /// business writing logs unless asked.
    pub log_file: Option<PathBuf>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server: "127.0.0.1".to_owned(),
            port: 7737,
            tls: false,
            theme: "matrix".to_owned(),
            notifications: true,
            bell: true,
            title_flash: true,
            typing_indicators: true,
            clock_24h: true,
            log_file: None,
        }
    }
}

impl ClientConfig {
    /// The directory all client state lives in.
    ///
    /// `~/.config/teresa` on Linux and macOS (XDG-style, and macOS terminal
    /// users expect dotfiles, not `~/Library`), `%APPDATA%\teresa` on
    /// Windows.
    #[must_use]
    pub fn state_dir() -> PathBuf {
        #[cfg(not(windows))]
        {
            dirs::home_dir().map_or_else(
                || PathBuf::from(".teresa"),
                |home| home.join(".config").join("teresa"),
            )
        }
        #[cfg(windows)]
        {
            dirs::config_dir().map_or_else(|| PathBuf::from(".teresa"), |dir| dir.join("teresa"))
        }
    }

    /// The default config file path.
    #[must_use]
    pub fn default_path() -> PathBuf {
        Self::state_dir().join("config.toml")
    }

    /// Loads the file, creating it with defaults if missing.
    ///
    /// # Errors
    /// Fails if the file exists but cannot be parsed — a config typo must be
    /// fixed, not shrugged off — or if the directory cannot be created.
    pub fn load_or_init(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            let raw = std::fs::read_to_string(path)?;
            toml::from_str(&raw)
                .map_err(|e| anyhow::anyhow!("config {} is invalid: {e}", path.display()))
        } else {
            let config = Self::default();
            config.write(path)?;
            Ok(config)
        }
    }

    /// Writes the config back out (used by `/theme` to persist the choice).
    ///
    /// # Errors
    /// Fails if the directory cannot be created or the file cannot be written.
    ///
    /// # Panics
    /// Never in practice: serialising this struct to TOML cannot fail.
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self).expect("config serialises");
        std::fs::write(path, format!("# TERESA client configuration\n{body}"))?;
        Ok(())
    }

    /// Base URL for REST calls.
    #[must_use]
    pub fn http_base(&self) -> String {
        let scheme = if self.tls { "https" } else { "http" };
        format!("{scheme}://{}:{}/api/v1", self.server, self.port)
    }

    /// URL for the WebSocket.
    #[must_use]
    pub fn ws_url(&self) -> String {
        let scheme = if self.tls { "wss" } else { "ws" };
        format!("{scheme}://{}:{}/api/v1/ws", self.server, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_the_tls_switch() {
        let mut config = ClientConfig::default();
        assert_eq!(config.http_base(), "http://127.0.0.1:7737/api/v1");
        assert_eq!(config.ws_url(), "ws://127.0.0.1:7737/api/v1/ws");

        config.tls = true;
        config.server = "chat.example.com".into();
        config.port = 443;
        assert_eq!(config.http_base(), "https://chat.example.com:443/api/v1");
        assert_eq!(config.ws_url(), "wss://chat.example.com:443/api/v1/ws");
    }

    #[test]
    fn first_run_writes_a_default_file_and_reloads_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let created = ClientConfig::load_or_init(&path).unwrap();
        assert_eq!(created, ClientConfig::default());
        assert!(path.exists());

        // The written file round-trips.
        let reloaded = ClientConfig::load_or_init(&path).unwrap();
        assert_eq!(reloaded, created);
    }

    #[test]
    fn a_partial_file_keeps_defaults_for_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "server = \"192.168.1.15\"\nport = 8080\ntheme = \"cyberpunk\"\n",
        )
        .unwrap();

        let config = ClientConfig::load_or_init(&path).unwrap();
        assert_eq!(config.server, "192.168.1.15");
        assert_eq!(config.port, 8080);
        assert_eq!(config.theme, "cyberpunk");
        assert!(config.notifications, "unspecified keys default");
    }

    #[test]
    fn typos_in_the_file_are_refused_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "servr = \"oops\"\n").unwrap();
        assert!(ClientConfig::load_or_init(&path).is_err());
    }
}
