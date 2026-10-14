//! Application state.
//!
//! [`App`] is everything the renderer may draw and the only thing `update`
//! may mutate. It holds no channels, no sockets and no terminal handles —
//! effects the outside world must perform are queued in [`App::effects`] and
//! drained by the main loop. That boundary is what makes the whole UI
//! state-machine testable without a terminal or a server.

pub mod commands;
pub mod event;
pub mod history;
pub mod update;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use teresa_proto::{
    ClientFrame, FileCard, FileId, JoinedRoom, Message, MessageId, Room, RoomId, ServerInfo, User,
    UserId,
};

use crate::config::ClientConfig;
use crate::net::transfer::Stage;
use crate::ui::theme::Theme;
use history::InputLine;

/// How many messages a room keeps in memory before the oldest are shed.
/// Scrolling past this triggers a history fetch anyway.
const SCROLLBACK_CAP: usize = 2000;

/// Where the connection currently stands, as shown in the header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    /// Dialling (attempt n).
    Connecting {
        /// 1-based attempt counter.
        attempt: u32,
    },
    /// Socket open, `Ready` received; timestamp for the uptime display.
    Online {
        /// When the session became ready.
        since: Instant,
    },
    /// Waiting out a backoff.
    Offline {
        /// Why the connection dropped.
        reason: String,
        /// When the next dial happens.
        retry_at: Instant,
    },
    /// The session is dead; only a fresh login helps. Terminal state.
    AuthFailed {
        /// The server's explanation.
        message: String,
    },
}

/// One room's client-side state.
#[derive(Debug)]
pub struct RoomState {
    /// The room and this user's relationship to it.
    pub info: JoinedRoom,
    /// Scrollback, oldest first.
    pub messages: Vec<Message>,
    /// Ids already in `messages` — the de-duplication set that makes the
    /// Posted/Message double-delivery harmless.
    pub seen: HashSet<MessageId>,
    /// Current member list (only populated once visited).
    pub members: Vec<User>,
    /// Who is typing right now: user, display name, last signal.
    pub typing: Vec<(UserId, String, Instant)>,
    /// Whether the top of history has been reached.
    pub history_complete: bool,
    /// A history request is in flight.
    pub fetching_history: bool,
    /// The in-flight request is for an *older* page (scrollback), not the tail.
    pub fetching_older: bool,
    /// Lines scrolled up from the live tail; 0 = following.
    pub scroll: u16,
    /// Local echoes awaiting their `Posted` ack, keyed by nonce.
    pub pending: HashMap<String, PendingSend>,
    /// Unread mentions (subset of `info.unread`).
    pub mentions: u32,
    /// Live action hits (`/shoot` and friends): the glyph flashing next to a
    /// member's name until it expires.
    pub fx: Vec<Fx>,
}

/// One action landing on one member, briefly.
#[derive(Debug)]
pub struct Fx {
    /// Who got hit.
    pub user: UserId,
    /// The glyph to flash next to their name.
    pub glyph: &'static str,
    /// When it landed; drives the blink and the expiry.
    pub at: Instant,
}

/// A message optimistically shown before the server confirms it.
#[derive(Debug)]
pub struct PendingSend {
    /// The body as typed.
    pub body: String,
    /// Emote or chat.
    pub emote: bool,
    /// When it was sent, for a "still sending…" hint on slow links.
    pub at: Instant,
}

impl RoomState {
    fn new(info: JoinedRoom) -> Self {
        Self {
            info,
            messages: Vec::new(),
            seen: HashSet::new(),
            members: Vec::new(),
            typing: Vec::new(),
            history_complete: false,
            fetching_history: false,
            fetching_older: false,
            scroll: 0,
            pending: HashMap::new(),
            mentions: 0,
            fx: Vec::new(),
        }
    }

    /// Appends a message if it is new; keeps scrollback bounded.
    /// Returns whether it was actually added.
    fn push_message(&mut self, message: Message) -> bool {
        if !self.seen.insert(message.id) {
            return false;
        }
        self.messages.push(message);
        if self.messages.len() > SCROLLBACK_CAP {
            let dropped = self.messages.remove(0);
            self.seen.remove(&dropped.id);
            // The cap means the top of local history is no longer the top of
            // room history.
            self.history_complete = false;
        }
        true
    }

    /// Merges an older history page below the current scrollback.
    fn prepend_history(&mut self, page: Vec<Message>) {
        let fresh: Vec<Message> = page
            .into_iter()
            .filter(|m| self.seen.insert(m.id))
            .collect();
        // Page and scrollback are both ascending; the page is strictly older.
        self.messages.splice(0..0, fresh);
    }

    /// The label for tabs and lists: `#room` or `@peer`.
    #[must_use]
    pub fn label(&self) -> String {
        self.info.label()
    }
}

