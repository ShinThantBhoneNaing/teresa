//! Shared files: streaming bytes to disk, publishing their cards, and
//! deciding who may fetch them back.
//!
//! The bytes never pass through the database or the hub. An upload streams
//! from the socket straight to `<files_dir>/<id>.part`, hashed on the way, and
//! is renamed into place only once complete; then — and only then — a row is
//! written and an ordinary message of kind `file` is posted, so a file every
//! client saw is always a file every client can fetch.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use futures::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use teresa_proto::{file, FileId, FileKind, Message, MessageKind, RoomId, Timestamp};
use tokio::io::AsyncWriteExt;

use crate::error::{Result, ServerError};
use crate::repo::{FileRow, UserRow};

use super::{Core, Store};

/// Write buffer between the socket and the disk. Large enough that the disk
/// sees few, big writes; small enough to be irrelevant per upload.
const WRITE_BUFFER: usize = 1024 * 1024;

/// Bytes on disk with no row this old are debris from a deleted room or a
/// crashed upload. The grace period keeps the sweep away from uploads that
/// are mid-flight or between their rename and their insert.
const ORPHAN_GRACE: Duration = Duration::from_secs(30 * 60);

/// What the uploader says about the bytes it is about to send.
#[derive(Debug, Clone)]
pub struct UploadMeta {
    /// File name, validated as a single path component.
    pub name: String,
    /// File, folder or archive.
    pub kind: FileKind,
    /// Packed file count, for folders.
    pub entries: Option<u32>,
}

impl<S: Store> Core<S> {
    /// Where shared files live.
    #[must_use]
    pub fn files_dir(&self) -> PathBuf {
        self.config().files_path()
    }

    /// Receives an upload into a room and posts its card.
    ///
    /// Membership is checked before a single byte is written, so a stranger
    /// cannot use the server as free storage. The size cap is enforced while
    /// streaming, not after, for the same reason.
    pub async fn receive_file<St, B, E>(
        &self,
        actor: &UserRow,
        room: RoomId,
        meta: UploadMeta,
        body: St,
    ) -> Result<Message>
    where
        St: Stream<Item = std::result::Result<B, E>> + Unpin,
        B: AsRef<[u8]>,
        E: std::fmt::Display,
    {
        self.require_member(room, actor.id).await?;
        let name = file::file_name(&meta.name)?;

        let dir = self.files_dir();
        tokio::fs::create_dir_all(&dir).await.map_err(|e| {
            ServerError::Internal(anyhow::anyhow!("creating {}: {e}", dir.display()))
        })?;

        let id = FileId::new();
        let partial = dir.join(format!("{id}.part"));
        let (size, sha256) = match self.stream_to_disk(&partial, body).await {
            Ok(done) => done,
            Err(error) => {
                let _ = tokio::fs::remove_file(&partial).await;
                return Err(error);
            }
        };
        tokio::fs::rename(&partial, dir.join(id.to_string()))
            .await
            .map_err(|e| ServerError::Internal(anyhow::anyhow!("finalising upload: {e}")))?;

        let row = FileRow {
            id,
            room_id: room,
            user_id: actor.id,
            name,
            size,
            sha256,
            kind: meta.kind,
            entries: meta.entries,
            created_at: Timestamp::now(),
        };
        self.store().insert_file(&row).await?;
        tracing::info!(
            user = %actor.username,
            file = %row.name,
            bytes = row.size,
            "file shared"
        );

        self.post_message(actor, room, &row.to_card().to_body(), MessageKind::File)
            .await
    }

    /// Copies the body to `path`, returning its size and hex SHA-256.
    async fn stream_to_disk<St, B, E>(&self, path: &PathBuf, mut body: St) -> Result<(u64, String)>
    where
        St: Stream<Item = std::result::Result<B, E>> + Unpin,
        B: AsRef<[u8]>,
        E: std::fmt::Display,
    {
        let max = self.config().upload_max_bytes();
        let file = tokio::fs::File::create(path)
            .await
            .map_err(|e| ServerError::Internal(anyhow::anyhow!("creating upload: {e}")))?;
        let mut out = tokio::io::BufWriter::with_capacity(WRITE_BUFFER, file);
        let mut hasher = Sha256::new();
        let mut size: u64 = 0;

        while let Some(chunk) = body.next().await {
            let chunk =
                chunk.map_err(|e| ServerError::BadRequest(format!("upload interrupted: {e}")))?;
            let bytes = chunk.as_ref();
            size += bytes.len() as u64;
            if size > max {
                return Err(ServerError::BadRequest(format!(
                    "file exceeds the server's {} upload limit",
                    file::human_bytes(max)
                )));
            }
            hasher.update(bytes);
            out.write_all(bytes)
                .await
                .map_err(|e| ServerError::Internal(anyhow::anyhow!("writing upload: {e}")))?;
        }
        out.flush()
            .await
            .map_err(|e| ServerError::Internal(anyhow::anyhow!("flushing upload: {e}")))?;

        if size == 0 {
            return Err(ServerError::BadRequest("empty upload".to_owned()));
        }
        Ok((size, hex(&hasher.finalize())))
    }

