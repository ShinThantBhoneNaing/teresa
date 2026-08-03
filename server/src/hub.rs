//! In-memory fan-out between live connections.
//!
//! The hub is the reason a slow disk never stalls the chat: services persist a
//! message and then hand it to the hub, which delivers it to every subscribed
//! connection through `tokio::sync::broadcast` channels. Frames travel as
//! `Arc<ServerFrame>`, so a 500-member room serialises a message once, not 500
//! times.
//!
//! Three delivery scopes:
//!
//! - **room** — everyone subscribed to a room's channel ([`Hub::publish_room`])
//! - **user** — every live connection of one account ([`Hub::send_user`]),
//!   used when the target may not be subscribed yet: invites, new DMs
//! - **everyone** — all connections ([`Hub::publish_global`]): presence,
//!   profile changes, room creation, shutdown notices
//!
//! Global presence broadcast is O(connections) per status change, which is the
//! right trade below a few thousand connections; sharding presence by shared
//! room membership is a contained change inside this module if that ceiling is
//! ever reached.

use std::sync::Arc;

use dashmap::DashMap;
use teresa_proto::{ConnectionId, RoomId, ServerFrame, UserId};
use tokio::sync::{broadcast, mpsc};

/// Capacity of each room's broadcast channel.
///
/// A receiver that falls this many frames behind gets `Lagged` and is
/// disconnected by its actor; the alternative is unbounded memory growth on
/// behalf of a stalled client.
const ROOM_CHANNEL_CAPACITY: usize = 512;

/// Capacity of the global broadcast channel. Larger than a room's because
/// every connection listens to it.
const GLOBAL_CHANNEL_CAPACITY: usize = 1024;

/// A frame as it travels between tasks: shared, never copied.
pub type SharedFrame = Arc<ServerFrame>;

/// One live connection's control inbox.
///
/// Targeted pushes land here and pass *through the connection's actor* on the
/// way to the socket, because some of them (a DM opened from the far side, an
/// invite) change what the actor must be subscribed to. Bounded: a connection
/// too stuck to drain it is already being disconnected for lagging.
pub type ConnectionSender = mpsc::Sender<SharedFrame>;

