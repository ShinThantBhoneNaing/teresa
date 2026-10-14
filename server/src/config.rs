//! Server configuration.
//!
//! Precedence, lowest to highest: built-in defaults, then `server.toml`, then
//! `TERESA_*` environment variables. The file is for operators; the
//! environment overlay is for containers and CI, where editing files is the
//! wrong tool.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

/// Everything `teresad` can be told at startup.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Interface to bind.
    pub bind: IpAddr,
    /// Port to listen on.
    pub port: u16,
    /// Name shown in every client's header bar.
    pub name: String,
    /// Message of the day, surfaced to clients on connect.
    pub motd: Option<String>,
    /// Path of the SQLite database. Created on first run if absent.
    pub database: PathBuf,
    /// Where shared files are kept. Defaults to `files/` beside the database.
    pub files_dir: Option<PathBuf>,
    /// TLS settings.
    pub tls: TlsConfig,
    /// Rate-limit settings.
    pub limits: LimitsConfig,
    /// Session lifetime settings.
    pub sessions: SessionConfig,
    /// Whether new accounts may register themselves.
    ///
    /// Off, an operator seeds accounts and the register endpoint refuses —
    /// the right posture for a private team server exposed to the internet.
    pub open_registration: bool,
    /// Rooms created automatically on first boot.
    pub seed_rooms: Vec<String>,
    /// Log filter, in `tracing_subscriber::EnvFilter` syntax.
    pub log: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 7737,
            name: "teresa".to_owned(),
            motd: None,
            database: PathBuf::from("teresa.db"),
            files_dir: None,
            tls: TlsConfig::default(),
            limits: LimitsConfig::default(),
            sessions: SessionConfig::default(),
            open_registration: true,
            seed_rooms: vec![
                "general".to_owned(),
                "backend".to_owned(),
                "frontend".to_owned(),
                "security".to_owned(),
                "devops".to_owned(),
                "random".to_owned(),
            ],
            log: "teresa_server=info,tower_http=warn".to_owned(),
        }
    }
}

/// TLS material. Both paths must be set together or not at all.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TlsConfig {
    /// PEM certificate chain.
    pub cert: Option<PathBuf>,
    /// PEM private key.
    pub key: Option<PathBuf>,
}

impl TlsConfig {
    /// Whether TLS is fully configured.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.cert.is_some() && self.key.is_some()
    }
}

/// Rate limits. Zeroes are rejected at load rather than treated as "unlimited",
/// because an operator who wants no limit should have to say a large number
/// out loud.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LimitsConfig {
    /// Login and register attempts allowed per IP per minute.
    pub auth_per_minute: u32,
    /// Messages allowed per connection per ten seconds.
    pub messages_per_10s: u32,
    /// Outbound frames buffered per connection before it is dropped as too slow.
    pub outbox_depth: usize,
    /// Maximum simultaneous connections per user account.
    pub connections_per_user: usize,
    /// Largest accepted upload, in MiB.
    pub upload_max_mb: u64,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            auth_per_minute: 5,
            messages_per_10s: 20,
            outbox_depth: 256,
            connections_per_user: 8,
            upload_max_mb: 4096,
        }
    }
}

/// Session lifetime.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    /// Days a session survives without use.
    pub idle_days: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self { idle_days: 30 }
    }
}

