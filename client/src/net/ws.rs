//! The connection supervisor: one task that owns the WebSocket for the whole
//! life of the app, reconnecting with backoff whenever it drops.
//!
//! The UI never touches the socket. It receives [`NetEvent`]s and sends
//! [`ClientFrame`]s; everything about transport — TLS, backoff, the difference
//! between "server restarting" and "wifi died" — stays in here.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use teresa_proto::{ClientFrame, ServerFrame};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use tokio_util::sync::CancellationToken;

/// Everything the network layer can tell the UI.
#[derive(Debug)]
pub enum NetEvent {
    /// Dialling attempt `attempt` (1-based) is underway.
    Connecting {
        /// Which attempt this is since the last successful connection.
        attempt: u32,
    },
    /// The socket is open; a `Ready` frame will follow as a [`NetEvent::Frame`].
    Connected,
    /// A protocol frame arrived.
    Frame(ServerFrame),
    /// The socket dropped; the supervisor will retry in `retry_in`.
    Disconnected {
        /// Human-readable cause.
        reason: String,
        /// How long until the next attempt.
        retry_in: Duration,
    },
    /// The server rejected our session; retrying cannot help until the user
    /// logs in again. The supervisor has stopped.
    AuthRejected {
        /// The server's explanation.
        message: String,
    },
}

/// Handles the UI holds to talk to the supervisor.
#[derive(Debug)]
pub struct NetHandle {
    /// Frames the UI wants sent. Dropped (with a toast via `Disconnected`)
    /// while offline; chat frames are not worth queueing into a stale future.
    pub outgoing: mpsc::Sender<ClientFrame>,
    /// Pokes the supervisor to abandon its backoff and dial now (Ctrl+R).
    pub reconnect_now: mpsc::Sender<()>,
    /// Stops the supervisor entirely (quit).
    pub shutdown: CancellationToken,
}

/// Spawns the supervisor. `events` is the UI's inbox.
#[must_use]
pub fn spawn(ws_url: String, token: String, events: mpsc::Sender<NetEvent>) -> NetHandle {
    let (outgoing_tx, outgoing_rx) = mpsc::channel(64);
    let (reconnect_tx, reconnect_rx) = mpsc::channel(1);
    let shutdown = CancellationToken::new();

    tokio::spawn(supervisor(
        ws_url,
        token,
        events,
        outgoing_rx,
        reconnect_rx,
        shutdown.clone(),
    ));

    NetHandle {
        outgoing: outgoing_tx,
        reconnect_now: reconnect_tx,
        shutdown,
    }
}

/// Backoff schedule: fast first retry, then doubling to a ceiling. The first
/// retry being nearly immediate matters — the commonest disconnect is a server
/// restart that is back before a human notices.
fn backoff(attempt: u32) -> Duration {
    match attempt {
        0 | 1 => Duration::from_millis(500),
        2 => Duration::from_secs(1),
        3 => Duration::from_secs(2),
        4 => Duration::from_secs(5),
        5 => Duration::from_secs(10),
        _ => Duration::from_secs(30),
    }
}

async fn supervisor(
    ws_url: String,
    token: String,
    events: mpsc::Sender<NetEvent>,
    mut outgoing: mpsc::Receiver<ClientFrame>,
    mut reconnect_now: mpsc::Receiver<()>,
    shutdown: CancellationToken,
) {
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;
        if events.send(NetEvent::Connecting { attempt }).await.is_err() {
            return; // UI is gone
        }

        match dial(&ws_url, &token).await {
            Ok(socket) => {
                attempt = 0;
                if events.send(NetEvent::Connected).await.is_err() {
                    return;
                }
                let outcome = pump(socket, &events, &mut outgoing, &shutdown).await;
                match outcome {
                    PumpEnd::Shutdown => return,
                    PumpEnd::AuthRejected(message) => {
                        let _ = events.send(NetEvent::AuthRejected { message }).await;
                        return;
                    }
                    PumpEnd::Dropped(reason) => {
                        let retry_in = backoff(attempt + 1);
                        if events
                            .send(NetEvent::Disconnected { reason, retry_in })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        if wait_or_shutdown(retry_in, &mut reconnect_now, &shutdown).await {
                            return;
                        }
                    }
                }
            }
            Err(DialError::AuthRejected(message)) => {
                let _ = events.send(NetEvent::AuthRejected { message }).await;
                return;
            }
            Err(DialError::Transport(reason)) => {
                let retry_in = backoff(attempt);
                if events
                    .send(NetEvent::Disconnected { reason, retry_in })
                    .await
                    .is_err()
                {
                    return;
                }
                if wait_or_shutdown(retry_in, &mut reconnect_now, &shutdown).await {
                    return;
                }
            }
        }
    }
}