/// The fan-out registry.
#[derive(Debug)]
pub struct Hub {
    /// Per-room broadcast channels, created lazily.
    rooms: DashMap<RoomId, broadcast::Sender<SharedFrame>>,
    /// Direct mailboxes of every live connection, grouped by account.
    connections: DashMap<UserId, Vec<(ConnectionId, ConnectionSender)>>,
    /// The all-connections channel.
    global: broadcast::Sender<SharedFrame>,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    /// An empty hub.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
            connections: DashMap::new(),
            global: broadcast::channel(GLOBAL_CHANNEL_CAPACITY).0,
        }
    }

    // --- room scope ---------------------------------------------------------

    /// Subscribes to a room's frames.
    ///
    /// The channel is created on first use; there is nothing special about a
    /// room nobody is listening to.
    #[must_use]
    pub fn subscribe_room(&self, room: RoomId) -> broadcast::Receiver<SharedFrame> {
        self.rooms
            .entry(room)
            .or_insert_with(|| broadcast::channel(ROOM_CHANNEL_CAPACITY).0)
            .subscribe()
    }

    /// Delivers a frame to every subscriber of a room.
    ///
    /// Returns how many receivers it reached. Zero is normal — an empty room
    /// still accepts messages; they simply go straight to the database.
    pub fn publish_room(&self, room: RoomId, frame: ServerFrame) -> usize {
        match self.rooms.get(&room) {
            Some(sender) => sender.send(Arc::new(frame)).unwrap_or(0),
            None => 0,
        }
    }

    /// Forgets a deleted room's channel.
    ///
    /// Live receivers see the channel close and drop their subscription.
    pub fn drop_room(&self, room: RoomId) {
        self.rooms.remove(&room);
    }

    // --- user scope ---------------------------------------------------------

    /// Registers a connection's mailbox. Returns how many connections the
    /// account now has, the caller being responsible for enforcing its cap.
    pub fn register(&self, user: UserId, conn: ConnectionId, sender: ConnectionSender) -> usize {
        let mut entry = self.connections.entry(user).or_default();
        entry.push((conn, sender));
        entry.len()
    }

    /// Removes a connection. Returns `true` if it was the account's last —
    /// the moment the user goes offline.
    pub fn unregister(&self, user: UserId, conn: ConnectionId) -> bool {
        let Some(mut entry) = self.connections.get_mut(&user) else {
            return false;
        };
        entry.retain(|(id, _)| *id != conn);
        let now_offline = entry.is_empty();
        drop(entry);
        if now_offline {
            // remove_if re-checks under the write lock: a connection that
            // registered between our check and now must not be swept away.
            self.connections
                .remove_if(&user, |_, conns| conns.is_empty());
        }
        now_offline
    }

    /// Whether an account has any live connection.
    #[must_use]
    pub fn is_live(&self, user: UserId) -> bool {
        self.connections.get(&user).is_some_and(|c| !c.is_empty())
    }

    /// Sends a frame directly to every connection of one account.
    ///
    /// `try_send` by design: a mailbox full enough to reject a direct frame
    /// belongs to a connection that is already being disconnected for lagging,
    /// and this call must never block a service on one bad socket.
    pub fn send_user(&self, user: UserId, frame: ServerFrame) -> usize {
        let Some(entry) = self.connections.get(&user) else {
            return 0;
        };
        let frame = Arc::new(frame);
        entry
            .iter()
            .filter(|(_, sender)| sender.try_send(Arc::clone(&frame)).is_ok())
            .count()
    }

    // --- global scope -------------------------------------------------------

    /// Subscribes to the all-connections channel.
    #[must_use]
    pub fn subscribe_global(&self) -> broadcast::Receiver<SharedFrame> {
        self.global.subscribe()
    }

    /// Delivers a frame to every connection.
    pub fn publish_global(&self, frame: ServerFrame) -> usize {
        self.global.send(Arc::new(frame)).unwrap_or(0)
    }

    // --- bookkeeping --------------------------------------------------------

    /// Total live connections.
    #[must_use]
    pub fn connection_count(&self) -> usize {
        self.connections.iter().map(|entry| entry.len()).sum()
    }

    /// Distinct accounts with at least one live connection.
    #[must_use]
    pub fn live_user_count(&self) -> usize {
        self.connections.len()
    }

    /// Discards room channels nobody is subscribed to.
    ///
    /// Called from the housekeeping task. Publishing recreates a channel on
    /// demand, so this is invisible to correctness.
    pub fn prune(&self) {
        self.rooms.retain(|_, sender| sender.receiver_count() > 0);
    }

    /// The number of room channels currently held.
    #[must_use]
    pub fn room_channel_count(&self) -> usize {
        self.rooms.len()
    }
}

#[cfg(test)]
mod tests {
    use teresa_proto::ErrorCode;

    use super::*;

    fn frame(text: &str) -> ServerFrame {
        ServerFrame::Notice {
            text: text.to_owned(),
        }
    }

