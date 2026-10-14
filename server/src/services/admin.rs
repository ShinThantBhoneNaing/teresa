//! Admin maintenance: storage reports and the levers that shrink storage.
//!
//! Authority is checked against the database, not the connection's cached
//! profile, so promoting someone with `teresad promote` takes effect on
//! their next command, without a reconnect.

use std::path::Path;

use teresa_proto::{
    validate, MaintenanceOp, MaintenanceReport, RoomKind, ServerFrame, StorageStats, Timestamp,
    User, UserRole,
};

use crate::error::{Result, ServerError};
use crate::repo::UserRow;

use super::{Core, Store};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

impl<S: Store> Core<S> {
    /// Runs one maintenance operation for an admin.
    pub async fn maintenance(
        &self,
        actor: &UserRow,
        op: MaintenanceOp,
    ) -> Result<MaintenanceReport> {
        let current = self
            .store()
            .user_by_id(actor.id)
            .await?
            .ok_or(ServerError::SessionExpired)?;
        if current.role != UserRole::Admin {
            return Err(ServerError::Forbidden("maintenance is for admins only"));
        }

        let (removed, freed_bytes) = match op {
            MaintenanceOp::Stats => (0, 0),
            MaintenanceOp::PurgeFiles { older_than_days } => {
                self.purge_files(cutoff(older_than_days)).await?
            }
            MaintenanceOp::PurgeMessages { older_than_days } => {
                let before = self.db_bytes();
                let removed = self
                    .store()
                    .delete_messages_before(cutoff(older_than_days))
                    .await?;
                // Deleted rows only become free disk once the file is rewritten.
                self.store().compact().await?;
                (removed, before.saturating_sub(self.db_bytes()))
            }
            MaintenanceOp::Compact => {
                let before = self.db_bytes();
                self.store().compact().await?;
                (0, before.saturating_sub(self.db_bytes()))
            }
        };

        if !matches!(op, MaintenanceOp::Stats) {
            tracing::info!(admin = %actor.username, ?op, removed, freed_bytes, "maintenance");
        }
        Ok(MaintenanceReport {
            op,
            removed,
            freed_bytes,
            stats: self.storage_stats().await?,
        })
    }

    /// Every account, for an admin.
    pub async fn accounts(&self, actor: &UserRow) -> Result<Vec<User>> {
        self.require_admin(actor).await?;
        Ok(self
            .store()
            .all_users()
            .await?
            .iter()
            .map(|row| row.to_user(self.hub().is_live(row.id)))
            .collect())
    }

    /// An admin deleting someone else's account.
    ///
    /// Removing yourself is refused, which also guarantees the node always
    /// keeps at least one admin: the caller is one, and stays.
    pub async fn remove_account_as(&self, actor: &UserRow, username: &str) -> Result<UserRow> {
        self.require_admin(actor).await?;
        let username = validate::username(username)?;
        if username == actor.username {
            return Err(ServerError::Forbidden(
                "you cannot remove your own account; ask another admin",
            ));
        }
        let removed = self.remove_account(&username, &actor.username).await?;
        tracing::info!(admin = %actor.username, removed = %removed.username, "account removed");
        Ok(removed)
    }

    /// Deletes an account: live terminals are told and dropped, sessions,
    /// memberships and invites go, and every room it was in hears it leave.
    /// Messages it wrote stay in history under its old name. `by` names who
    /// did it, for the room narration.
    pub async fn remove_account(&self, username: &str, by: &str) -> Result<UserRow> {
        let username = validate::username(username)?;
        let user = self
            .store()
            .user_by_username(&username)
            .await?
            .ok_or(ServerError::NotFound("user"))?;
        let rooms = self.store().joined_by(user.id).await?;

        self.hub().send_user(
            user.id,
            ServerFrame::Kicked {
                reason: format!("your account @{} was removed by @{by}", user.username),
            },
        );
        self.store().delete_user(user.id).await?;

        for joined in rooms {
            let room = joined.room.id;
            self.hub().publish_room(
                room,
                ServerFrame::MemberLeft {
                    room,
                    user: user.id,
                },
            );
            if joined.room.kind == RoomKind::Channel {
                self.system_line(
                    room,
                    format!("@{} was removed from the node", user.username),
                )
                .await?;
            }
        }
        Ok(user)
    }

