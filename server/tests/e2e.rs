//! End-to-end tests: a real server on a real port, driven by real HTTP and
//! WebSocket clients. Everything a terminal client will do, done here first.

use std::net::SocketAddr;
use std::time::Duration;

use axum_server::Handle;
use futures::{SinkExt, StreamExt};
use teresa_proto::{ClientFrame, JoinedRoom, RoomId, ServerFrame, UserStatus};
use teresa_server::repo::sqlite::SqliteStore;
use teresa_server::services::Core;
use teresa_server::state::AppState;
use teresa_server::{api, ServerConfig};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Nothing in these tests should legitimately take longer than this.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// A running server plus the paths to reach it.
struct TestServer {
    http: String,
    ws: String,
    handle: Handle<SocketAddr>,
    // Held for its Drop: deletes the database file with the test.
    _db: tempfile::NamedTempFile,
}

impl TestServer {
    /// Boots a fully assembled server on an OS-assigned port.
    async fn spawn() -> Self {
        Self::spawn_with(|_| {}).await
    }

    /// Boots a server after letting the test bend the configuration.
    async fn spawn_with(customise: impl FnOnce(&mut ServerConfig)) -> Self {
        // The binaries install this in main(); tests stand in for main here.
        // (reqwest is built with rustls-no-provider via workspace feature
        // unification, so a provider must exist before any client is built.)
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();

        // First caller wins; later inits fail harmlessly. RUST_LOG applies.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .with_test_writer()
            .try_init();

        let db = tempfile::NamedTempFile::new().expect("temp database file");
        let mut config = ServerConfig {
            database: db.path().to_path_buf(),
            ..ServerConfig::default()
        };
        customise(&mut config);

        let store = SqliteStore::open(db.path()).await.expect("store opens");
        let core = Core::new(store, config);
        core.bootstrap().await.expect("bootstrap");
        let state = AppState::new(core);

        let handle = Handle::new();
        let server = axum_server::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            .handle(handle.clone())
            .serve(api::router(state).into_make_service_with_connect_info::<SocketAddr>());
        tokio::spawn(server);

        let addr = tokio::time::timeout(STEP_TIMEOUT, handle.listening())
            .await
            .expect("server binds promptly")
            .expect("server binds");

        Self {
            http: format!("http://{addr}/api/v1"),
            ws: format!("ws://{addr}/api/v1/ws"),
            handle,
            _db: db,
        }
    }

    /// Registers an account over REST and returns its bearer token.
    async fn register(&self, username: &str) -> String {
        let response = reqwest::Client::new()
            .post(format!("{}/auth/register", self.http))
            .json(&serde_json::json!({
                "username": username,
                "password": "a perfectly fine password",
                "display_name": username,
            }))
            .send()
            .await
            .expect("register request");
        assert_eq!(response.status(), 201, "register {username}");
        response.json::<serde_json::Value>().await.expect("json")["token"]
            .as_str()
            .expect("token present")
            .to_owned()
    }

    /// Opens an authenticated WebSocket and consumes the `Ready` frame.
    async fn connect(&self, token: &str) -> (Client, teresa_proto::User, Vec<JoinedRoom>) {
        let mut request = self.ws.as_str().into_client_request().expect("ws url");
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {token}").parse().expect("header"),
        );
        let (stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("websocket connects");
        let mut client = Client { stream };

        let ready = client.next_frame().await;
        let ServerFrame::Ready { user, rooms, .. } = ready else {
            panic!("first frame must be Ready, got {ready:?}");
        };
        (client, user, rooms)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.shutdown();
    }
}

/// A test-side WebSocket speaking the protocol.
struct Client {
    stream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
}

impl Client {
    async fn send(&mut self, frame: &ClientFrame) {
        let text = frame.encode().expect("frame encodes");
        self.stream
            .send(WsMessage::Text(text.into()))
            .await
            .expect("send succeeds");
    }

