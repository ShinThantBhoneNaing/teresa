//! File transfer: packing, streaming, verifying, unpacking.
//!
//! Each transfer is one tokio task that reports [`TransferUpdate`]s to the UI
//! and owns nothing the UI holds. Bytes go over plain HTTP — raw bodies in
//! 1 MiB chunks, no base64 — so the link and the disk set the speed. Folders
//! become a zip on the way out (level-1 deflate; already-compressed formats
//! are stored, not recompressed) and a folder again on the way in. Every
//! download is hashed as it lands and checked against the SHA-256 the server
//! computed on upload before it is given its final name.
//!
//! Any transfer can be cancelled by id: the task stops where it is, the
//! connection drops (so the server discards a half upload and posts no card),
//! and temporary files on this side are removed.

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use sha2::{Digest, Sha256};
use teresa_proto::{FileCard, FileKind, Message, MessageKind, RoomId};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio_util::io::ReaderStream;
use tokio_util::sync::CancellationToken;

use crate::config::ClientConfig;

/// Chunk size on the wire and on disk.
const CHUNK: usize = 1024 * 1024;
/// Progress reports per second, at most. The UI redraws at ~30 Hz anyway.
const REPORT_EVERY: Duration = Duration::from_millis(50);

/// Extensions whose contents are already compressed; deflating them again
/// burns CPU for nothing, so they are stored as-is inside a zip.
const PRECOMPRESSED: &[&str] = &[
    "zip", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar", "jar", "apk", "whl", "png", "jpg", "jpeg",
    "gif", "webp", "heic", "avif", "mp3", "aac", "ogg", "opus", "flac", "mp4", "mkv", "mov",
    "webm", "avi", "pdf", "docx", "xlsx", "pptx", "woff2", "dmg", "iso",
];

/// What a transfer task is doing right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Zipping the source: `done` of `total` source bytes read.
    Packing {
        /// Source bytes consumed so far.
        done: u64,
        /// Source bytes in total.
        total: u64,
    },
    /// Uploading.
    Sending {
        /// Bytes on the wire so far.
        done: u64,
        /// Bytes to send.
        total: u64,
    },
    /// Downloading.
    Receiving {
        /// Bytes landed so far.
        done: u64,
        /// Bytes expected.
        total: u64,
    },
    /// Comparing the received bytes' SHA-256 with the server's.
    Verifying,
    /// Unzipping a received folder.
    Unpacking,
    /// Finished.
    Done(Outcome),
    /// Gave up; the reason is human text.
    Failed(String),
    /// Stopped on request; nothing was published or saved.
    Cancelled,
}

/// How a transfer ended well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// An upload: the card the server published.
    Shared(FileCard),
    /// A download: where it landed. The SHA-256 has been verified.
    Saved(PathBuf),
}

/// One report from one transfer task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferUpdate {
    /// The id the UI assigned when it asked for the transfer.
    pub id: u64,
    /// Where it stands.
    pub stage: Stage,
}

/// Message used internally when a task notices it was cancelled.
const CANCELLED: &str = "cancelled";

/// The transfer client: base URL, bearer token, and an HTTP client with no
/// overall deadline — a large file is allowed to take as long as it takes.
#[derive(Debug, Clone)]
pub struct TransferClient {
    base: String,
    token: String,
    http: reqwest::Client,
    /// Cancel handles of the running transfers, by id.
    running: Arc<Mutex<HashMap<u64, CancellationToken>>>,
}

impl TransferClient {
    /// Builds a client for the configured server.
    ///
    /// # Panics
    /// Never in practice: the reqwest builder only fails over TLS backend
    /// misconfiguration, and the backend is compiled in.
    #[must_use]
    pub fn new(config: &ClientConfig, token: &str) -> Self {
        Self {
            base: config.http_base(),
            token: token.to_owned(),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .tcp_nodelay(true)
                .build()
                .expect("reqwest client builds"),
            running: Arc::default(),
        }
    }

