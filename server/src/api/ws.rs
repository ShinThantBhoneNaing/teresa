//! The WebSocket endpoint and its per-connection actor.
//!
//! Each accepted socket becomes three kinds of task sharing one bounded
//! mailbox:
//!
//! ```text
//!             ┌────────────── reader (this task) ──────────────┐
//!   socket ──►│ decode ClientFrame → rate limit → Core method  │
//!             └────────────────────────────────────────────────┘
//!                                              │ direct replies
//!   hub room A ──► forwarder ─┐                ▼
//!   hub room B ──► forwarder ─┼──► mailbox (mpsc, bounded) ──► writer ──► socket
//!   hub global ──► forwarder ─┘                                  ▲
//!                                                       keepalive pings
//! ```
//!
//! The mailbox being bounded is the server's memory ceiling per client: a
//! connection that cannot drain its own traffic lags its broadcast receivers,
//! the forwarder notices, and the whole actor shuts down rather than buffering
//! without limit.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use teresa_proto::{ClientFrame, ConnectionId, ErrorCode, RoomId, ServerFrame, UserId};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::error::ServerError;
use crate::hub::SharedFrame;
use crate::repo::UserRow;
use crate::state::SharedState;

/// How often the server pings an idle connection.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
/// How long a kicked connection lingers so its farewell reaches the socket.
const KICK_FLUSH: Duration = Duration::from_millis(250);
/// Silence longer than this — no frames, no pongs — means the peer is gone.
const KEEPALIVE_DEADLINE: Duration = Duration::from_secs(75);

/// `GET /api/v1/ws` — authenticates, then upgrades.
///
/// Authentication happens *before* the upgrade so that a bad token costs one
/// HTTP 401 and never a socket.
pub async fn upgrade(
    State(state): State<SharedState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let token = match super::auth::bearer_token(&headers) {
        Ok(token) => token.to_owned(),
        Err(error) => return error.into_response(),
    };
    let user = match state.core.authenticate(&token).await {
        Ok(user) => user,
        Err(error) => return error.into_response(),
    };

    ws.on_upgrade(move |socket| connection(state, socket, user))
}

/// Runs one connection from upgrade to close.
async fn connection(state: SharedState, socket: WebSocket, user: UserRow) {
    let conn = ConnectionId::new();
    let username = user.username.clone();
    tracing::info!(user = %username, %conn, "connected");

    let (mut sink, stream) = socket.split();

    // The mailbox every outbound frame funnels through on its way to the
    // socket, and the control inbox the hub uses for targeted pushes (invites,
    // DMs opened from the far side). Control frames pass through the actor
    // rather than going straight to the writer, because some of them change
    // what the actor must be subscribed to.
    let depth = state.config().limits.outbox_depth;
    let (mailbox, outbox) = mpsc::channel::<SharedFrame>(depth);
    let (control_tx, control_rx) = mpsc::channel::<SharedFrame>(depth);

    // Register with the core; on refusal (connection cap), say why and hang up.
    let ready = match state.core.connect(&user, conn, control_tx).await {
        Ok(ready) => ready,
        Err(error) => {
            let frame = error.to_frame();
            if let Ok(text) = frame.encode() {
                let _ = sink.send(WsMessage::Text(text.into())).await;
            }
            let _ = sink.close().await;
            return;
        }
    };

    let mut actor = Actor {
        state: state.clone(),
        user,
        conn,
        mailbox,
        subscriptions: HashMap::new(),
        forwarders: JoinSet::new(),
        cancel: CancellationToken::new(),
        liveness: std::sync::Arc::new(Mutex::new(Instant::now())),
    };

    // Every room the user was already in gets a forwarder, plus the global
    // channel; then the Ready frame goes out as the very first thing the
    // client hears.
    if let ServerFrame::Ready { rooms, .. } = &ready {
        for joined in rooms {
            actor.subscribe(joined.room.id);
        }
    }
    actor.spawn_global_forwarder();
    let _ = actor.mailbox.send(ready.into()).await;

    let writer = tokio::spawn(writer_task(
        sink,
        outbox,
        actor.cancel.clone(),
        std::sync::Arc::clone(&actor.liveness),
    ));

    actor.read_loop(stream, control_rx).await;

    // Teardown, in dependency order: stop producing, then stop writing, then
    // tell the core the connection is gone.
    actor.cancel.cancel();
    actor.forwarders.shutdown().await;
    drop(actor.mailbox);
    let _ = writer.await;
    if let Err(error) = state.core.disconnect(actor.user.id, conn).await {
        tracing::warn!(user = %username, %error, "disconnect bookkeeping failed");
    }
    tracing::info!(user = %username, %conn, "disconnected");
}

