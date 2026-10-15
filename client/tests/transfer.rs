//! File transfer against a real server: the client's own TransferClient
//! uploading files and folders over HTTP, another account pulling them back,
//! integrity verified end to end.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use teresa_client::config::ClientConfig;
use teresa_client::net::http::AuthClient;
use teresa_client::net::transfer::{Outcome, Stage, TransferClient, TransferUpdate};
use teresa_proto::{FileCard, FileKind, RoomId};
use teresa_server::repo::sqlite::SqliteStore;
use teresa_server::repo::RoomRepo;
use teresa_server::services::Core;
use teresa_server::state::AppState;
use teresa_server::{api, ServerConfig};
use tokio::sync::mpsc;

const STEP: Duration = Duration::from_secs(20);

struct Rig {
    config: ClientConfig,
    general: RoomId,
    _scratch: tempfile::TempDir,
}

async fn rig() -> Rig {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let scratch = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        database: scratch.path().join("teresa.db"),
        ..ServerConfig::default()
    };
    let store = SqliteStore::open(&config.database).await.unwrap();
    let core = Core::new(store, config);
    core.bootstrap().await.unwrap();
    let general = core
        .store()
        .room_by_name("general")
        .await
        .unwrap()
        .unwrap()
        .id;
    let state = AppState::new(core);

    let handle = axum_server::Handle::new();
    tokio::spawn(
        axum_server::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            .handle(handle.clone())
            .serve(api::router(state).into_make_service_with_connect_info::<SocketAddr>()),
    );
    let addr = handle.listening().await.unwrap();

    Rig {
        config: ClientConfig {
            server: addr.ip().to_string(),
            port: addr.port(),
            downloads: Some(scratch.path().join("downloads")),
            ..ClientConfig::default()
        },
        general,
        _scratch: scratch,
    }
}

async fn client(config: &ClientConfig, name: &str) -> TransferClient {
    let ok = AuthClient::new(config)
        .register(name, "a sufficiently long password", name)
        .await
        .unwrap();
    TransferClient::new(config, &ok.token)
}

/// Waits for the transfer's final word, checking progress was reported.
async fn finish(rx: &mut mpsc::Receiver<TransferUpdate>) -> (Outcome, usize) {
    let mut progress = 0;
    loop {
        let update = tokio::time::timeout(STEP, rx.recv())
            .await
            .expect("transfer finishes in time")
            .expect("task alive");
        match update.stage {
            Stage::Done(outcome) => return (outcome, progress),
            Stage::Failed(reason) => panic!("transfer failed: {reason}"),
            _ => progress += 1,
        }
    }
}

fn shared(outcome: Outcome) -> FileCard {
    match outcome {
        Outcome::Shared(card) => card,
        Outcome::Saved(path) => panic!("expected an upload, got a download to {path:?}"),
    }
}

fn saved(outcome: Outcome) -> std::path::PathBuf {
    match outcome {
        Outcome::Saved(path) => path,
        Outcome::Shared(card) => panic!("expected a download, got {card:?}"),
    }
}

/// Pseudo-random, incompressible-ish bytes.
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state & 0xff) as u8
        })
        .collect()
}

#[tokio::test]
async fn a_file_crosses_from_one_account_to_another_intact() {
    let rig = rig().await;
    let alice = client(&rig.config, "alice").await;
    let bob = client(&rig.config, "bob").await;

    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("dump.sql");
    let bytes = noise(5 * 1024 * 1024 + 123);
    std::fs::write(&source, &bytes).unwrap();

    let (tx, mut rx) = mpsc::channel(256);
    alice.spawn_upload(1, rig.general, source, false, tx);
    let (outcome, progress) = finish(&mut rx).await;
    let card = shared(outcome);
    assert!(progress > 0, "the dock had something to show");
    assert_eq!(card.name, "dump.sql");
    assert_eq!(card.kind, FileKind::File);
    assert_eq!(card.size, bytes.len() as u64);

    let (tx, mut rx) = mpsc::channel(256);
    bob.spawn_download(2, card.clone(), rig.config.downloads_dir(), tx);
    let path = saved(finish(&mut rx).await.0);
    assert_eq!(path.file_name().unwrap(), "dump.sql");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    // A second pull never clobbers the first.
    let (tx, mut rx) = mpsc::channel(256);
    bob.spawn_download(3, card, rig.config.downloads_dir(), tx);
    let again = saved(finish(&mut rx).await.0);
    assert_eq!(again.file_name().unwrap(), "dump (1).sql");
}