    /// Stops a running transfer. Unknown or finished ids are ignored.
    ///
    /// # Panics
    /// Never in practice: the lock is held only for map inserts and removes,
    /// which cannot panic.
    pub fn cancel(&self, id: u64) {
        if let Some(token) = self.running.lock().expect("lock").get(&id) {
            token.cancel();
        }
    }

    /// Runs one transfer task: registers its cancel handle, races the work
    /// against it, and reports the ending. Cancelling drops the work future,
    /// which closes the connection; [`TempFile`] guards clean up the disk.
    fn spawn_task<F>(
        &self,
        id: u64,
        updates: mpsc::Sender<TransferUpdate>,
        work: impl FnOnce(Self, Reporter) -> F,
    ) where
        F: std::future::Future<Output = Result<Outcome, String>> + Send + 'static,
    {
        let cancel = CancellationToken::new();
        self.running
            .lock()
            .expect("lock")
            .insert(id, cancel.clone());
        let reporter = Reporter::new(id, updates, cancel.clone());
        let task = work(self.clone(), reporter.clone());
        let running = self.running.clone();
        tokio::spawn(async move {
            let stage = tokio::select! {
                result = task => match result {
                    Ok(outcome) => Stage::Done(outcome),
                    Err(_) if cancel.is_cancelled() => Stage::Cancelled,
                    Err(error) => Stage::Failed(error),
                },
                () = cancel.cancelled() => Stage::Cancelled,
            };
            running.lock().expect("lock").remove(&id);
            reporter.finish(stage).await;
        });
    }

    /// Spawns an upload of `path` into `room`. Folders are always zipped;
    /// `zip` forces it for a single file too.
    pub fn spawn_upload(
        &self,
        id: u64,
        room: RoomId,
        path: PathBuf,
        zip: bool,
        updates: mpsc::Sender<TransferUpdate>,
    ) {
        self.spawn_task(id, updates, move |client, reporter| async move {
            client
                .upload(room, &path, zip, &reporter)
                .await
                .map(Outcome::Shared)
        });
    }

    /// Spawns a download of `card` into the folder `dest` (created if needed).
    pub fn spawn_download(
        &self,
        id: u64,
        card: FileCard,
        dest: PathBuf,
        updates: mpsc::Sender<TransferUpdate>,
    ) {
        self.spawn_task(id, updates, move |client, reporter| async move {
            client
                .download(&card, &dest, &reporter)
                .await
                .map(Outcome::Saved)
        });
    }