/// One file transfer as the UI tracks it.
#[derive(Debug)]
pub struct Transfer {
    /// Assigned by the reducer; echoed in every update.
    pub id: u64,
    /// Up (sharing) or down (fetching).
    pub upload: bool,
    /// The name shown in the dock.
    pub name: String,
    /// Where it is going or coming from: `#room` or `@peer`.
    pub place: String,
    /// For downloads (and finished uploads): the file on the server.
    pub file: Option<FileId>,
    /// Latest reported stage.
    pub stage: Stage,
    /// When the transfer was requested.
    pub started: Instant,
    /// When the current stage began — the clock throughput is measured on.
    pub stage_started: Instant,
    /// When it finished or failed; drives how long it lingers in the dock.
    pub ended: Option<Instant>,
    /// Bytes moved over the wire and how long that took, once known — the
    /// numbers the finished row reports.
    pub wire: Option<(u64, std::time::Duration)>,
}

impl Transfer {
    /// Whether the task is still running.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.ended.is_none()
    }
}

/// A `/send @user` waiting for its DM room to exist.
#[derive(Debug)]
pub struct PendingDmSend {
    /// The recipient's handle.
    pub peer: String,
    /// What to send.
    pub path: PathBuf,
    /// Force a zip.
    pub zip: bool,
    /// When it was asked for; abandoned if no room turns up.
    pub at: Instant,
}

/// A transient popup in the corner of the screen.
#[derive(Debug)]
pub struct Toast {
    /// What it says.
    pub text: String,
    /// Colour class.
    pub kind: ToastKind,
    /// When it disappears.
    pub until: Instant,
}

/// What flavour of toast to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    /// Neutral information.
    Info,
    /// Something good (invite, connect).
    Success,
    /// Something to notice (mention, DM while elsewhere).
    Mention,
    /// Something went wrong.
    Error,
}

/// Which modal overlay is open, if any.
#[derive(Debug, PartialEq, Eq)]
pub enum Overlay {
    /// `/help` or F1: command and keybinding reference.
    Help,
    /// Ctrl+P: own profile.
    Profile,
    /// Ctrl+N: the notification log.
    Notifications,
    /// Ctrl+W: room switcher with a live filter.
    RoomSwitcher {
        /// What has been typed into the filter.
        filter: String,
        /// Which visible entry is highlighted.
        selected: usize,
    },
    /// Session death notice with instructions.
    AuthFailed {
        /// The server's explanation.
        message: String,
    },
    /// Quit confirmation (Ctrl+C).
    ConfirmQuit,
    /// `/get`: where should this file land?
    SaveAs {
        /// The file to fetch.
        card: FileCard,
        /// Who shared it, for the prompt.
        from: String,
        /// The folder being typed, prefilled with the default.
        path: String,
    },
}

/// A side effect the main loop must perform for the reducer.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    /// Send a frame to the server.
    Send(ClientFrame),
    /// Persist the config file (theme changed).
    PersistConfig,
    /// Log out: revoke server-side, delete the session file, then quit.
    Logout,
    /// Ask the supervisor to redial immediately.
    ReconnectNow,
    /// Ring the terminal bell.
    Bell,
    /// Start uploading a file or folder into a room.
    Upload {
        /// Transfer id for progress updates.
        id: u64,
        /// Destination room.
        room: RoomId,
        /// What to send.
        path: PathBuf,
        /// Zip even a single file.
        zip: bool,
    },
    /// Start downloading a shared file.
    Download {
        /// Transfer id for progress updates.
        id: u64,
        /// What to fetch.
        card: FileCard,
        /// The folder to save it in; created if missing.
        dest: PathBuf,
    },
}

/// The whole application.
pub struct App {
    /// Client configuration (theme name is kept in sync with `theme`).
    pub config: ClientConfig,
    /// The active colour scheme.
    pub theme: Theme,
    /// Connection state for the header.
    pub conn: ConnState,
    /// Who we are; `None` until the first `Ready` (the session file's names
    /// are used for the boot screen meanwhile).
    pub me: Option<User>,
    /// Server identity from `Ready`.
    pub server: Option<ServerInfo>,
    /// Rooms in display order.
    pub room_order: Vec<RoomId>,
    /// Room state by id.
    pub rooms: HashMap<RoomId, RoomState>,
    /// The room on screen.
    pub active: Option<RoomId>,
    /// Channels known to exist (from `/rooms`), joined or not.
    pub discovered: Vec<Room>,
    /// The input line.
    pub input: InputLine,
    /// The open overlay, if any.
    pub overlay: Option<Overlay>,
    /// Corner popups.
    pub toasts: VecDeque<Toast>,
    /// The persistent notification log (Ctrl+N).
    pub notice_log: VecDeque<(Instant, String)>,
    /// When the app started, for the boot animation.
    pub started: Instant,
    /// Latest measured round trip, for the status bar.
    pub rtt_ms: Option<u64>,
    /// Outstanding `/ping` nonces.
    pub pending_pings: HashMap<u64, Instant>,
    /// Last time we told the server we were typing, per the active room.
    pub typing_sent: Option<Instant>,
    /// Set when the user issues /join, /create or /dm; a `Joined` frame inside
    /// the window switches the screen, one outside it stays in the background.
    pub pending_activate: Option<Instant>,
    /// Monotonic source for send nonces.
    pub next_nonce: u64,
    /// True while there is activity the user has not looked at (title flash).
    pub unseen_activity: bool,
    /// File transfers, running and recently finished, oldest first.
    pub transfers: Vec<Transfer>,
    /// Monotonic source for transfer ids.
    pub next_transfer: u64,
    /// `/send @user` requests waiting for their DM room.
    pub pending_dm_sends: Vec<PendingDmSend>,
    /// Effects for the main loop, drained every iteration.
    pub effects: Vec<Effect>,
    /// The renderer should redraw.
    pub dirty: bool,
    /// The main loop should exit.
    pub quit: bool,
}