    fn text_of(frame: &SharedFrame) -> &str {
        match frame.as_ref() {
            ServerFrame::Notice { text } => text,
            other => panic!("expected a notice, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn room_frames_reach_every_subscriber() {
        let hub = Hub::new();
        let room = RoomId::new();
        let mut a = hub.subscribe_room(room);
        let mut b = hub.subscribe_room(room);

        assert_eq!(hub.publish_room(room, frame("ship it")), 2);
        assert_eq!(text_of(&a.recv().await.unwrap()), "ship it");
        assert_eq!(text_of(&b.recv().await.unwrap()), "ship it");
    }

    #[tokio::test]
    async fn rooms_are_isolated_from_each_other() {
        let hub = Hub::new();
        let ours = RoomId::new();
        let theirs = RoomId::new();
        let mut ours_rx = hub.subscribe_room(ours);

        hub.publish_room(theirs, frame("secret"));
        hub.publish_room(ours, frame("public"));

        assert_eq!(text_of(&ours_rx.recv().await.unwrap()), "public");
        assert!(
            ours_rx.try_recv().is_err(),
            "the other room's frame must not arrive"
        );
    }

    #[tokio::test]
    async fn publishing_to_an_empty_room_is_fine() {
        let hub = Hub::new();
        assert_eq!(hub.publish_room(RoomId::new(), frame("into the void")), 0);
    }

    #[tokio::test]
    async fn direct_send_reaches_all_of_a_users_terminals() {
        let hub = Hub::new();
        let user = UserId::new();
        let (tx1, mut rx1) = mpsc::channel(8);
        let (tx2, mut rx2) = mpsc::channel(8);

        assert_eq!(hub.register(user, ConnectionId::new(), tx1), 1);
        assert_eq!(hub.register(user, ConnectionId::new(), tx2), 2);

        assert_eq!(hub.send_user(user, frame("psst")), 2);
        assert_eq!(text_of(&rx1.recv().await.unwrap()), "psst");
        assert_eq!(text_of(&rx2.recv().await.unwrap()), "psst");

        // A user with no connections is silently skipped.
        assert_eq!(hub.send_user(UserId::new(), frame("nobody home")), 0);
    }

    #[tokio::test]
    async fn unregister_reports_the_offline_transition_exactly_once() {
        let hub = Hub::new();
        let user = UserId::new();
        let first = ConnectionId::new();
        let second = ConnectionId::new();
        let (tx, _rx) = mpsc::channel(8);

        hub.register(user, first, tx.clone());
        hub.register(user, second, tx);
        assert!(hub.is_live(user));

        assert!(!hub.unregister(user, first), "one terminal remains");
        assert!(hub.unregister(user, second), "last terminal closed");
        assert!(!hub.is_live(user));
        assert!(
            !hub.unregister(user, second),
            "double unregister is not a second transition"
        );
    }

    #[tokio::test]
    async fn global_frames_reach_everyone() {
        let hub = Hub::new();
        let mut a = hub.subscribe_global();
        let mut b = hub.subscribe_global();

        hub.publish_global(ServerFrame::error(ErrorCode::Internal, "restarting"));
        assert!(matches!(
            a.recv().await.unwrap().as_ref(),
            ServerFrame::Error { .. }
        ));
        assert!(matches!(
            b.recv().await.unwrap().as_ref(),
            ServerFrame::Error { .. }
        ));
    }

    #[tokio::test]
    async fn a_full_mailbox_does_not_block_the_sender() {
        let hub = Hub::new();
        let user = UserId::new();
        let (tx, _rx) = mpsc::channel(1);
        hub.register(user, ConnectionId::new(), tx);

        assert_eq!(hub.send_user(user, frame("first")), 1);
        // The mailbox is now full; the send is dropped, not blocked on.
        assert_eq!(hub.send_user(user, frame("second")), 0);
    }

    #[tokio::test]
    async fn prune_discards_only_subscriberless_channels() {
        let hub = Hub::new();
        let kept = RoomId::new();
        let dropped = RoomId::new();

        let _keeper = hub.subscribe_room(kept);
        drop(hub.subscribe_room(dropped));
        assert_eq!(hub.room_channel_count(), 2);

        hub.prune();
        assert_eq!(hub.room_channel_count(), 1);

        // A pruned room springs back on demand.
        let mut back = hub.subscribe_room(dropped);
        hub.publish_room(dropped, frame("hello again"));
        assert_eq!(text_of(&back.recv().await.unwrap()), "hello again");
    }

    #[tokio::test]
    async fn counts_reflect_reality() {
        let hub = Hub::new();
        let alice = UserId::new();
        let bob = UserId::new();
        let (tx, _rx) = mpsc::channel(8);

        hub.register(alice, ConnectionId::new(), tx.clone());
        hub.register(alice, ConnectionId::new(), tx.clone());
        hub.register(bob, ConnectionId::new(), tx);

        assert_eq!(hub.connection_count(), 3);
        assert_eq!(hub.live_user_count(), 2);
    }
}