    /// Grants admin to an existing account — the operator's way in, used by
    /// `teresad promote`. Returns the updated row.
    pub async fn promote(&self, username: &str) -> Result<UserRow> {
        let username = validate::username(username)?;
        let mut user = self
            .store()
            .user_by_username(&username)
            .await?
            .ok_or(ServerError::NotFound("user"))?;
        self.store().set_role(user.id, UserRole::Admin).await?;
        user.role = UserRole::Admin;
        Ok(user)
    }

    /// Authority is read fresh from the database, never from the caller's
    /// cached row, so promotions and removals apply at once.
    async fn require_admin(&self, actor: &UserRow) -> Result<()> {
        let current = self
            .store()
            .user_by_id(actor.id)
            .await?
            .ok_or(ServerError::SessionExpired)?;
        if current.role == UserRole::Admin {
            Ok(())
        } else {
            Err(ServerError::Forbidden("admins only"))
        }
    }

    /// What the node is holding.
    pub async fn storage_stats(&self) -> Result<StorageStats> {
        let (users, messages, files, file_bytes) = self.store().storage_counts().await?;
        Ok(StorageStats {
            users,
            messages,
            files,
            file_bytes,
            db_bytes: self.db_bytes(),
        })
    }

    /// Deletes shared files older than `cutoff`, bytes first, then records.
    async fn purge_files(&self, cutoff: Timestamp) -> Result<(u64, u64)> {
        let doomed = self.store().files_before(cutoff).await?;
        let dir = self.files_dir();
        let mut freed = 0;
        for file in &doomed {
            let path = dir.join(file.id.to_string());
            let size = tokio::fs::metadata(&path).await.map_or(0, |m| m.len());
            match tokio::fs::remove_file(&path).await {
                Ok(()) => freed += size,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(ServerError::Internal(anyhow::anyhow!(
                        "removing {}: {e}",
                        path.display()
                    )))
                }
            }
        }
        let ids: Vec<_> = doomed.iter().map(|f| f.id).collect();
        let removed = self.store().delete_files(&ids).await?;
        Ok((removed, freed))
    }

    /// The database's footprint: main file plus its WAL and shared-memory files.
    fn db_bytes(&self) -> u64 {
        let db = &self.config().database;
        let size = |p: &Path| std::fs::metadata(p).map_or(0, |m| m.len());
        let sidecar = |suffix: &str| {
            let mut name = db.as_os_str().to_owned();
            name.push(suffix);
            size(Path::new(&name))
        };
        size(db) + sidecar("-wal") + sidecar("-shm")
    }
}