impl App {
    /// A fresh application in its boot state.
    #[must_use]
    pub fn new(config: ClientConfig) -> Self {
        let theme = Theme::by_name(&config.theme);
        Self {
            config,
            theme,
            conn: ConnState::Connecting { attempt: 1 },
            me: None,
            server: None,
            room_order: Vec::new(),
            rooms: HashMap::new(),
            active: None,
            discovered: Vec::new(),
            input: InputLine::default(),
            overlay: None,
            toasts: VecDeque::new(),
            notice_log: VecDeque::new(),
            started: Instant::now(),
            rtt_ms: None,
            pending_pings: HashMap::new(),
            typing_sent: None,
            pending_activate: None,
            next_nonce: 1,
            unseen_activity: false,
            transfers: Vec::new(),
            next_transfer: 1,
            pending_dm_sends: Vec::new(),
            effects: Vec::new(),
            dirty: true,
            quit: false,
        }
    }

    /// The active room's state.
    #[must_use]
    pub fn active_room(&self) -> Option<&RoomState> {
        self.rooms.get(self.active.as_ref()?)
    }

    /// The active room's state, mutably.
    pub fn active_room_mut(&mut self) -> Option<&mut RoomState> {
        self.rooms.get_mut(self.active.as_ref()?)
    }

    /// Adds (or refreshes) a room, keeping display order stable.
    pub(crate) fn upsert_room(&mut self, info: JoinedRoom) -> RoomId {
        let id = info.room.id;
        if let Some(existing) = self.rooms.get_mut(&id) {
            existing.info = info;
        } else {
            self.rooms.insert(id, RoomState::new(info));
            self.room_order.push(id);
        }
        id
    }

    /// Removes a room everywhere it is referenced.
    pub(crate) fn remove_room(&mut self, id: RoomId) {
        self.rooms.remove(&id);
        self.room_order.retain(|r| *r != id);
        if self.active == Some(id) {
            self.active = self.room_order.first().copied();
        }
    }

    /// Queues a frame for the server.
    pub(crate) fn send(&mut self, frame: ClientFrame) {
        self.effects.push(Effect::Send(frame));
    }

    /// Shows a toast and records it in the notification log.
    pub(crate) fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        self.notice_log.push_front((Instant::now(), text.clone()));
        self.notice_log.truncate(100);
        self.toasts.push_back(Toast {
            text,
            kind,
            until: Instant::now() + std::time::Duration::from_secs(4),
        });
        while self.toasts.len() > 3 {
            self.toasts.pop_front();
        }
        self.dirty = true;
    }

    /// A system-style line shown only locally in the active room.
    pub(crate) fn local_note(&mut self, text: impl Into<String>) {
        if let Some(room_id) = self.active {
            if let Some(room) = self.rooms.get_mut(&room_id) {
                let note = Message::system(room_id, text.into());
                room.push_message(note);
            }
        }
        self.dirty = true;
    }

    /// Fresh nonce for a send.
    pub(crate) fn nonce(&mut self) -> String {
        let value = self.next_nonce;
        self.next_nonce += 1;
        format!("n{value:x}")
    }

    /// File cards in the active room, newest first — the numbering `/get`
    /// and `/files` share with the `#n` on each card.
    #[must_use]
    pub fn file_cards(&self) -> Vec<(&Message, FileCard)> {
        self.active_room()
            .map(|room| {
                room.messages
                    .iter()
                    .rev()
                    .filter(|m| m.kind == teresa_proto::MessageKind::File)
                    .filter_map(|m| FileCard::from_body(&m.body).ok().map(|card| (m, card)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The newest transfer touching a given server file.
    #[must_use]
    pub fn transfer_for(&self, file: FileId) -> Option<&Transfer> {
        self.transfers.iter().rev().find(|t| t.file == Some(file))
    }

    /// All usernames known in the active room, for Tab completion.
    #[must_use]
    pub fn completion_candidates(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .active_room()
            .map(|room| room.members.iter().map(|m| m.username.clone()).collect())
            .unwrap_or_default();

        // At the start of a line, commands complete too.
        if self.input.text().starts_with('/') && !self.input.text().contains(' ') {
            names = commands::COMMAND_NAMES
                .iter()
                .map(|c| format!("/{c}"))
                .collect();
        }
        names
    }
}
