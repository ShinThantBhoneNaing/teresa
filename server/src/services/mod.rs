//! The application core.
//!
//! [`Core`] is the composition root's product: every rule about who may do
//! what, and every fan-out decision, lives in its methods. The API layer above
//! translates HTTP and WebSocket into calls on it; the repositories below
//! translate its needs into SQL. Neither direction contains policy.
//!
//! The methods are grouped by domain into submodules — [`auth`], [`rooms`],
//! [`messages`], [`presence`] — but they are all `impl Core<S>`, because the
//! domains genuinely interlock: posting a message consults membership,
//! presence changes touch the user table, joining a room posts a system line.
//! One struct with grouped impls states that honestly; four structs holding
//! clones of each other would not.

pub mod admin;
pub mod auth;
pub mod files;
pub mod messages;
pub mod presence;
pub mod rooms;

use teresa_proto::{ServerInfo, Timestamp, PROTOCOL_VERSION};

use crate::config::ServerConfig;
use crate::error::Result;
use crate::hub::Hub;
use crate::repo::{FileRepo, MaintenanceRepo, MessageRepo, RoomRepo, SessionRepo, UserRepo};
use crate::security::PasswordHasherService;

/// Everything the application needs from persistence, as one bound.
///
/// `Clone + Send + Sync + 'static` because the store is shared across
/// connection actors; the SQLite adapter meets this by cloning its pool.
pub trait Store:
    UserRepo
    + SessionRepo
    + RoomRepo
    + MessageRepo
    + FileRepo
    + MaintenanceRepo
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<T> Store for T where
    T: UserRepo
        + SessionRepo
        + RoomRepo
        + MessageRepo
        + FileRepo
        + MaintenanceRepo
        + Clone
        + Send
        + Sync
        + 'static
{
}

/// The assembled application.
#[derive(Debug)]
pub struct Core<S> {
    store: S,
    hub: Hub,
    hasher: PasswordHasherService,
    config: ServerConfig,
}

impl<S: Store> Core<S> {
    /// Assembles the application over a store and configuration.
    #[must_use]
    pub fn new(store: S, config: ServerConfig) -> Self {
        Self {
            store,
            hub: Hub::new(),
            hasher: PasswordHasherService::new(),
            config,
        }
    }

    /// The fan-out hub, for connection actors to subscribe through.
    #[must_use]
    pub const fn hub(&self) -> &Hub {
        &self.hub
    }

    /// The live configuration.
    #[must_use]
    pub const fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// The store, for health checks and metrics.
    #[must_use]
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Identity block sent in every `Ready` frame.
    #[must_use]
    pub fn server_info(&self) -> ServerInfo {
        ServerInfo {
            name: self.config.name.clone(),
            version: crate::VERSION.to_owned(),
            motd: self.config.motd.clone(),
            protocol: PROTOCOL_VERSION,
        }
    }

    /// First-boot work: creates the configured seed rooms if they are missing.
    ///
    /// Idempotent by construction — it checks before inserting — so calling it
    /// on every startup is the intended use.
    pub async fn bootstrap(&self) -> Result<()> {
        for name in &self.config.seed_rooms {
            if self.store.room_by_name(name).await?.is_none() {
                let room = crate::repo::RoomRow {
                    id: teresa_proto::RoomId::new(),
                    name: name.clone(),
                    topic: None,
                    kind: teresa_proto::RoomKind::Channel,
                    visibility: teresa_proto::Visibility::Public,
                    owner_id: None,
                    created_at: Timestamp::now(),
                };
                self.store.insert_room(&room, None).await?;
                tracing::info!(room = %name, "seeded room");
            }
        }
        Ok(())
    }

    /// Periodic maintenance: expired sessions, idle fan-out channels.
    ///
    /// The caller decides the cadence; every few minutes is plenty.
    pub async fn housekeeping(&self) -> Result<()> {
        let purged = self.store.purge_expired(Timestamp::now()).await?;
        if purged > 0 {
            tracing::info!(sessions = purged, "purged expired sessions");
        }
        self.hub.prune();
        self.sweep_orphaned_files().await;
        Ok(())
    }
}

/// A timestamp `duration` in the future of `from` — session expiry arithmetic.
pub(crate) fn after(from: Timestamp, duration: std::time::Duration) -> Timestamp {
    let millis = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
    Timestamp::from_millis(from.as_millis().saturating_add(millis))
}

#[cfg(test)]
pub(crate) mod testkit {
    //! Shared scaffolding for the service tests.

    use crate::repo::sqlite::SqliteStore;

    use super::*;

    #[allow(unused_imports)]
    pub(crate) use crate::repo::{
        FileRepo, MaintenanceRepo, MessageRepo, RoomRepo, SessionRepo, UserRepo,
    };

    /// A core over a fresh in-memory database with default (loopback) config,
    /// seed rooms already planted.
    pub async fn core() -> Core<SqliteStore> {
        let store = SqliteStore::memory().await.expect("in-memory store opens");
        let config = ServerConfig {
            files_dir: Some(
                std::env::temp_dir()
                    .join(format!("teresa-test-files-{}", teresa_proto::FileId::new())),
            ),
            ..ServerConfig::default()
        };
        let core = Core::new(store, config);
        core.bootstrap().await.expect("bootstrap succeeds");
        core
    }

    /// Registers an account and returns (token, wire user).
    pub async fn register(core: &Core<SqliteStore>, name: &str) -> (String, teresa_proto::User) {
        core.register(name, "a sufficiently long password", name)
            .await
            .unwrap_or_else(|e| panic!("registering {name} failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::RoomRepo;

    #[tokio::test]
    async fn bootstrap_seeds_rooms_exactly_once() {
        let core = testkit::core().await;
        // A second bootstrap must not conflict with the first.
        core.bootstrap().await.expect("bootstrap is idempotent");

        let room = core
            .store()
            .room_by_name("general")
            .await
            .unwrap()
            .expect("#general exists");
        assert!(room.owner_id.is_none(), "seed rooms belong to nobody");
    }

    #[tokio::test]
    async fn server_info_reports_the_protocol() {
        let core = testkit::core().await;
        let info = core.server_info();
        assert_eq!(info.protocol, PROTOCOL_VERSION);
        assert_eq!(info.name, "teresa");
    }

    #[test]
    fn after_saturates_rather_than_wrapping() {
        let far = after(
            Timestamp::from_millis(i64::MAX - 5),
            std::time::Duration::from_secs(60),
        );
        assert_eq!(far.as_millis(), i64::MAX);
    }
}