    /// Authorises a download: the file must exist and the caller must be a
    /// member of the room it was shared into. Returns the row and its path.
    pub async fn open_file(&self, actor: &UserRow, id: FileId) -> Result<(FileRow, PathBuf)> {
        let row = self
            .store()
            .file_by_id(id)
            .await?
            .ok_or(ServerError::NotFound("file"))?;
        self.require_member(row.room_id, actor.id).await?;
        let path = self.files_dir().join(id.to_string());
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            Ok((row, path))
        } else {
            Err(ServerError::NotFound("file"))
        }
    }

    /// Removes bytes whose row is gone (deleted rooms) and abandoned partials.
    pub(crate) async fn sweep_orphaned_files(&self) {
        let Ok(mut entries) = tokio::fs::read_dir(self.files_dir()).await else {
            return; // nothing shared yet
        };
        let mut removed = 0u32;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let old_enough = entry
                .metadata()
                .await
                .and_then(|m| m.modified())
                .ok()
                .and_then(|at| SystemTime::now().duration_since(at).ok())
                .is_some_and(|age| age > ORPHAN_GRACE);
            if !old_enough {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let orphaned = match name.strip_suffix(".part") {
                Some(_) => true,
                None => match name.parse::<FileId>() {
                    Ok(id) => matches!(self.store().file_by_id(id).await, Ok(None)),
                    Err(_) => false, // not ours; leave it alone
                },
            };
            if orphaned && tokio::fs::remove_file(entry.path()).await.is_ok() {
                removed += 1;
            }
        }
        if removed > 0 {
            tracing::info!(files = removed, "swept orphaned file data");
        }
    }
}

/// Lowercase hex.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::super::testkit;
    use super::*;
    use crate::repo::sqlite::SqliteStore;
    use crate::repo::RoomRepo;
    use teresa_proto::{FileCard, ServerFrame, Visibility};

    async fn actor(core: &Core<SqliteStore>, name: &str) -> UserRow {
        let (token, _) = testkit::register(core, name).await;
        core.authenticate(&token).await.unwrap()
    }

    fn body(
        chunks: &[&'static [u8]],
    ) -> impl Stream<Item = std::result::Result<&'static [u8], String>> + Unpin {
        futures::stream::iter(chunks.iter().copied().map(Ok).collect::<Vec<_>>())
    }

    fn meta(name: &str) -> UploadMeta {
        UploadMeta {
            name: name.into(),
            kind: FileKind::File,
            entries: None,
        }
    }

    #[tokio::test]
    async fn an_upload_lands_on_disk_and_announces_itself() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let general = core.store().room_by_name("general").await.unwrap().unwrap();
        let mut rx = core.hub().subscribe_room(general.id);

        let message = core
            .receive_file(
                &alice,
                general.id,
                meta("notes.txt"),
                body(&[b"hello ", b"world"]),
            )
            .await
            .unwrap();
        assert_eq!(message.kind, MessageKind::File);

        let card = FileCard::from_body(&message.body).unwrap();
        assert_eq!(card.name, "notes.txt");
        assert_eq!(card.size, 11);
        // sha256("hello world")
        assert_eq!(
            card.sha256,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );

        let (row, path) = core.open_file(&alice, card.id).await.unwrap();
        assert_eq!(row.name, "notes.txt");
        assert_eq!(std::fs::read(path).unwrap(), b"hello world");

        match rx.recv().await.unwrap().as_ref() {
            ServerFrame::Message { message } => assert_eq!(message.kind, MessageKind::File),
            other => panic!("expected the card to fan out, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn outsiders_can_neither_upload_nor_download() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let mallory = actor(&core, "mallory").await;
        let (vault, _) = core
            .create_room(&alice, "vault", None, Visibility::InviteOnly)
            .await
            .unwrap();
        let vault = vault.room.id;

        let refused = core
            .receive_file(&mallory, vault, meta("x.bin"), body(&[b"x"]))
            .await
            .unwrap_err();
        assert!(matches!(refused, ServerError::Forbidden(_)));

        let shared = core
            .receive_file(&alice, vault, meta("secret.txt"), body(&[b"s"]))
            .await
            .unwrap();
        let card = FileCard::from_body(&shared.body).unwrap();
        let refused = core.open_file(&mallory, card.id).await.unwrap_err();
        assert!(matches!(refused, ServerError::Forbidden(_)));
    }

    #[tokio::test]
    async fn hostile_names_and_empty_bodies_are_refused_without_debris() {
        let core = testkit::core().await;
        let alice = actor(&core, "alice").await;
        let general = core.store().room_by_name("general").await.unwrap().unwrap();

        assert!(core
            .receive_file(&alice, general.id, meta("../../etc/passwd"), body(&[b"x"]))
            .await
            .is_err());
        assert!(core
            .receive_file(&alice, general.id, meta("empty.txt"), body(&[]))
            .await
            .is_err());

        let leftovers = std::fs::read_dir(core.files_dir()).map_or(0, Iterator::count);
        assert_eq!(leftovers, 0, "a refused upload must leave nothing behind");
    }

    #[tokio::test]
    async fn the_size_cap_is_enforced_while_streaming() {
        static MEG: [u8; 1024 * 1024] = [7; 1024 * 1024];
        let store = SqliteStore::memory().await.unwrap();
        let mut config = crate::ServerConfig {
            files_dir: Some(std::env::temp_dir().join(format!("teresa-cap-{}", FileId::new()))),
            ..crate::ServerConfig::default()
        };
        config.limits.upload_max_mb = 1;
        let core = Core::new(store, config);
        core.bootstrap().await.unwrap();
        let alice = actor(&core, "alice").await;
        let general = core.store().room_by_name("general").await.unwrap().unwrap();

        let refused = core
            .receive_file(&alice, general.id, meta("big.bin"), body(&[&MEG, b"!"]))
            .await
            .unwrap_err();
        assert!(matches!(refused, ServerError::BadRequest(_)));
    }

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex(&[0x00, 0x0f, 0xab]), "000fab");
    }
}