    /// The next protocol frame, skipping transport noise.
    async fn next_frame(&mut self) -> ServerFrame {
        loop {
            let message = tokio::time::timeout(STEP_TIMEOUT, self.stream.next())
                .await
                .expect("a frame arrives in time")
                .expect("stream is open")
                .expect("no transport error");
            match message {
                WsMessage::Text(text) => {
                    return ServerFrame::decode(&text).expect("server frames decode")
                }
                WsMessage::Ping(_) | WsMessage::Pong(_) => {}
                other => panic!("unexpected transport message: {other:?}"),
            }
        }
    }

    /// Reads frames until one satisfies the predicate, discarding interleaved
    /// broadcast traffic (presence, other rooms) that tests should not have to
    /// order-match.
    async fn frame_where<F>(&mut self, describe: &str, mut predicate: F) -> ServerFrame
    where
        F: FnMut(&ServerFrame) -> bool,
    {
        for _ in 0..50 {
            let frame = self.next_frame().await;
            if predicate(&frame) {
                return frame;
            }
        }
        panic!("gave up waiting for: {describe}");
    }
}

fn general_room(rooms: &[JoinedRoom]) -> RoomId {
    rooms
        .iter()
        .find(|r| r.room.name == "general")
        .expect("#general in Ready")
        .room
        .id
}