/// The instant "older than `days`" begins; 0 days means everything.
fn cutoff(days: u32) -> Timestamp {
    if days == 0 {
        Timestamp::from_millis(i64::MAX)
    } else {
        Timestamp::from_millis(Timestamp::now().as_millis() - i64::from(days) * DAY_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit;
    use super::*;
    use crate::repo::sqlite::SqliteStore;
    use crate::repo::{RoomRepo, UserRepo};
    use crate::services::files::UploadMeta;
    use teresa_proto::{FileCard, FileKind, MessageKind};

    async fn actor(core: &Core<SqliteStore>, name: &str) -> UserRow {
        let (token, _) = testkit::register(core, name).await;
        core.authenticate(&token).await.unwrap()
    }

    #[tokio::test]
    async fn only_admins_may_maintain_and_promotion_is_immediate() {
        let core = testkit::core().await;
        let _first = actor(&core, "pintkuu").await; // first account: admin
        let friday = actor(&core, "friday").await;

        let refused = core
            .maintenance(&friday, MaintenanceOp::Stats)
            .await
            .unwrap_err();
        assert!(matches!(refused, ServerError::Forbidden(_)));

        core.promote("Friday").await.unwrap();
        // Same cached row as the live connection holds: no reconnect needed.
        let report = core
            .maintenance(&friday, MaintenanceOp::Stats)
            .await
            .unwrap();
        assert_eq!(report.stats.users, 2);
        assert!(core.promote("nobody").await.is_err());
    }

    #[tokio::test]
    async fn an_admin_removes_an_account_which_is_kicked_and_forgotten() {
        let core = testkit::core().await;
        let admin = actor(&core, "friday").await; // first account: admin
        let (bob_token, _) = testkit::register(&core, "bob").await;
        let bob = core.authenticate(&bob_token).await.unwrap();
        let general = core.store().room_by_name("general").await.unwrap().unwrap();
        core.post_message(&bob, general.id, "my last words", MessageKind::Chat)
            .await
            .unwrap();

        // Bob's terminal is live.
        let (control_tx, mut control) = tokio::sync::mpsc::channel(8);
        let conn = teresa_proto::ConnectionId::new();
        core.connect(&bob, conn, control_tx).await.unwrap();
        let mut room_feed = core.hub().subscribe_room(general.id);

        // Members may not, and nobody may remove themselves.
        assert!(matches!(
            core.remove_account_as(&bob, "friday").await.unwrap_err(),
            ServerError::Forbidden(_)
        ));
        assert!(matches!(
            core.remove_account_as(&admin, "friday").await.unwrap_err(),
            ServerError::Forbidden(_)
        ));

        let gone = core
            .remove_account_as(&admin, "@bob".trim_start_matches('@'))
            .await
            .unwrap();
        assert_eq!(gone.username, "bob");

        // Told, then locked out.
        let kicked = loop {
            let frame = control.recv().await.unwrap();
            if let ServerFrame::Kicked { reason } = frame.as_ref() {
                break reason.clone();
            }
        };
        assert!(kicked.contains("@friday"));
        assert!(core.authenticate(&bob_token).await.is_err());
        assert!(core
            .store()
            .user_by_username("bob")
            .await
            .unwrap()
            .is_none());

        // The room hears him leave; his words stay.
        let left = loop {
            if let ServerFrame::MemberLeft { user, .. } = room_feed.recv().await.unwrap().as_ref() {
                break *user;
            }
        };
        assert_eq!(left, bob.id);
        let (history, _) = core.history(&admin, general.id, None, 50).await.unwrap();
        assert!(history.iter().any(|m| m.body == "my last words"));

        let names: Vec<String> = core
            .accounts(&admin)
            .await
            .unwrap()
            .into_iter()
            .map(|u| u.username)
            .collect();
        assert_eq!(names, ["friday"]);
        assert!(
            core.remove_account_as(&admin, "bob").await.is_err(),
            "already gone"
        );
    }

    #[tokio::test]
    async fn purging_files_frees_disk_and_records_but_keeps_history() {
        let core = testkit::core().await;
        let admin = actor(&core, "admin").await;
        let general = core.store().room_by_name("general").await.unwrap().unwrap();
        let body = futures::stream::iter(vec![Ok::<_, String>(vec![9u8; 5000])]);
        let posted = core
            .receive_file(
                &admin,
                general.id,
                UploadMeta {
                    name: "dump.sql".into(),
                    kind: FileKind::File,
                    entries: None,
                },
                body,
            )
            .await
            .unwrap();
        let card = FileCard::from_body(&posted.body).unwrap();

        // Too young for a 30-day purge.
        let report = core
            .maintenance(
                &admin,
                MaintenanceOp::PurgeFiles {
                    older_than_days: 30,
                },
            )
            .await
            .unwrap();
        assert_eq!((report.removed, report.stats.files), (0, 1));

        let report = core
            .maintenance(&admin, MaintenanceOp::PurgeFiles { older_than_days: 0 })
            .await
            .unwrap();
        assert_eq!(report.removed, 1);
        assert_eq!(report.freed_bytes, 5000);
        assert_eq!((report.stats.files, report.stats.file_bytes), (0, 0));
        assert!(core.open_file(&admin, card.id).await.is_err());

        let (history, _) = core.history(&admin, general.id, None, 10).await.unwrap();
        assert!(history.iter().any(|m| m.kind == MessageKind::File));
    }

    #[tokio::test]
    async fn purging_messages_respects_the_age_threshold() {
        let core = testkit::core().await;
        let admin = actor(&core, "admin").await;
        let general = core.store().room_by_name("general").await.unwrap().unwrap();
        for i in 0..3 {
            core.post_message(&admin, general.id, &format!("m{i}"), MessageKind::Chat)
                .await
                .unwrap();
        }
        let before = core.storage_stats().await.unwrap().messages;
        assert!(before >= 3);

        let kept = core
            .maintenance(&admin, MaintenanceOp::PurgeMessages { older_than_days: 7 })
            .await
            .unwrap();
        assert_eq!(kept.removed, 0);

        let wiped = core
            .maintenance(&admin, MaintenanceOp::PurgeMessages { older_than_days: 0 })
            .await
            .unwrap();
        assert_eq!(wiped.removed, before);
        assert_eq!(wiped.stats.messages, 0);

        core.maintenance(&admin, MaintenanceOp::Compact)
            .await
            .unwrap();
    }
}