/// Drains the mailbox onto the socket and keeps the connection alive.
async fn writer_task(
    mut sink: SplitSink<WebSocket, WsMessage>,
    mut outbox: mpsc::Receiver<SharedFrame>,
    cancel: CancellationToken,
    liveness: std::sync::Arc<Mutex<Instant>>,
) {
    let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            () = cancel.cancelled() => break,

            frame = outbox.recv() => {
                let Some(frame) = frame else { break };
                let Ok(text) = frame.encode() else {
                    tracing::error!("outbound frame failed to serialise");
                    continue;
                };
                if sink.send(WsMessage::Text(text.into())).await.is_err() {
                    break; // peer hung up mid-write
                }
            }

            _ = keepalive.tick() => {
                let last_seen = *liveness.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if last_seen.elapsed() > KEEPALIVE_DEADLINE {
                    tracing::debug!("keepalive deadline missed; dropping connection");
                    break;
                }
                if sink.send(WsMessage::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }

    // Politeness only; the peer may already be gone.
    let _ = sink.close().await;
    cancel.cancel();
}

/// The reader half's state.
struct Actor {
    state: SharedState,
    /// Cached profile, refreshed when this connection changes it. Another
    /// terminal's `/nick` leaves this stale until the next reconnect, which
    /// costs at most an outdated display name on messages sent meanwhile.
    user: UserRow,
    conn: ConnectionId,
    mailbox: mpsc::Sender<SharedFrame>,
    /// Live room subscriptions, so `Leave` can stop exactly one forwarder.
    subscriptions: HashMap<RoomId, AbortHandle>,
    forwarders: JoinSet<()>,
    cancel: CancellationToken,
    /// Last moment the peer proved it exists.
    liveness: std::sync::Arc<Mutex<Instant>>,
}

impl Actor {
    /// Consumes socket messages and control pushes until the peer leaves or
    /// the actor cancels.
    async fn read_loop(
        &mut self,
        mut stream: SplitStream<WebSocket>,
        mut control: mpsc::Receiver<SharedFrame>,
    ) {
        loop {
            enum Input {
                Socket(Option<Result<WsMessage, axum::Error>>),
                Control(SharedFrame),
            }

            let input = tokio::select! {
                () = self.cancel.cancelled() => break,
                message = stream.next() => Input::Socket(message),
                // The control channel outlives the loop (the hub holds the
                // sender until disconnect), so recv() = None cannot happen
                // while we are here; treat it as shutdown anyway.
                pushed = control.recv() => match pushed {
                    Some(frame) => Input::Control(frame),
                    None => break,
                },
            };

            match input {
                Input::Control(frame) => {
                    // An admin removed this account: say so, give the writer
                    // a moment to flush it, and hang up.
                    if matches!(frame.as_ref(), ServerFrame::Kicked { .. }) {
                        let _ = self.mailbox.send(frame).await;
                        tokio::time::sleep(KICK_FLUSH).await;
                        break;
                    }
                    // A push that grants this user a room — an invite accepted
                    // elsewhere, a DM opened from the far side — must also
                    // start the flow of that room's traffic to this socket.
                    if let ServerFrame::Joined { room, .. } = frame.as_ref() {
                        self.subscribe(room.room.id);
                    }
                    if self.mailbox.send(frame).await.is_err() {
                        break;
                    }
                }
                Input::Socket(message) => {
                    let Some(Ok(message)) = message else { break };

                    *self
                        .liveness
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();

                    match message {
                        WsMessage::Text(text) => match ClientFrame::decode(&text) {
                            Ok(frame) => {
                                if self.dispatch(frame).await.is_err() {
                                    break; // mailbox gone: writer died
                                }
                            }
                            Err(error) => {
                                tracing::debug!(user = %self.user.username, %error, "undecodable frame");
                                let refused =
                                    ServerFrame::error(ErrorCode::Invalid, "unrecognised frame");
                                if self.reply(refused).await.is_err() {
                                    break;
                                }
                            }
                        },
                        WsMessage::Close(_) => break,
                        // Pongs already updated liveness above; binary is not
                        // part of the protocol and is ignored rather than fatal.
                        WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Binary(_) => {}
                    }
                }
            }
        }
    }

    /// Routes one decoded frame. `Err` means the mailbox is closed and the
    /// connection is over; protocol-level failures are answered in-band.
    ///
    /// One arm per protocol frame — the length is the protocol's width, and
    /// splitting it would only scatter the dispatch table.
    #[allow(clippy::too_many_lines)]
    async fn dispatch(&mut self, frame: ClientFrame) -> Result<(), ()> {
        // The flood budget covers everything but the frames that are cheap by
        // construction; a refusal names the wait.
        if !frame.is_cheap() {
            let decision = self.state.message_limiter.check(self.conn);
            if !decision.is_allowed() {
                let refusal = ServerError::RateLimited {
                    retry_after_secs: decision.retry_after_secs(),
                };
                return self.reply(refusal.to_frame()).await;
            }
        }

        match frame {
            ClientFrame::Send { room, body, nonce } => {
                let result = self
                    .state
                    .core
                    .post_message(&self.user, room, &body, teresa_proto::MessageKind::Chat)
                    .await;
                match result {
                    Ok(message) => self.reply(ServerFrame::Posted { message, nonce }).await,
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::Emote { room, body, nonce } => {
                let result = self
                    .state
                    .core
                    .post_message(&self.user, room, &body, teresa_proto::MessageKind::Emote)
                    .await;
                match result {
                    Ok(message) => self.reply(ServerFrame::Posted { message, nonce }).await,
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::Typing { room, active } => {
                // Best-effort by nature; a failed typing hint merits no error
                // frame, just a log line.
                if let Err(error) = self.state.core.typing(&self.user, room, active).await {
                    tracing::debug!(user = %self.user.username, %error, "typing relay refused");
                }
                Ok(())
            }

            ClientFrame::Join { name } => {
                match self.state.core.join_room(&self.user, &name).await {
                    Ok((room, members)) => {
                        self.subscribe(room.room.id);
                        self.reply(ServerFrame::Joined { room, members }).await
                    }
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::Leave { room } => {
                match self.state.core.leave_room(&self.user, room).await {
                    Ok(()) => {
                        self.unsubscribe(room);
                        self.reply(ServerFrame::Left { room }).await
                    }
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::CreateRoom {
                name,
                topic,
                visibility,
            } => {
                let result = self
                    .state
                    .core
                    .create_room(&self.user, &name, topic.as_deref(), visibility)
                    .await;
                match result {
                    Ok((room, members)) => {
                        self.subscribe(room.room.id);
                        self.reply(ServerFrame::Joined { room, members }).await
                    }
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::DeleteRoom { room } => {
                match self.state.core.delete_room(&self.user, room).await {
                    // The RoomDeleted announcement arrived through the room's
                    // own channel before it closed; nothing further to say.
                    Ok(()) => Ok(()),
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::Invite { room, username } => {
                match self.state.core.invite(&self.user, room, &username).await {
                    Ok(()) => {
                        self.reply(ServerFrame::Notice {
                            text: format!("invited @{username}"),
                        })
                        .await
                    }
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::OpenDm { username } => {
                match self.state.core.open_dm(&self.user, &username).await {
                    Ok((room, members)) => {
                        self.subscribe(room.room.id);
                        self.reply(ServerFrame::Joined { room, members }).await
                    }
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::History {
                room,
                before,
                limit,
            } => {
                match self
                    .state
                    .core
                    .history(&self.user, room, before, limit)
                    .await
                {
                    Ok((messages, complete)) => {
                        self.reply(ServerFrame::History {
                            room,
                            messages,
                            complete,
                        })
                        .await
                    }
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::Members { room } => {
                match self.state.core.members(self.user.id, room).await {
                    Ok(members) => self.reply(ServerFrame::Members { room, members }).await,
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::ListRooms => match self.state.core.list_rooms(self.user.id).await {
                Ok(rooms) => self.reply(ServerFrame::RoomList { rooms }).await,
                Err(error) => self.reply_error(error).await,
            },

            ClientFrame::SetStatus { status, text } => {
                match self
                    .state
                    .core
                    .set_status(&self.user, status, text.as_deref())
                    .await
                {
                    Ok(()) => {
                        // Keep the cached profile honest for future stamps.
                        self.user.status = status;
                        self.user.status_text = text;
                        Ok(())
                    }
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::SetDisplayName { display_name } => {
                match self
                    .state
                    .core
                    .set_display_name(&self.user, &display_name)
                    .await
                {
                    Ok(updated) => {
                        self.user.display_name = updated.display_name;
                        // The ProfileUpdated broadcast reaches this client via
                        // the global forwarder like everyone else.
                        Ok(())
                    }
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::MarkRead { room } => {
                if let Err(error) = self.state.core.mark_read(self.user.id, room).await {
                    tracing::debug!(user = %self.user.username, %error, "mark_read refused");
                }
                Ok(())
            }

            ClientFrame::Maintenance { op } => {
                match self.state.core.maintenance(&self.user, op).await {
                    Ok(report) => self.reply(ServerFrame::Maintenance { report }).await,
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::ListAccounts => match self.state.core.accounts(&self.user).await {
                Ok(users) => self.reply(ServerFrame::Accounts { users }).await,
                Err(error) => self.reply_error(error).await,
            },

            ClientFrame::RemoveAccount { username } => {
                match self
                    .state
                    .core
                    .remove_account_as(&self.user, &username)
                    .await
                {
                    Ok(user) => {
                        self.reply(ServerFrame::AccountRemoved {
                            username: user.username,
                        })
                        .await
                    }
                    Err(error) => self.reply_error(error).await,
                }
            }

            ClientFrame::Ping { nonce } => self.reply(ServerFrame::Pong { nonce }).await,
        }
    }

    /// Queues a direct reply. `Err` only when the connection is finished.
    async fn reply(&self, frame: ServerFrame) -> Result<(), ()> {
        self.mailbox.send(frame.into()).await.map_err(|_| ())
    }

    /// Answers with an error frame, logging server faults.
    async fn reply_error(&self, error: ServerError) -> Result<(), ()> {
        if error.is_server_fault() {
            tracing::error!(user = %self.user.username, ?error, "request failed");
        }
        self.reply(error.to_frame()).await
    }

    /// Starts forwarding a room's broadcast into this connection's mailbox.
    fn subscribe(&mut self, room: RoomId) {
        if self.subscriptions.contains_key(&room) {
            return;
        }
        let receiver = self.state.core.hub().subscribe_room(room);
        let handle = self.spawn_forwarder(receiver, Some(room));
        self.subscriptions.insert(room, handle);
    }

    /// Stops forwarding a room.
    fn unsubscribe(&mut self, room: RoomId) {
        if let Some(handle) = self.subscriptions.remove(&room) {
            handle.abort();
        }
    }

    /// Starts forwarding the global channel.
    fn spawn_global_forwarder(&mut self) {
        let receiver = self.state.core.hub().subscribe_global();
        let _ = self.spawn_forwarder(receiver, None);
    }

    /// The common forwarder body: broadcast → mailbox until lag, closure or
    /// cancellation. Lagging is fatal to the connection — a client that
    /// missed frames has a hole in its world it cannot detect, so an honest
    /// disconnect (and clean reconnect) beats silent gaps.
    fn spawn_forwarder(
        &mut self,
        mut receiver: tokio::sync::broadcast::Receiver<SharedFrame>,
        room: Option<RoomId>,
    ) -> AbortHandle {
        let mailbox = self.mailbox.clone();
        let cancel = self.cancel.clone();
        let user = self.user.id;

        self.forwarders.spawn(async move {
            loop {
                let frame = tokio::select! {
                    () = cancel.cancelled() => break,
                    frame = receiver.recv() => frame,
                };
                match frame {
                    Ok(frame) => {
                        if !relevant_to(&frame, user) {
                            continue;
                        }
                        if mailbox.send(frame).await.is_err() {
                            break; // connection is shutting down
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        tracing::warn!(?room, missed, "connection lagged; disconnecting");
                        let overloaded = ServerFrame::error(
                            ErrorCode::Overloaded,
                            "you fell too far behind; reconnect",
                        );
                        let _ = mailbox.try_send(overloaded.into());
                        cancel.cancel();
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }
}

/// Broadcast-level filtering that is cheaper done here than at every client:
/// a user's own typing echo is pure noise, and nobody needs to be told they
/// themself are typing.
fn relevant_to(frame: &SharedFrame, user: UserId) -> bool {
    match frame.as_ref() {
        ServerFrame::Typing { user: typist, .. } => *typist != user,
        _ => true,
    }
}