#[tokio::test]
async fn rest_auth_flow_works_end_to_end() {
    let server = TestServer::spawn().await;
    let http = reqwest::Client::new();

    // Liveness.
    let health = http
        .get(format!("{}/healthz", server.http))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);

    // Register, duplicate, wrong password, correct login, logout.
    let token = server.register("neo").await;

    let duplicate = http
        .post(format!("{}/auth/register", server.http))
        .json(&serde_json::json!({
            "username": "neo",
            "password": "a different password",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), 409);

    let wrong = http
        .post(format!("{}/auth/login", server.http))
        .json(&serde_json::json!({"username": "neo", "password": "wrong password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);

    let login = http
        .post(format!("{}/auth/login", server.http))
        .json(&serde_json::json!({"username": "neo", "password": "a perfectly fine password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200);

    let logout = http
        .post(format!("{}/auth/logout", server.http))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 204);

    // The revoked token no longer upgrades.
    let mut request = server.ws.as_str().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let refused = tokio_tungstenite::connect_async(request).await;
    assert!(refused.is_err(), "revoked token must not open a socket");

    // Metrics saw the account.
    let metrics: serde_json::Value = http
        .get(format!("{}/metrics", server.http))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(metrics["total_users"], 1);
}

#[tokio::test]
async fn a_missing_or_garbage_token_cannot_open_a_socket() {
    let server = TestServer::spawn().await;

    let bare = server.ws.as_str().into_client_request().unwrap();
    assert!(tokio_tungstenite::connect_async(bare).await.is_err());

    let mut forged = server.ws.as_str().into_client_request().unwrap();
    forged
        .headers_mut()
        .insert("authorization", "Bearer forged".parse().unwrap());
    assert!(tokio_tungstenite::connect_async(forged).await.is_err());
}

#[tokio::test]
async fn two_clients_converse_in_a_room() {
    let server = TestServer::spawn().await;
    let alice_token = server.register("alice").await;
    let bob_token = server.register("bob").await;

    let (mut alice, _, alice_rooms) = server.connect(&alice_token).await;
    let (mut bob, _, _) = server.connect(&bob_token).await;
    let general = general_room(&alice_rooms);

    alice
        .send(&ClientFrame::Send {
            room: general,
            body: "wake up, bob 🐇".to_owned(),
            nonce: "n-1".to_owned(),
        })
        .await;

    // The sender gets an acknowledgement carrying her nonce…
    let posted = alice
        .frame_where("Posted ack", |f| matches!(f, ServerFrame::Posted { .. }))
        .await;
    let ServerFrame::Posted { message, nonce } = posted else {
        unreachable!()
    };
    assert_eq!(nonce, "n-1");
    assert_eq!(message.body, "wake up, bob 🐇");

    // …and the other member sees the message itself.
    let received = bob
        .frame_where(
            "bob sees the message",
            |f| matches!(f, ServerFrame::Message { message } if message.body == "wake up, bob 🐇"),
        )
        .await;
    let ServerFrame::Message { message: bobs_copy } = received else {
        unreachable!()
    };
    assert_eq!(bobs_copy.author.as_ref().unwrap().username, "alice");
    assert_eq!(bobs_copy.id, message.id, "both sides hold the same message");

    // Alice also receives her own copy through the room broadcast — same id,
    // which is exactly what lets clients de-duplicate against the Posted ack.
    alice
        .frame_where(
            "alice's room echo",
            |f| matches!(f, ServerFrame::Message { message: m } if m.id == message.id),
        )
        .await;
}

#[tokio::test]
async fn a_dm_reaches_a_connected_peer_without_any_refresh() {
    let server = TestServer::spawn().await;
    let alice_token = server.register("alice").await;
    let bob_token = server.register("bob").await;

    let (mut alice, _, _) = server.connect(&alice_token).await;
    let (mut bob, _, _) = server.connect(&bob_token).await;

    alice
        .send(&ClientFrame::OpenDm {
            username: "bob".to_owned(),
        })
        .await;

    // Alice's side opens…
    let opened = alice
        .frame_where("alice's Joined", |f| {
            matches!(f, ServerFrame::Joined { .. })
        })
        .await;
    let ServerFrame::Joined { room, .. } = opened else {
        unreachable!()
    };
    assert_eq!(room.peer.as_deref(), Some("bob"));
    let dm_room = room.room.id;

    // …bob's side is pushed open without him doing anything…
    let pushed = bob
        .frame_where("bob's pushed Joined", |f| {
            matches!(f, ServerFrame::Joined { .. })
        })
        .await;
    let ServerFrame::Joined { room, .. } = pushed else {
        unreachable!()
    };
    assert_eq!(room.peer.as_deref(), Some("alice"));
    assert_eq!(room.room.id, dm_room);

    // …and a message flows through.
    alice
        .send(&ClientFrame::Send {
            room: dm_room,
            body: "meet me at the arcade".to_owned(),
            nonce: "n-2".to_owned(),
        })
        .await;
    bob.frame_where("bob receives the dm", |f| {
        matches!(f, ServerFrame::Message { message }
            if message.room_id == dm_room && message.body == "meet me at the arcade")
    })
    .await;
}

#[tokio::test]
async fn presence_and_typing_flow_between_clients() {
    let server = TestServer::spawn().await;
    let alice_token = server.register("alice").await;
    let bob_token = server.register("bob").await;

    let (mut alice, _, alice_rooms) = server.connect(&alice_token).await;
    let (mut bob, bob_user, _) = server.connect(&bob_token).await;
    let general = general_room(&alice_rooms);

    // Bob announces he is deep in code.
    bob.send(&ClientFrame::SetStatus {
        status: UserStatus::Coding,
        text: Some("in the construct".to_owned()),
    })
    .await;

    let presence = alice
        .frame_where("bob's status reaches alice", |f| {
            matches!(f, ServerFrame::Presence { user, status: UserStatus::Coding, .. }
                if *user == bob_user.id)
        })
        .await;
    let ServerFrame::Presence { text, .. } = presence else {
        unreachable!()
    };
    assert_eq!(text.as_deref(), Some("in the construct"));

    // Bob starts typing; alice sees it; bob never hears his own echo.
    bob.send(&ClientFrame::Typing {
        room: general,
        active: true,
    })
    .await;
    alice
        .frame_where(
            "typing indicator",
            |f| matches!(f, ServerFrame::Typing { user, active: true, .. } if *user == bob_user.id),
        )
        .await;

    // Everything queued for bob up to his pong may include his own Presence
    // echo (that is how his other terminals stay in sync) — but never an echo
    // of his own typing, which the server filters at the fan-out.
    bob.send(&ClientFrame::Ping { nonce: 99 }).await;
    loop {
        let frame = bob.next_frame().await;
        match frame {
            ServerFrame::Pong { nonce: 99 } => break,
            ServerFrame::Typing { user, .. } => {
                assert_ne!(user, bob_user.id, "bob heard his own typing echo");
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn room_lifecycle_over_the_wire() {
    let server = TestServer::spawn().await;
    let alice_token = server.register("alice").await;
    let bob_token = server.register("bob").await;

    let (mut alice, _, _) = server.connect(&alice_token).await;
    let (mut bob, _, _) = server.connect(&bob_token).await;

    // Alice creates a room; bob hears about it globally.
    alice
        .send(&ClientFrame::CreateRoom {
            name: "heist".to_owned(),
            topic: Some("the plan".to_owned()),
            visibility: teresa_proto::Visibility::Public,
        })
        .await;
    let created = alice
        .frame_where(
            "alice's Joined for heist",
            |f| matches!(f, ServerFrame::Joined { room, .. } if room.room.name == "heist"),
        )
        .await;
    let ServerFrame::Joined { room, .. } = created else {
        unreachable!()
    };
    let heist = room.room.id;

    bob.frame_where(
        "bob told of the new room",
        |f| matches!(f, ServerFrame::RoomCreated { room } if room.name == "heist"),
    )
    .await;

    // Bob joins it and both see the membership grow.
    bob.send(&ClientFrame::Join {
        name: "heist".to_owned(),
    })
    .await;
    bob.frame_where(
        "bob's Joined",
        |f| matches!(f, ServerFrame::Joined { room, .. } if room.room.id == heist),
    )
    .await;
    alice
        .frame_where("alice sees bob join", |f| {
            matches!(f, ServerFrame::MemberJoined { room, user }
                if *room == heist && user.username == "bob")
        })
        .await;

    // History over the wire includes the join narration.
    bob.send(&ClientFrame::History {
        room: heist,
        before: None,
        limit: 50,
    })
    .await;
    let history = bob
        .frame_where("history page", |f| matches!(f, ServerFrame::History { .. }))
        .await;
    let ServerFrame::History {
        messages, complete, ..
    } = history
    else {
        unreachable!()
    };
    assert!(complete);
    assert!(messages.iter().any(|m| m.body == "@bob joined"));

    // Deleting the room reaches both members.
    alice.send(&ClientFrame::DeleteRoom { room: heist }).await;
    alice
        .frame_where(
            "alice sees deletion",
            |f| matches!(f, ServerFrame::RoomDeleted { room } if *room == heist),
        )
        .await;
    bob.frame_where(
        "bob sees deletion",
        |f| matches!(f, ServerFrame::RoomDeleted { room } if *room == heist),
    )
    .await;
}

#[tokio::test]
async fn auth_rate_limiting_refuses_with_retry_after() {
    let server = TestServer::spawn_with(|config| {
        config.limits.auth_per_minute = 2;
    })
    .await;
    let http = reqwest::Client::new();

    let attempt = || async {
        http.post(format!("{}/auth/login", server.http))
            .json(&serde_json::json!({"username": "ghost", "password": "whatever this is"}))
            .send()
            .await
            .unwrap()
    };

    assert_eq!(attempt().await.status(), 401);
    assert_eq!(attempt().await.status(), 401);

    let limited = attempt().await;
    assert_eq!(limited.status(), 429);
    let retry_after: u64 = limited
        .headers()
        .get("retry-after")
        .expect("retry-after header")
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(retry_after >= 1, "the hint must be actionable");
}

#[tokio::test]
async fn message_flood_is_throttled_per_connection() {
    let server = TestServer::spawn_with(|config| {
        config.limits.messages_per_10s = 3;
    })
    .await;
    let token = server.register("chatterbox").await;
    let (mut client, _, rooms) = server.connect(&token).await;
    let general = general_room(&rooms);

    for i in 0..4 {
        client
            .send(&ClientFrame::Send {
                room: general,
                body: format!("spam {i}"),
                nonce: format!("n-{i}"),
            })
            .await;
    }

    // Three land, the fourth bounces.
    client
        .frame_where("rate limit error", |f| {
            matches!(
                f,
                ServerFrame::Error {
                    code: teresa_proto::ErrorCode::RateLimited,
                    ..
                }
            )
        })
        .await;
}
