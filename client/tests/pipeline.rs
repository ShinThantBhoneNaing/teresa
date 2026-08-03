//! The whole client pipeline against a real server: REST auth through the
//! client's own AuthClient, then the WebSocket supervisor, then the reducer —
//! everything the TUI runs on, minus the terminal itself.

use std::net::SocketAddr;
use std::time::Duration;

use teresa_client::app::event::Event;
use teresa_client::app::update::update;
use teresa_client::app::{App, ConnState, Effect};
use teresa_client::config::ClientConfig;
use teresa_client::net::http::AuthClient;
use teresa_client::net::ws::{self, NetEvent};
use teresa_server::repo::sqlite::SqliteStore;
use teresa_server::services::Core;
use teresa_server::state::AppState;
use teresa_server::{api, ServerConfig};
use tokio::sync::mpsc;

const STEP: Duration = Duration::from_secs(5);

/// Boots a real server and returns a client config pointed at it.
async fn spawn_server() -> (ClientConfig, tempfile::NamedTempFile) {
    // The binary does this in main(); the test stands in for main here.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let db = tempfile::NamedTempFile::new().expect("temp db");
    let config = ServerConfig {
        database: db.path().to_path_buf(),
        ..ServerConfig::default()
    };
    let store = SqliteStore::open(db.path()).await.expect("store");
    let core = Core::new(store, config);
    core.bootstrap().await.expect("bootstrap");
    let state = AppState::new(core);

    let handle = axum_server::Handle::new();
    tokio::spawn(
        axum_server::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            .handle(handle.clone())
            .serve(api::router(state).into_make_service_with_connect_info::<SocketAddr>()),
    );
    let addr = tokio::time::timeout(STEP, handle.listening())
        .await
        .expect("binds in time")
        .expect("binds");

    let client_config = ClientConfig {
        server: addr.ip().to_string(),
        port: addr.port(),
        tls: false,
        ..ClientConfig::default()
    };
    (client_config, db)
}

#[tokio::test]
async fn register_connect_chat_via_the_real_stack() {
    let (config, _db) = spawn_server().await;

    // Register through the client's own HTTP path — and prove the healthz
    // probe the animated login leans on answers.
    let auth = AuthClient::new(&config);
    let version = auth.healthz().await.expect("healthz answers");
    assert!(!version.is_empty());

    let ok = auth
        .register("neo", "follow the white rabbit", "Neo")
        .await
        .expect("registration succeeds");
    assert_eq!(ok.user.username, "neo");

    // Drive the app exactly as main() does: supervisor → events → reducer.
    let mut app = App::new(config);
    let (net_tx, mut net_rx) = mpsc::channel::<NetEvent>(64);
    let net = ws::spawn(app.config.ws_url(), ok.token, net_tx);

    // Pump until the app is online.
    let deadline = tokio::time::Instant::now() + STEP;
    while !matches!(app.conn, ConnState::Online { .. }) {
        let event = tokio::time::timeout_at(deadline, net_rx.recv())
            .await
            .expect("net event in time")
            .expect("supervisor alive");
        update(&mut app, Event::Net(event));
    }
    assert_eq!(app.me.as_ref().map(|me| me.username.as_str()), Some("neo"));
    assert!(app.active.is_some(), "landed in #general");

    // Type and submit a message through the reducer, deliver its effects to
    // the supervisor — the exact main-loop contract.
    for c in "it works :rocket:".chars() {
        update(
            &mut app,
            Event::Key(ratatui::crossterm::event::KeyEvent::from(
                ratatui::crossterm::event::KeyCode::Char(c),
            )),
        );
    }
    update(
        &mut app,
        Event::Key(ratatui::crossterm::event::KeyEvent::from(
            ratatui::crossterm::event::KeyCode::Enter,
        )),
    );
    for effect in std::mem::take(&mut app.effects) {
        if let Effect::Send(frame) = effect {
            net.outgoing.send(frame).await.expect("send to supervisor");
        }
    }

    // The server's Posted (and room echo) come back and settle the pending send.
    let deadline = tokio::time::Instant::now() + STEP;
    loop {
        let room = app.active_room().expect("room");
        if room.pending.is_empty() && !room.messages.is_empty() {
            break;
        }
        let event = tokio::time::timeout_at(deadline, net_rx.recv())
            .await
            .expect("posted arrives in time")
            .expect("supervisor alive");
        update(&mut app, Event::Net(event));
    }
    let room = app.active_room().expect("room");
    assert!(
        room.messages.iter().any(|m| m.body == "it works :rocket:"),
        "message persisted and echoed"
    );

    net.shutdown.cancel();
}

#[tokio::test]
async fn a_dead_token_surfaces_as_auth_rejection_not_a_retry_loop() {
    let (config, _db) = spawn_server().await;

    let (net_tx, mut net_rx) = mpsc::channel::<NetEvent>(16);
    let net = ws::spawn(config.ws_url(), "forged-token".to_owned(), net_tx);

    let deadline = tokio::time::Instant::now() + STEP;
    let rejected = loop {
        let event = tokio::time::timeout_at(deadline, net_rx.recv())
            .await
            .expect("event in time")
            .expect("supervisor alive");
        match event {
            NetEvent::AuthRejected { .. } => break true,
            NetEvent::Disconnected { .. } => break false,
            _ => {}
        }
    };
    assert!(rejected, "a 401 must be terminal, not retried");
    net.shutdown.cancel();
}

#[tokio::test]
async fn the_supervisor_retries_when_no_server_listens() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    // A port with nothing behind it.
    let config = ClientConfig {
        server: "127.0.0.1".to_owned(),
        port: 1, // reserved, nothing listens
        ..ClientConfig::default()
    };

    let (net_tx, mut net_rx) = mpsc::channel::<NetEvent>(16);
    let net = ws::spawn(config.ws_url(), "irrelevant".to_owned(), net_tx);

    let mut saw_connecting = false;
    let mut saw_disconnected = false;
    let deadline = tokio::time::Instant::now() + STEP;
    while !(saw_connecting && saw_disconnected) {
        let event = tokio::time::timeout_at(deadline, net_rx.recv())
            .await
            .expect("events in time")
            .expect("supervisor alive");
        match event {
            NetEvent::Connecting { .. } => saw_connecting = true,
            NetEvent::Disconnected { retry_in, .. } => {
                saw_disconnected = true;
                assert!(retry_in <= Duration::from_secs(30));
            }
            _ => {}
        }
    }
    net.shutdown.cancel();
}