    async fn upload(
        &self,
        room: RoomId,
        path: &Path,
        force_zip: bool,
        reporter: &Reporter,
    ) -> Result<FileCard, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let source_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} has no file name", path.display()))?;

        // Decide what goes on the wire, packing first if needed.
        let (wire_path, name, kind, entries, temp) = if meta.is_dir() || force_zip {
            let kind = if meta.is_dir() {
                FileKind::Folder
            } else {
                FileKind::Zip
            };
            let temp = TempFile(std::env::temp_dir().join(format!(
                "teresa-{}-{}.zip",
                std::process::id(),
                reporter.id
            )));
            let source = path.to_path_buf();
            let target = temp.0.clone();
            let progress = reporter.clone();
            let entries = tokio::task::spawn_blocking(move || pack(&source, &target, &progress))
                .await
                .map_err(|e| format!("packing crashed: {e}"))?
                .map_err(|e| {
                    if reporter.cancelled() {
                        CANCELLED.to_owned()
                    } else {
                        format!("packing failed: {e}")
                    }
                })?;
            let entries = (kind == FileKind::Folder).then_some(entries);
            (
                temp.0.clone(),
                format!("{source_name}.zip"),
                kind,
                entries,
                Some(temp),
            )
        } else if meta.is_file() {
            (path.to_path_buf(), source_name, FileKind::File, None, None)
        } else {
            return Err(format!("{} is not a file or folder", path.display()));
        };

        let result = self
            .send(room, &wire_path, &name, kind, entries, reporter)
            .await;
        drop(temp); // removes the zip, also when this future is dropped mid-send
        result
    }

    async fn send(
        &self,
        room: RoomId,
        wire_path: &Path,
        name: &str,
        kind: FileKind,
        entries: Option<u32>,
        reporter: &Reporter,
    ) -> Result<FileCard, String> {
        let file = tokio::fs::File::open(wire_path)
            .await
            .map_err(|e| format!("{}: {e}", wire_path.display()))?;
        let total = file.metadata().await.map_err(|e| e.to_string())?.len();
        if total == 0 {
            return Err("refusing to send an empty file".to_owned());
        }

        let sent = Arc::new(AtomicU64::new(0));
        let progress = reporter.clone();
        let counter = sent.clone();
        let stream = ReaderStream::with_capacity(file, CHUNK).inspect(move |chunk| {
            if let Ok(bytes) = chunk {
                let done =
                    counter.fetch_add(bytes.len() as u64, Ordering::Relaxed) + bytes.len() as u64;
                progress.report(Stage::Sending { done, total });
            }
        });
        reporter.force(Stage::Sending { done: 0, total });

        let mut query: Vec<(&str, String)> = vec![
            ("name", name.to_owned()),
            ("kind", kind.as_str().to_owned()),
        ];
        if let Some(entries) = entries {
            query.push(("entries", entries.to_string()));
        }

        let url =
            reqwest::Url::parse_with_params(&format!("{}/rooms/{room}/files", self.base), &query)
                .map_err(|e| format!("bad server address: {e}"))?;
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_LENGTH, total)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await
            .map_err(|e| format!("upload failed: {e}"))?;

        if !response.status().is_success() {
            return Err(refusal(response).await);
        }
        let message: Message = response
            .json()
            .await
            .map_err(|e| format!("unexpected reply: {e}"))?;
        if message.kind != MessageKind::File {
            return Err("server did not publish a file card".to_owned());
        }
        FileCard::from_body(&message.body).map_err(|e| e.to_string())
    }

    async fn download(
        &self,
        card: &FileCard,
        dest: &Path,
        reporter: &Reporter,
    ) -> Result<PathBuf, String> {
        tokio::fs::create_dir_all(dest)
            .await
            .map_err(|e| format!("{}: {e}", dest.display()))?;

        let response = self
            .http
            .get(format!("{}/files/{}", self.base, card.id))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("download failed: {e}"))?;
        if !response.status().is_success() {
            return Err(refusal(response).await);
        }
        let total = response.content_length().unwrap_or(card.size);

        // Land beside the destination under a temporary name, so a half
        // transfer never masquerades as a finished one.
        // The guard deletes it on every way out but success, including a
        // cancel that drops this future mid-download.
        let partial = TempFile(dest.join(format!(".{}.{}.part", card.landing_name(), card.id)));
        let digest = receive(response, &partial.0, total, reporter).await?;

        reporter.force(Stage::Verifying);
        if !digest.eq_ignore_ascii_case(&card.sha256) {
            return Err("integrity check failed — sha256 mismatch, file discarded".to_owned());
        }

        let destination = unique_path(dest, card.landing_name());
        if card.kind == FileKind::Folder {
            reporter.force(Stage::Unpacking);
            let archive = partial.0.clone();
            let target = destination.clone();
            let unpacked = tokio::task::spawn_blocking(move || unpack(&archive, &target))
                .await
                .map_err(|e| format!("unpacking crashed: {e}"))?;
            drop(partial);
            unpacked.map_err(|e| format!("unpacking failed: {e}"))?;
        } else {
            tokio::fs::rename(&partial.0, &destination)
                .await
                .map_err(|e| format!("{}: {e}", destination.display()))?;
            std::mem::forget(partial); // landed under its real name
        }
        Ok(destination)
    }
}