/// Sleeps out the backoff unless poked or shut down. Returns `true` on shutdown.
async fn wait_or_shutdown(
    delay: Duration,
    reconnect_now: &mut mpsc::Receiver<()>,
    shutdown: &CancellationToken,
) -> bool {
    tokio::select! {
        () = shutdown.cancelled() => true,
        _ = reconnect_now.recv() => false,
        () = tokio::time::sleep(delay) => false,
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

enum DialError {
    /// HTTP 401 at the upgrade: the token is dead.
    AuthRejected(String),
    /// Anything else; worth retrying.
    Transport(String),
}

async fn dial(ws_url: &str, token: &str) -> Result<Socket, DialError> {
    let mut request = ws_url
        .into_client_request()
        .map_err(|e| DialError::Transport(e.to_string()))?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| DialError::Transport("token contains invalid characters".to_owned()))?,
    );

    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            let status = response.status();
            if status == 401 {
                Err(DialError::AuthRejected(
                    "session expired or revoked".to_owned(),
                ))
            } else {
                Err(DialError::Transport(format!(
                    "server answered HTTP {status}"
                )))
            }
        }
        Err(e) => Err(DialError::Transport(e.to_string())),
    }
}

enum PumpEnd {
    Dropped(String),
    AuthRejected(String),
    Shutdown,
}

/// Runs one live connection until something ends it.
async fn pump(
    mut socket: Socket,
    events: &mpsc::Sender<NetEvent>,
    outgoing: &mut mpsc::Receiver<ClientFrame>,
    shutdown: &CancellationToken,
) -> PumpEnd {
    loop {
        tokio::select! {
            () = shutdown.cancelled() => {
                let _ = socket.close(None).await;
                return PumpEnd::Shutdown;
            }

            frame = outgoing.recv() => {
                let Some(frame) = frame else { return PumpEnd::Shutdown };
                let Ok(text) = frame.encode() else { continue };
                if let Err(e) = socket.send(WsMessage::Text(text.into())).await {
                    return PumpEnd::Dropped(format!("send failed: {e}"));
                }
            }

            inbound = socket.next() => {
                match inbound {
                    Some(Ok(WsMessage::Text(text))) => {
                        match ServerFrame::decode(&text) {
                            Ok(frame) => {
                                // A SessionExpired error is terminal, not a blip.
                                if let ServerFrame::Error { code, message } = &frame {
                                    if code.requires_reauth() {
                                        return PumpEnd::AuthRejected(message.clone());
                                    }
                                }
                                if events.send(NetEvent::Frame(frame)).await.is_err() {
                                    return PumpEnd::Shutdown;
                                }
                            }
                            Err(e) => {
                                // A frame we cannot read is a version-skew
                                // symptom worth logging, not dying over.
                                tracing::warn!(%e, "undecodable frame from server");
                            }
                        }
                    }
                    // tungstenite answers pings internally; binary is not
                    // part of the protocol.
                    Some(Ok(
                        WsMessage::Ping(_)
                        | WsMessage::Pong(_)
                        | WsMessage::Frame(_)
                        | WsMessage::Binary(_),
                    )) => {}
                    Some(Ok(WsMessage::Close(reason))) => {
                        let text = reason.map_or_else(
                            || "server closed the connection".to_owned(),
                            |r| format!("server closed the connection: {}", r.reason),
                        );
                        return PumpEnd::Dropped(text);
                    }
                    Some(Err(e)) => return PumpEnd::Dropped(format!("connection error: {e}")),
                    None => return PumpEnd::Dropped("connection lost".to_owned()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_starts_fast_and_caps() {
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert!(backoff(3) < backoff(5));
        assert_eq!(backoff(6), Duration::from_secs(30));
        assert_eq!(backoff(60), Duration::from_secs(30), "capped forever after");
    }
}