#[tokio::test]
async fn a_folder_travels_zipped_and_lands_unpacked() {
    let rig = rig().await;
    let alice = client(&rig.config, "alice").await;
    let bob = client(&rig.config, "bob").await;

    let source_dir = tempfile::tempdir().unwrap();
    let project = source_dir.path().join("st's project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(project.join("src/main.rs"), "fn main() {}\n".repeat(5000)).unwrap();

    let (tx, mut rx) = mpsc::channel(256);
    alice.spawn_upload(1, rig.general, project.clone(), false, tx);
    let card = shared(finish(&mut rx).await.0);
    assert_eq!(card.kind, FileKind::Folder);
    assert_eq!(card.name, "st's project.zip");
    assert_eq!(card.entries, Some(2));
    assert!(
        card.size < 65_000,
        "deflate earned its keep: {} bytes",
        card.size
    );

    let (tx, mut rx) = mpsc::channel(256);
    bob.spawn_download(2, card, rig.config.downloads_dir(), tx);
    let landed = saved(finish(&mut rx).await.0);
    assert_eq!(landed.file_name().unwrap(), "st's project");
    assert_same_file(&landed.join("src/main.rs"), &project.join("src/main.rs"));
    assert_same_file(&landed.join("Cargo.toml"), &project.join("Cargo.toml"));
}

#[tokio::test]
async fn zip_forces_compression_of_a_single_file() {
    let rig = rig().await;
    let alice = client(&rig.config, "alice").await;

    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("server.log");
    std::fs::write(&source, "GET /healthz 200\n".repeat(20_000)).unwrap();

    let (tx, mut rx) = mpsc::channel(256);
    alice.spawn_upload(1, rig.general, source, true, tx);
    let card = shared(finish(&mut rx).await.0);
    assert_eq!(card.kind, FileKind::Zip);
    assert_eq!(card.name, "server.log.zip");
    assert!(card.size < 20_000);
}

fn assert_same_file(a: &Path, b: &Path) {
    assert_eq!(
        std::fs::read(a).unwrap(),
        std::fs::read(b).unwrap(),
        "{} differs from {}",
        a.display(),
        b.display()
    );
}

/// Waits for the transfer's final stage, cancelling it as soon as it is
/// moving bytes.
async fn cancel_midway(
    client: &TransferClient,
    id: u64,
    rx: &mut mpsc::Receiver<TransferUpdate>,
) -> Stage {
    let mut asked = false;
    loop {
        let update = tokio::time::timeout(STEP, rx.recv())
            .await
            .expect("transfer ends in time")
            .expect("task alive");
        match update.stage {
            Stage::Sending { .. } | Stage::Receiving { .. } | Stage::Packing { .. } if !asked => {
                client.cancel(id);
                asked = true;
            }
            stage @ (Stage::Done(_) | Stage::Failed(_) | Stage::Cancelled) => return stage,
            _ => {}
        }
    }
}

#[tokio::test]
async fn a_cancelled_upload_publishes_nothing_and_leaves_no_bytes() {
    let rig = rig().await;
    let alice = client(&rig.config, "alice").await;
    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("huge.bin");
    std::fs::write(&source, noise(64 * 1024 * 1024)).unwrap();

    let (tx, mut rx) = mpsc::channel(256);
    alice.spawn_upload(1, rig.general, source, false, tx);
    assert_eq!(cancel_midway(&alice, 1, &mut rx).await, Stage::Cancelled);

    // The server notices the dropped connection and discards the partial.
    let files = rig._scratch.path().join("files");
    let mut left = usize::MAX;
    for _ in 0..50 {
        left = std::fs::read_dir(&files).map_or(0, Iterator::count);
        if left == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(left, 0, "no stored or partial bytes on the server");
}

#[tokio::test]
async fn a_cancelled_download_leaves_nothing_behind() {
    let rig = rig().await;
    let alice = client(&rig.config, "alice").await;
    let bob = client(&rig.config, "bob").await;
    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("huge.bin");
    std::fs::write(&source, noise(64 * 1024 * 1024)).unwrap();

    let (tx, mut rx) = mpsc::channel(256);
    alice.spawn_upload(1, rig.general, source, false, tx);
    let card = shared(finish(&mut rx).await.0);

    let inbox = rig._scratch.path().join("inbox");
    let (tx, mut rx) = mpsc::channel(256);
    bob.spawn_download(2, card, inbox.clone(), tx);
    assert_eq!(cancel_midway(&bob, 2, &mut rx).await, Stage::Cancelled);
    assert_eq!(
        std::fs::read_dir(&inbox).map_or(0, Iterator::count),
        0,
        "the partial download is removed"
    );
}

#[tokio::test]
async fn a_cancelled_folder_send_removes_its_zip() {
    let rig = rig().await;
    let alice = client(&rig.config, "alice").await;
    let source_dir = tempfile::tempdir().unwrap();
    let folder = source_dir.path().join("dataset");
    std::fs::create_dir(&folder).unwrap();
    for i in 0..4 {
        std::fs::write(folder.join(format!("part{i}.bin")), noise(16 * 1024 * 1024)).unwrap();
    }

    let (tx, mut rx) = mpsc::channel(256);
    alice.spawn_upload(77, rig.general, folder, false, tx);
    assert_eq!(cancel_midway(&alice, 77, &mut rx).await, Stage::Cancelled);

    let zip = std::env::temp_dir().join(format!("teresa-{}-77.zip", std::process::id()));
    for _ in 0..50 {
        if !zip.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!zip.exists(), "the packed archive is cleaned up");
}