/// Streams a response body to `path`, returning its hex SHA-256.
async fn receive(
    mut response: reqwest::Response,
    path: &Path,
    total: u64,
    reporter: &Reporter,
) -> Result<String, String> {
    let file = tokio::fs::File::create(path)
        .await
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = tokio::io::BufWriter::with_capacity(CHUNK, file);
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    reporter.force(Stage::Receiving { done, total });

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("download interrupted: {e}"))?
    {
        hasher.update(&chunk);
        out.write_all(&chunk)
            .await
            .map_err(|e| format!("writing {}: {e}", path.display()))?;
        done += chunk.len() as u64;
        reporter.report(Stage::Receiving { done, total });
    }
    out.flush().await.map_err(|e| e.to_string())?;
    if done != total {
        return Err(format!("connection dropped at {done} of {total} bytes"));
    }
    Ok(hex(&hasher.finalize()))
}

/// The server's error text, or the status line if it sent none.
async fn refusal(response: reqwest::Response) -> String {
    let status = response.status();
    response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|body| body["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("server answered {status}"))
}

// --- progress ----------------------------------------------------------------

/// Throttled progress reporting for one transfer. Cheap to clone into the
/// blocking packer and the upload stream.
#[derive(Debug, Clone)]
struct Reporter {
    id: u64,
    updates: mpsc::Sender<TransferUpdate>,
    last: Arc<std::sync::Mutex<Instant>>,
    cancel: CancellationToken,
}

impl Reporter {
    fn new(id: u64, updates: mpsc::Sender<TransferUpdate>, cancel: CancellationToken) -> Self {
        Self {
            id,
            updates,
            cancel,
            last: Arc::new(std::sync::Mutex::new(
                Instant::now()
                    .checked_sub(REPORT_EVERY)
                    .unwrap_or_else(Instant::now),
            )),
        }
    }

    /// Reports if enough time has passed since the last report. Never blocks:
    /// a full inbox means the UI is busy, and a skipped frame of progress is
    /// invisible.
    fn report(&self, stage: Stage) {
        let mut last = self.last.lock().expect("reporter lock is never poisoned");
        if last.elapsed() >= REPORT_EVERY {
            *last = Instant::now();
            let _ = self.updates.try_send(TransferUpdate { id: self.id, stage });
        }
    }

    /// Whether the user asked this transfer to stop. Checked by the blocking
    /// packer, which a dropped future cannot interrupt.
    fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Reports a stage change unconditionally (still non-blocking).
    fn force(&self, stage: Stage) {
        *self.last.lock().expect("reporter lock is never poisoned") = Instant::now();
        let _ = self.updates.try_send(TransferUpdate { id: self.id, stage });
    }

    /// The terminal report — this one waits for room, it must not be lost.
    async fn finish(&self, stage: Stage) {
        let _ = self
            .updates
            .send(TransferUpdate { id: self.id, stage })
            .await;
    }
}

/// A temporary file deleted when dropped — on error, on cancel, or when the
/// owning future is dropped. `mem::forget` keeps one that became the result.
#[derive(Debug)]
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

// --- packing ----------------------------------------------------------------

/// Zips `source` (a folder's contents, or one file) into `target`.
/// Returns how many files went in.
fn pack(source: &Path, target: &Path, reporter: &Reporter) -> std::io::Result<u32> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    let mut files = Vec::new();
    let mut dirs = Vec::new();
    if source.is_dir() {
        walk(source, Path::new(""), &mut files, &mut dirs)?;
    } else {
        let name = source
            .file_name()
            .map_or_else(|| PathBuf::from("file"), PathBuf::from);
        files.push((source.to_path_buf(), name));
    }
    let total: u64 = files
        .iter()
        .filter_map(|(path, _)| std::fs::metadata(path).ok())
        .map(|m| m.len())
        .sum();

    let out = std::io::BufWriter::with_capacity(CHUNK, std::fs::File::create(target)?);
    let mut zip = zip::ZipWriter::new(out);
    for dir in &dirs {
        zip.add_directory(zip_name(dir), SimpleFileOptions::default())
            .map_err(std::io::Error::other)?;
    }

    let mut done = 0u64;
    let mut buffer = vec![0u8; CHUNK];
    reporter.force(Stage::Packing { done, total });
    for (path, relative) in &files {
        let meta = std::fs::metadata(path)?;
        let stored = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| PRECOMPRESSED.contains(&e.to_ascii_lowercase().as_str()));
        let mut options = SimpleFileOptions::default()
            .large_file(meta.len() >= u64::from(u32::MAX))
            .compression_method(if stored {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            });
        if !stored {
            options = options.compression_level(Some(1));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            options = options.unix_permissions(meta.permissions().mode());
        }
        zip.start_file(zip_name(relative), options)
            .map_err(std::io::Error::other)?;

        let mut input = std::fs::File::open(path)?;
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            if reporter.cancelled() {
                // The awaiting future is gone, so its guard may already have
                // run; remove the archive here too (Windows cannot delete a
                // file that is still open).
                drop(zip);
                let _ = std::fs::remove_file(target);
                return Err(std::io::Error::other(CANCELLED));
            }
            zip.write_all(&buffer[..read])?;
            done += read as u64;
            reporter.report(Stage::Packing { done, total });
        }
    }
    zip.finish().map_err(std::io::Error::other)?.flush()?;
    Ok(u32::try_from(files.len()).unwrap_or(u32::MAX))
}