impl ServerConfig {
    /// The socket address to bind.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind, self.port)
    }

    /// The directory shared files are stored in.
    #[must_use]
    pub fn files_path(&self) -> PathBuf {
        self.files_dir.clone().unwrap_or_else(|| {
            self.database
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("files"), |p| p.join("files"))
        })
    }

    /// Largest accepted upload, in bytes.
    #[must_use]
    pub const fn upload_max_bytes(&self) -> u64 {
        self.limits.upload_max_mb.saturating_mul(1024 * 1024)
    }

    /// How long a session may sit idle before it expires.
    #[must_use]
    pub const fn session_idle(&self) -> Duration {
        Duration::from_secs(self.sessions.idle_days as u64 * 24 * 60 * 60)
    }

    /// Loads configuration from an optional file plus the environment.
    ///
    /// A missing file is fine — defaults are serviceable for local use. A file
    /// that exists but does not parse is a hard error: silently ignoring a typo
    /// in `server.toml` is how a server ends up on the wrong port with no TLS.
    ///
    /// # Errors
    /// Returns an error if the file is unreadable or invalid, or if an
    /// environment override cannot be parsed.
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let mut config = match path {
            Some(path) => {
                let raw = std::fs::read_to_string(path)
                    .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
                toml::from_str(&raw)
                    .map_err(|e| anyhow::anyhow!("cannot parse {}: {e}", path.display()))?
            }
            None => Self::default(),
        };
        config.apply_env()?;
        config.validate()?;
        Ok(config)
    }

    /// Applies `TERESA_*` environment overrides.
    fn apply_env(&mut self) -> anyhow::Result<()> {
        /// Parses one override, naming the variable in the error it returns.
        fn parse<T: std::str::FromStr>(name: &str) -> anyhow::Result<Option<T>>
        where
            T::Err: std::fmt::Display,
        {
            match std::env::var(name) {
                Ok(raw) => raw
                    .parse()
                    .map(Some)
                    .map_err(|e| anyhow::anyhow!("invalid {name}: {e}")),
                Err(_) => Ok(None),
            }
        }

        if let Some(bind) = parse("TERESA_BIND")? {
            self.bind = bind;
        }
        if let Some(port) = parse("TERESA_PORT")? {
            self.port = port;
        }
        if let Some(database) = parse::<PathBuf>("TERESA_DB")? {
            self.database = database;
        }
        if let Some(files) = parse::<PathBuf>("TERESA_FILES_DIR")? {
            self.files_dir = Some(files);
        }
        if let Some(log) = parse::<String>("TERESA_LOG")? {
            self.log = log;
        }
        if let Some(open) = parse("TERESA_OPEN_REGISTRATION")? {
            self.open_registration = open;
        }
        if let Some(cert) = parse::<PathBuf>("TERESA_TLS_CERT")? {
            self.tls.cert = Some(cert);
        }
        if let Some(key) = parse::<PathBuf>("TERESA_TLS_KEY")? {
            self.tls.key = Some(key);
        }
        Ok(())
    }

    /// Rejects configurations that would misbehave quietly.
    fn validate(&self) -> anyhow::Result<()> {
        if self.tls.cert.is_some() != self.tls.key.is_some() {
            anyhow::bail!("tls.cert and tls.key must be set together");
        }
        if !self.tls.enabled() && !self.bind.is_loopback() {
            // Not an error: a reverse proxy terminating TLS in front of a
            // plaintext bind is a legitimate deployment. But it must be a
            // choice someone can see in the log, not a silent default.
            tracing::warn!(
                bind = %self.bind,
                "listening on a non-loopback address without TLS; \
                 credentials will cross the network unencrypted unless a proxy terminates TLS"
            );
        }
        if self.limits.auth_per_minute == 0
            || self.limits.messages_per_10s == 0
            || self.limits.outbox_depth == 0
            || self.limits.connections_per_user == 0
            || self.limits.upload_max_mb == 0
        {
            anyhow::bail!(
                "limits must be positive; use a large value to mean 'effectively unlimited'"
            );
        }
        if self.sessions.idle_days == 0 {
            anyhow::bail!("sessions.idle_days must be positive");
        }
        for name in &self.seed_rooms {
            teresa_proto::validate::room_name(name)
                .map_err(|e| anyhow::anyhow!("seed room {name:?}: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        ServerConfig::default()
            .validate()
            .expect("defaults must validate");
    }

    #[test]
    fn defaults_bind_loopback_only() {
        // Binding a fresh install to 0.0.0.0 without TLS would be a footgun.
        assert!(ServerConfig::default().bind.is_loopback());
    }

    #[test]
    fn a_full_file_parses() {
        let config: ServerConfig = toml::from_str(
            r#"
            bind = "0.0.0.0"
            port = 8443
            name = "zion"
            motd = "no system is safe"
            database = "/var/lib/teresa/chat.db"
            open_registration = false
            seed_rooms = ["general", "ops"]
            log = "debug"

            [tls]
            cert = "/etc/teresa/cert.pem"
            key = "/etc/teresa/key.pem"

            [limits]
            auth_per_minute = 10
            messages_per_10s = 30
            outbox_depth = 512
            connections_per_user = 4
            upload_max_mb = 512

            [sessions]
            idle_days = 7
            "#,
        )
        .expect("full config must parse");

        assert_eq!(config.port, 8443);
        assert_eq!(config.name, "zion");
        assert!(config.tls.enabled());
        assert!(!config.open_registration);
        assert_eq!(config.upload_max_bytes(), 512 * 1024 * 1024);
        assert_eq!(
            config.files_path(),
            PathBuf::from("/var/lib/teresa/files"),
            "files default to living beside the database"
        );
        assert_eq!(config.session_idle(), Duration::from_secs(7 * 24 * 60 * 60));
        config.validate().expect("must validate");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        // `prot = 8080` must fail loudly, not run on the default port.
        let result = toml::from_str::<ServerConfig>("prot = 8080");
        assert!(result.is_err());
    }

    #[test]
    fn half_configured_tls_is_rejected() {
        let config = ServerConfig {
            tls: TlsConfig {
                cert: Some(PathBuf::from("cert.pem")),
                key: None,
            },
            ..ServerConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_limits_are_rejected() {
        let mut config = ServerConfig::default();
        config.limits.messages_per_10s = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_seed_rooms_are_rejected() {
        let config = ServerConfig {
            seed_rooms: vec!["general".into(), "not a room!".into()],
            ..ServerConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn partial_files_keep_defaults_for_the_rest() {
        let config: ServerConfig = toml::from_str("port = 9000").expect("partial config parses");
        assert_eq!(config.port, 9000);
        assert_eq!(config.name, "teresa");
        assert_eq!(config.limits.messages_per_10s, 20);
    }
}