/// Collects regular files and directories under `root`, relative to it.
/// Symlinks are skipped: following them could pack half the disk, and
/// recreating them on someone else's machine is a trap.
fn walk(
    root: &Path,
    relative: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
    dirs: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(root.join(relative))?.collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let kind = entry.file_type()?;
        let path = relative.join(entry.file_name());
        if kind.is_dir() {
            dirs.push(path.clone());
            walk(root, &path, files, dirs)?;
        } else if kind.is_file() {
            files.push((root.join(&path), path));
        }
    }
    Ok(())
}

/// A relative path as a zip entry name: forward slashes on every OS.
fn zip_name(relative: &Path) -> String {
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Unzips `archive` into a fresh directory `target`. Entry names are
/// sanitised by the zip crate; nothing can land outside `target`.
fn unpack(archive: &Path, target: &Path) -> std::io::Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::with_capacity(CHUNK, file))
        .map_err(std::io::Error::other)?;
    std::fs::create_dir_all(target)?;
    zip.extract(target).map_err(std::io::Error::other)
}

// --- paths ------------------------------------------------------------------

/// `dir/name`, or `dir/name (1).ext`, `dir/name (2).ext`… — never an existing path.
///
/// # Panics
/// Never: the counter is unbounded and a directory is not.
#[must_use]
#[allow(clippy::maybe_infinite_iter)] // finite in practice: a directory holds finitely many names
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|candidate| !candidate.exists())
        .expect("an unbounded counter finds a free name")
}

/// Turns what a user typed (or a terminal pasted on drag-and-drop) into a
/// path that exists: quotes stripped, `~` expanded, and backslash escapes
/// (`st\'s\ project`) undone when the literal form does not exist.
///
/// # Errors
/// A message naming the path when nothing exists there.
pub fn resolve_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    let unquoted = ['"', '\'']
        .iter()
        .find_map(|q| {
            trimmed
                .strip_prefix(*q)
                .and_then(|rest| rest.strip_suffix(*q))
        })
        .unwrap_or(trimmed);
    if unquoted.is_empty() {
        return Err("which file? /send <path>".to_owned());
    }

    let mut candidates = vec![expand_home(unquoted)];
    if unquoted.contains('\\') && !cfg!(windows) {
        candidates.push(expand_home(&unescape(unquoted)));
    }
    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| format!("nothing at {unquoted}"))
}

/// Where a typed download folder points: quotes stripped, `~` expanded.
/// It need not exist yet — the download creates it — but it must not be a
/// file.
///
/// # Errors
/// A message for a blank entry or a path that names a file.
pub fn destination_dir(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    let unquoted = ['"', '\'']
        .iter()
        .find_map(|q| trimmed.strip_prefix(*q).and_then(|r| r.strip_suffix(*q)))
        .unwrap_or(trimmed);
    if unquoted.is_empty() {
        return Err("type a folder to save into".to_owned());
    }
    let mut path = expand_home(unquoted);
    if !path.exists() && unquoted.contains('\\') && !cfg!(windows) {
        let unescaped = expand_home(&unescape(unquoted));
        if unescaped.exists() {
            path = unescaped;
        }
    }
    if path.is_file() {
        return Err(format!("{unquoted} is a file — pick a folder"));
    }
    Ok(path)
}

/// A path for display, with the home directory folded back into `~`.
#[must_use]
pub fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

/// Tab completion for a folder being typed: extends the last component to
/// the longest prefix shared by the folders that match it, adding a
/// separator once the match is unique. Returns the input unchanged if
/// nothing matches.
#[must_use]
pub fn complete_dir(typed: &str) -> String {
    let sep = std::path::MAIN_SEPARATOR;
    let (parent, stem) = match typed.rfind(['/', sep]) {
        Some(i) => (&typed[..=i], &typed[i + 1..]),
        None => ("", typed),
    };
    let lookup = if parent.is_empty() {
        PathBuf::from(".")
    } else {
        expand_home(parent)
    };
    let Ok(entries) = std::fs::read_dir(lookup) else {
        return typed.to_owned();
    };
    let mut matches: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(stem) && (stem.starts_with('.') || !name.starts_with('.')))
        .collect();
    matches.sort();
    match matches.as_slice() {
        [] => typed.to_owned(),
        [only] => format!("{parent}{only}{sep}"),
        [first, rest @ ..] => {
            let common = rest.iter().fold(first.clone(), |acc, name| {
                acc.chars()
                    .zip(name.chars())
                    .take_while(|(a, b)| a == b)
                    .map(|(a, _)| a)
                    .collect()
            });
            format!("{parent}{common}")
        }
    }
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') => {
            dirs::home_dir().map_or_else(
                || PathBuf::from(path),
                |home| home.join(rest.trim_start_matches(['/', '\\'])),
            )
        }
        _ => PathBuf::from(path),
    }
}

fn unescape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
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
    use super::*;

    fn reporter() -> (Reporter, mpsc::Receiver<TransferUpdate>) {
        let (tx, rx) = mpsc::channel(1024);
        (Reporter::new(7, tx, CancellationToken::new()), rx)
    }

    #[test]
    fn a_folder_survives_a_pack_unpack_round_trip() {
        let scratch = tempfile::tempdir().unwrap();
        let source = scratch.path().join("project");
        std::fs::create_dir_all(source.join("src/deep")).unwrap();
        std::fs::create_dir_all(source.join("empty")).unwrap();
        std::fs::write(source.join("README.md"), "# hi\n").unwrap();
        std::fs::write(source.join("src/main.rs"), "fn main() {}\n".repeat(1000)).unwrap();
        std::fs::write(source.join("src/deep/logo.png"), [0u8, 1, 2, 3]).unwrap();
        std::fs::write(source.join("မြန်မာ.txt"), "မင်္ဂလာပါ").unwrap();

        let archive = scratch.path().join("project.zip");
        let (progress, _rx) = reporter();
        let count = pack(&source, &archive, &progress).unwrap();
        assert_eq!(count, 4);

        let landed = scratch.path().join("landed");
        unpack(&archive, &landed).unwrap();
        assert_eq!(
            std::fs::read_to_string(landed.join("src/main.rs")).unwrap(),
            "fn main() {}\n".repeat(1000)
        );
        assert_eq!(
            std::fs::read(landed.join("src/deep/logo.png")).unwrap(),
            [0u8, 1, 2, 3]
        );
        assert_eq!(
            std::fs::read_to_string(landed.join("မြန်မာ.txt")).unwrap(),
            "မင်္ဂလာပါ"
        );
        assert!(landed.join("empty").is_dir(), "empty folders survive");
    }

    #[test]
    fn packing_reports_progress_up_to_the_total() {
        let scratch = tempfile::tempdir().unwrap();
        let source = scratch.path().join("one.log");
        std::fs::write(&source, vec![b'x'; 3 * CHUNK + 17]).unwrap();
        let (progress, mut rx) = reporter();
        pack(&source, &scratch.path().join("one.zip"), &progress).unwrap();

        let first = rx.try_recv().unwrap();
        assert_eq!(
            first.stage,
            Stage::Packing {
                done: 0,
                total: 3 * CHUNK as u64 + 17
            }
        );
    }

    #[test]
    fn unique_paths_never_clobber() {
        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path();
        assert_eq!(unique_path(dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), "").unwrap();
        assert_eq!(unique_path(dir, "a.txt"), dir.join("a (1).txt"));
        std::fs::write(dir.join("a (1).txt"), "").unwrap();
        assert_eq!(unique_path(dir, "a.txt"), dir.join("a (2).txt"));
        std::fs::create_dir(dir.join("src")).unwrap();
        assert_eq!(unique_path(dir, "src"), dir.join("src (1)"));
        std::fs::write(dir.join(".env"), "").unwrap();
        assert_eq!(unique_path(dir, ".env"), dir.join(".env (1)"));
    }

    #[test]
    fn typed_and_dragged_paths_resolve() {
        let scratch = tempfile::tempdir().unwrap();
        let tricky = scratch.path().join("st's project");
        std::fs::create_dir(&tricky).unwrap();
        let plain = tricky.display().to_string();

        assert_eq!(
            resolve_path(&plain).unwrap(),
            tricky,
            "spaces need no quotes"
        );
        assert_eq!(resolve_path(&format!("\"{plain}\"")).unwrap(), tricky);
        if !cfg!(windows) {
            // What macOS Terminal pastes when a folder is dragged in.
            let dragged = plain.replace('\'', "\\'").replace(' ', "\\ ");
            assert_eq!(resolve_path(&format!("{dragged} ")).unwrap(), tricky);
        }
        assert!(resolve_path("/definitely/not/here").is_err());
        assert!(resolve_path("  ").is_err());
    }

    #[test]
    fn download_folders_expand_complete_and_refuse_files() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        std::fs::create_dir_all(root.join("projects/teresa")).unwrap();
        std::fs::create_dir_all(root.join("photos")).unwrap();
        std::fs::create_dir_all(root.join("photos-2026")).unwrap();
        std::fs::write(root.join("notes.txt"), "x").unwrap();
        let base = format!("{}{}", root.display(), std::path::MAIN_SEPARATOR);
        let sep = std::path::MAIN_SEPARATOR;

        assert_eq!(
            complete_dir(&format!("{base}pro")),
            format!("{base}projects{sep}")
        );
        assert_eq!(complete_dir(&format!("{base}pho")), format!("{base}photos"));
        assert_eq!(
            complete_dir(&format!("{base}not")),
            format!("{base}not"),
            "files never complete"
        );
        assert_eq!(complete_dir(&format!("{base}zzz")), format!("{base}zzz"));

        assert_eq!(
            destination_dir(&format!("{base}new/inbox")).unwrap(),
            root.join("new/inbox")
        );
        assert!(destination_dir(&format!("{base}notes.txt")).is_err());
        assert!(destination_dir("   ").is_err());
        if let Some(home) = dirs::home_dir() {
            assert_eq!(
                destination_dir("~/Downloads").unwrap(),
                home.join("Downloads")
            );
            assert_eq!(tilde(&home.join("Downloads")), format!("~{sep}Downloads"));
        }
    }

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex(&[0x00, 0x0f, 0xab]), "000fab");
    }
}
