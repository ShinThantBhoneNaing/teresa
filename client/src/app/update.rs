//! The reducer: the only code that mutates [`App`].
//!
//! `update(app, event)` is a pure-ish state transition — its only outputs are
//! mutations on `app`, including the [`Effect`] queue the main loop performs.
//! Every branch here is reachable from a unit test with no terminal and no
//! server attached.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use teresa_proto::file::human_bytes;
use teresa_proto::{
    ClientFrame, FileCard, MaintenanceOp, MaintenanceReport, MessageKind, RoomId, RoomKind,
    ServerFrame, HISTORY_PAGE_DEFAULT,
};

use crate::net::transfer::{self, Outcome, Stage, TransferUpdate};
use crate::net::ws::NetEvent;
use crate::ui::theme::Theme;

use super::commands::{parse, Command};
use super::event::Event;
use super::{App, ConnState, Effect, Overlay, ToastKind};

/// Re-announce "still typing" no more often than this.
const TYPING_THROTTLE: Duration = Duration::from_secs(2);
/// A typing indicator with no refresh for this long is stale.
const TYPING_EXPIRY: Duration = Duration::from_secs(4);
/// Window in which an incoming `Joined` counts as the answer to our own
/// `/join`, `/create` or `/dm` (and therefore switches the screen to it).
const ACTIVATE_WINDOW: Duration = Duration::from_secs(5);
/// How long an action hit (`/shoot` and friends) flashes on the target's name.
pub(crate) const FX_DURATION: Duration = Duration::from_secs(4);
/// How long a finished transfer stays in the dock.
const TRANSFER_LINGER: Duration = Duration::from_secs(8);
/// How long a failed one does — long enough to read why.
const TRANSFER_FAIL_LINGER: Duration = Duration::from_secs(15);
/// A `/send @user` whose DM has not opened within this is abandoned.
const DM_SEND_WINDOW: Duration = Duration::from_secs(10);

/// Applies one event.
pub fn update(app: &mut App, event: Event) {
    match event {
        Event::Net(net) => on_net(app, net),
        Event::Key(key) => on_key(app, key),
        Event::Paste(text) => on_paste(app, &text),
        Event::Mouse(mouse) => on_mouse(app, mouse),
        Event::Resize => app.dirty = true,
        Event::Tick => on_tick(app),
        Event::Transfer(update) => on_transfer(app, update),
    }
}

// --- network ---------------------------------------------------------------

fn on_net(app: &mut App, event: NetEvent) {
    app.dirty = true;
    match event {
        NetEvent::Connecting { attempt } => {
            app.conn = ConnState::Connecting { attempt };
        }
        NetEvent::Connected => {
            // Ready follows momentarily; the header can already say "linked".
        }
        NetEvent::Disconnected { reason, retry_in } => {
            app.conn = ConnState::Offline {
                reason,
                retry_at: Instant::now() + retry_in,
            };
        }
        NetEvent::AuthRejected { message } => {
            app.conn = ConnState::AuthFailed {
                message: message.clone(),
            };
            app.overlay = Some(Overlay::AuthFailed { message });
        }
        NetEvent::Frame(frame) => on_frame(app, frame),
    }
}

#[allow(clippy::too_many_lines)] // one arm per protocol frame; the width is the protocol's
fn on_frame(app: &mut App, frame: ServerFrame) {
    app.dirty = true;
    match frame {
        ServerFrame::Ready {
            user,
            rooms,
            server,
        } => {
            app.conn = ConnState::Online {
                since: Instant::now(),
            };
            app.me = Some(user);
            app.server = Some(server);

            // Rooms the server no longer lists were deleted while we were away.
            let listed: std::collections::HashSet<_> = rooms.iter().map(|r| r.room.id).collect();
            let stale: Vec<_> = app
                .room_order
                .iter()
                .copied()
                .filter(|id| !listed.contains(id))
                .collect();
            for id in stale {
                app.remove_room(id);
            }
            for joined in rooms {
                app.upsert_room(joined);
            }

            let target = app
                .active
                .filter(|id| app.rooms.contains_key(id))
                .or_else(|| app.room_order.first().copied());
            if let Some(id) = target {
                activate_room(app, id);
            }

            if let Some(motd) = app.server.as_ref().and_then(|s| s.motd.clone()) {
                app.toast(ToastKind::Info, motd);
            }
            app.toast(ToastKind::Success, "uplink established");
        }

        ServerFrame::Message { message } => on_message(app, message),

        ServerFrame::Posted { message, nonce } => {
            let action = action_of(&message);
            if let Some(room) = app.rooms.get_mut(&message.room_id) {
                room.pending.remove(&nonce);
                if room.push_message(message) {
                    apply_fx(room, action);
                }
            }
        }

        ServerFrame::History {
            room,
            messages,
            complete,
        } => {
            if let Some(state) = app.rooms.get_mut(&room) {
                if state.fetching_older {
                    state.prepend_history(messages);
                    if complete {
                        state.history_complete = true;
                    }
                } else {
                    for message in messages {
                        state.push_message(message);
                    }
                    if complete {
                        state.history_complete = true;
                    }
                }
                state.fetching_history = false;
                state.fetching_older = false;
            }
        }

        ServerFrame::RoomList { rooms } => {
            app.discovered = rooms;
            app.toast(
                ToastKind::Info,
                format!(
                    "{} rooms on this server — Ctrl+W to browse",
                    app.discovered.len()
                ),
            );
        }

        ServerFrame::RoomCreated { room } => {
            if !app.discovered.iter().any(|r| r.id == room.id) {
                app.notice_log
                    .push_front((Instant::now(), format!("room #{} appeared", room.name)));
                app.discovered.push(room);
            }
        }

        ServerFrame::RoomDeleted { room } => {
            app.discovered.retain(|r| r.id != room);
            if let Some(state) = app.rooms.get(&room) {
                let label = state.label();
                app.toast(ToastKind::Info, format!("{label} was deleted"));
                app.remove_room(room);
            }
        }

        ServerFrame::Joined { room, members } => {
            let peer = room.peer.clone();
            let id = app.upsert_room(room);
            if let Some(state) = app.rooms.get_mut(&id) {
                state.members = members;
            }
            // A `/send @peer` was waiting for exactly this room.
            if let Some(peer) = peer {
                let (ready, waiting) = std::mem::take(&mut app.pending_dm_sends)
                    .into_iter()
                    .partition(|send| send.peer == peer);
                app.pending_dm_sends = waiting;
                for send in ready {
                    start_upload(app, id, send.path, send.zip);
                }
            }
            // Only a Joined we asked for steals the screen; one pushed by the
            // far side of a new DM must not yank the user mid-sentence.
            let ours = app
                .pending_activate
                .take()
                .is_some_and(|at| at.elapsed() < ACTIVATE_WINDOW);
            if ours {
                activate_room(app, id);
            } else if let Some(state) = app.rooms.get(&id) {
                let label = state.label();
                app.toast(
                    ToastKind::Mention,
                    format!("{label} opened — Ctrl+W to switch"),
                );
                if app.config.notifications && app.config.bell {
                    app.effects.push(Effect::Bell);
                }
            }
        }

        ServerFrame::Left { room } => app.remove_room(room),

        ServerFrame::MemberJoined { room, user } => {
            if let Some(state) = app.rooms.get_mut(&room) {
                if !state.members.iter().any(|m| m.id == user.id) {
                    state.members.push(user);
                    state.members.sort_by(|a, b| a.username.cmp(&b.username));
                }
                state.info.room.member_count = state.members.len() as u32;
            }
        }

        ServerFrame::MemberLeft { room, user } => {
            if let Some(state) = app.rooms.get_mut(&room) {
                state.members.retain(|m| m.id != user);
                state.info.room.member_count = state.members.len() as u32;
            }
        }

        ServerFrame::Members { room, members } => {
            if let Some(state) = app.rooms.get_mut(&room) {
                state.members = members;
                state.info.room.member_count = state.members.len() as u32;
            }
        }

        ServerFrame::Presence { user, status, text } => {
            for state in app.rooms.values_mut() {
                if let Some(member) = state.members.iter_mut().find(|m| m.id == user) {
                    member.status = status;
                    member.status_text.clone_from(&text);
                }
            }
            if let Some(me) = app.me.as_mut().filter(|me| me.id == user) {
                me.status = status;
                me.status_text = text;
            }
        }

        ServerFrame::Typing {
            room,
            user,
            display_name,
            active,
        } => {
            if !app.config.typing_indicators {
                return;
            }
            if let Some(state) = app.rooms.get_mut(&room) {
                state.typing.retain(|(id, _, _)| *id != user);
                if active {
                    state.typing.push((user, display_name, Instant::now()));
                }
            }
        }

        ServerFrame::ProfileUpdated { user } => {
            for state in app.rooms.values_mut() {
                if let Some(member) = state.members.iter_mut().find(|m| m.id == user.id) {
                    *member = user.clone();
                }
            }
            if app.me.as_ref().is_some_and(|me| me.id == user.id) {
                app.me = Some(user);
            }
        }

        ServerFrame::Invited { room, by } => {
            let label = format!("#{}", room.name);
            app.toast(
                ToastKind::Mention,
                format!("@{by} invited you to {label} — /join {}", room.name),
            );
            if !app.discovered.iter().any(|r| r.id == room.id) {
                app.discovered.push(room);
            }
            if app.config.notifications && app.config.bell {
                app.effects.push(Effect::Bell);
            }
            app.unseen_activity = true;
        }

        ServerFrame::Notice { text } => app.toast(ToastKind::Info, text),

        ServerFrame::Error { code, message } => {
            app.toast(ToastKind::Error, format!("{code}: {message}"));
        }

        ServerFrame::Maintenance { report } => on_maintenance(app, &report),

        ServerFrame::Pong { nonce } => {
            if let Some(sent) = app.pending_pings.remove(&nonce) {
                let rtt = sent.elapsed().as_millis() as u64;
                app.rtt_ms = Some(rtt);
                app.local_note(format!("pong — {rtt}ms round trip"));
            }
        }
    }
}

fn on_message(app: &mut App, message: teresa_proto::Message) {
    let my_name = app
        .me
        .as_ref()
        .map(|me| me.username.clone())
        .unwrap_or_default();
    let my_id = app.me.as_ref().map(|me| me.id);
    let room_id = message.room_id;
    let is_active = app.active == Some(room_id);

    let Some(state) = app.rooms.get_mut(&room_id) else {
        return; // a room we do not hold (already left)
    };

    // The author has plainly stopped typing.
    if let Some(author) = &message.author {
        state.typing.retain(|(id, _, _)| *id != author.id);
    }

    // "Own message" must be a positive match: if we do not know who we are
    // yet, nothing is ours — never the other way around.
    let own = match (message.author.as_ref(), my_id) {
        (Some(author), Some(me)) => author.id == me,
        _ => false,
    };
    let notable = message.kind.is_notable();
    let mentioned = !own && message.mentions(&my_name);
    let is_dm = state.info.room.kind == RoomKind::Dm;
    let author_label = message.author_label().to_owned();
    let preview = if message.kind == MessageKind::File {
        FileCard::from_body(&message.body).map_or_else(
            |_| "shared a file".to_owned(),
            |card| format!("⇪ {} ({})", card.name, human_bytes(card.size)),
        )
    } else {
        preview_of(&message.body)
    };
    let label = state.label();
    let at_tail = state.scroll == 0;
    let action = action_of(&message);
    let hit_me = action
        .as_ref()
        .is_some_and(|(_, target)| *target == my_name);
    let hit_glyph = action.as_ref().map(|(glyph, _)| *glyph);

    if !state.push_message(message) {
        return; // duplicate (our own echo after Posted)
    }
    apply_fx(state, action);

    if own {
        return; // no self-notification of any kind
    }

    // Getting shot in the room you are watching still deserves a flash of
    // theatre; background rooms are covered by the mention path below.
    if hit_me && is_active && app.config.notifications {
        let glyph = hit_glyph.unwrap_or_default();
        app.toast(
            ToastKind::Mention,
            format!("{glyph} {author_label} got you"),
        );
        if app.config.bell {
            app.effects.push(Effect::Bell);
        }
    }

    if is_active && at_tail {
        // Watched live: tell the server it is read.
        app.send(ClientFrame::MarkRead { room: room_id });
    } else if notable {
        let state = app.rooms.get_mut(&room_id).expect("checked above");
        state.info.unread += 1;
        if mentioned {
            state.mentions += 1;
        }
        app.unseen_activity = true;

        let should_pop = mentioned || is_dm;
        if should_pop && app.config.notifications {
            app.toast(
                ToastKind::Mention,
                format!("{label} {author_label}: {preview}"),
            );
            if app.config.bell {
                app.effects.push(Effect::Bell);
            }
        }
    }
}

/// The hit effect an incoming message carries, if it is a targeted action.
fn action_of(message: &teresa_proto::Message) -> Option<(&'static str, String)> {
    (message.kind == MessageKind::Emote)
        .then(|| super::commands::action_fx(&message.body))
        .flatten()
}

/// Lands an action's glyph on the target member, if they are in the room.
fn apply_fx(room: &mut super::RoomState, action: Option<(&'static str, String)>) {
    let Some((glyph, target)) = action else {
        return;
    };
    if let Some(user) = room
        .members
        .iter()
        .find(|m| m.username == target)
        .map(|m| m.id)
    {
        room.fx.push(super::Fx {
            user,
            glyph,
            at: Instant::now(),
        });
    }
}

/// First line, clipped, for toast previews.
fn preview_of(body: &str) -> String {
    let first = body.lines().next().unwrap_or_default();
    let mut clipped: String = first.chars().take(48).collect();
    if clipped.len() < first.len() {
        clipped.push('…');
    }
    clipped
}

// --- keys ------------------------------------------------------------------

fn on_key(app: &mut App, key: KeyEvent) {
    app.dirty = true;

    // Overlays swallow keys first.
    if app.overlay.is_some() {
        on_overlay_key(app, key);
        return;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match (key.code, ctrl, alt) {
        // -- application chrome --
        (KeyCode::Char('c'), true, _) => app.overlay = Some(Overlay::ConfirmQuit),
        (KeyCode::Char('k'), true, _) => clear_active_room(app),
        (KeyCode::Char('l'), true, _) => app.dirty = true, // full redraw happens anyway
        (KeyCode::Char('r'), true, _) => {
            app.effects.push(Effect::ReconnectNow);
            app.toast(ToastKind::Info, "re-establishing uplink…");
        }
        (KeyCode::Char('p'), true, _) => app.overlay = Some(Overlay::Profile),
        (KeyCode::Char('n'), true, _) => {
            app.unseen_activity = false;
            app.overlay = Some(Overlay::Notifications);
        }
        (KeyCode::Char('w'), true, _) => {
            app.overlay = Some(Overlay::RoomSwitcher {
                filter: String::new(),
                selected: 0,
            });
        }
        (KeyCode::F(1), _, _) => app.overlay = Some(Overlay::Help),

        // -- room navigation --
        (KeyCode::Up, _, true) => cycle_room(app, -1),
        (KeyCode::Down, _, true) => cycle_room(app, 1),
        (KeyCode::Char(c @ '1'..='9'), _, true) => {
            let index = (c as usize) - ('1' as usize);
            if let Some(id) = app.room_order.get(index).copied() {
                activate_room(app, id);
            }
        }

        // -- scrollback --
        (KeyCode::PageUp, _, _) => scroll_up(app, 10),
        (KeyCode::PageDown, _, _) => scroll_down(app, 10),
        (KeyCode::Esc, _, _) => {
            if let Some(room) = app.active_room_mut() {
                room.scroll = 0;
            }
        }

        // -- input editing --
        (KeyCode::Enter, _, _) => submit(app),
        (KeyCode::Backspace, false, false) => app.input.backspace(),
        // Ctrl+Backspace, Alt+Backspace, and the terminals that report the
        // first as Ctrl+H all mean the same thing.
        (KeyCode::Backspace, true | false, true)
        | (KeyCode::Backspace | KeyCode::Char('h'), true, _) => {
            app.input.delete_word();
        }
        (KeyCode::Delete, _, _) => app.input.delete(),
        (KeyCode::Left, _, _) => app.input.left(),
        (KeyCode::Right, _, _) => app.input.right(),
        (KeyCode::Home, _, _) | (KeyCode::Char('a'), true, _) => app.input.home(),
        (KeyCode::End, _, _) | (KeyCode::Char('e'), true, _) => app.input.end(),
        (KeyCode::Char('u'), true, _) => app.input.clear(),
        (KeyCode::Up, false, false) => app.input.history_prev(),
        (KeyCode::Down, false, false) => app.input.history_next(),
        (KeyCode::Tab, _, _) => {
            let candidates = app.completion_candidates();
            app.input.complete(&candidates);
        }
        (KeyCode::Char(c), false, false) => {
            app.input.insert(c);
            signal_typing(app);
        }
        _ => {}
    }
}

fn on_overlay_key(app: &mut App, key: KeyEvent) {
    let Some(overlay) = app.overlay.as_mut() else {
        return;
    };

    match overlay {
        Overlay::ConfirmQuit => match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => app.quit = true,
            _ => app.overlay = None,
        },

        Overlay::AuthFailed { .. } => {
            if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) {
                app.quit = true;
            }
        }

        Overlay::RoomSwitcher { filter, selected } => match key.code {
            KeyCode::Esc => app.overlay = None,
            KeyCode::Up => *selected = selected.saturating_sub(1),
            KeyCode::Down => *selected += 1, // renderer clamps
            KeyCode::Backspace => {
                filter.pop();
                *selected = 0;
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                filter.push(c);
                *selected = 0;
            }
            KeyCode::Enter => {
                let choice = switcher_entries(app).into_iter().nth(match &app.overlay {
                    Some(Overlay::RoomSwitcher { selected, .. }) => *selected,
                    _ => 0,
                });
                app.overlay = None;
                match choice {
                    Some(SwitcherEntry::Joined(id)) => activate_room(app, id),
                    Some(SwitcherEntry::Joinable(name)) => {
                        app.pending_activate = Some(Instant::now());
                        app.send(ClientFrame::Join { name });
                    }
                    None => {}
                }
            }
            _ => {}
        },

        // The read-only overlays close on anything sensible.
        Overlay::Help | Overlay::Profile | Overlay::Notifications => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q'))
                || matches!(key.code, KeyCode::F(1))
            {
                app.overlay = None;
            }
        }
    }
}

/// What the room switcher lists for the current filter: joined rooms first,
/// then joinable discovered channels.
pub enum SwitcherEntry {
    /// A room we are in: switch to it.
    Joined(teresa_proto::RoomId),
    /// A channel we could join, by name.
    Joinable(String),
}

/// The switcher's visible entries, filtered. Pub for the renderer, which must
/// show exactly what Enter will pick.
#[must_use]
pub fn switcher_entries(app: &App) -> Vec<SwitcherEntry> {
    let filter = match &app.overlay {
        Some(Overlay::RoomSwitcher { filter, .. }) => filter.to_lowercase(),
        _ => String::new(),
    };

    let mut entries = Vec::new();
    for id in &app.room_order {
        if let Some(state) = app.rooms.get(id) {
            if state.label().to_lowercase().contains(&filter) {
                entries.push(SwitcherEntry::Joined(*id));
            }
        }
    }
    for room in &app.discovered {
        let joined = app.rooms.contains_key(&room.id);
        if !joined && room.name.to_lowercase().contains(&filter) {
            entries.push(SwitcherEntry::Joinable(room.name.clone()));
        }
    }
    entries
}

// --- input submission ------------------------------------------------------

fn submit(app: &mut App) {
    let line = app.input.take();
    if line.trim().is_empty() {
        return;
    }
    stop_typing(app);
    execute(app, parse(&line));
}

#[allow(clippy::too_many_lines)] // one arm per command; the width is the command set's
fn execute(app: &mut App, command: Command) {
    match command {
        Command::Say(text) => {
            let Some(room_id) = app.active else {
                app.toast(ToastKind::Error, "no room selected — Ctrl+W");
                return;
            };
            let nonce = app.nonce();
            if let Some(room) = app.rooms.get_mut(&room_id) {
                room.pending.insert(
                    nonce.clone(),
                    super::PendingSend {
                        body: text.clone(),
                        emote: false,
                        at: Instant::now(),
                    },
                );
                room.scroll = 0; // sending snaps to the tail
            }
            app.send(ClientFrame::Send {
                room: room_id,
                body: text,
                nonce,
            });
        }

        Command::Me(action) => {
            let Some(room_id) = app.active else {
                app.toast(ToastKind::Error, "no room selected — Ctrl+W");
                return;
            };
            let nonce = app.nonce();
            if let Some(room) = app.rooms.get_mut(&room_id) {
                room.pending.insert(
                    nonce.clone(),
                    super::PendingSend {
                        body: action.clone(),
                        emote: true,
                        at: Instant::now(),
                    },
                );
                room.scroll = 0;
            }
            app.send(ClientFrame::Emote {
                room: room_id,
                body: action,
                nonce,
            });
        }

        Command::Action { verb, target } => {
            let Some(room_id) = app.active else {
                app.toast(ToastKind::Error, "no room selected — Ctrl+W");
                return;
            };
            let spec = super::commands::action(&verb).expect("parser only emits known verbs");
            let body = spec.body(&target);
            let nonce = app.nonce();
            if let Some(room) = app.rooms.get_mut(&room_id) {
                room.pending.insert(
                    nonce.clone(),
                    super::PendingSend {
                        body: body.clone(),
                        emote: true,
                        at: Instant::now(),
                    },
                );
                room.scroll = 0;
            }
            app.send(ClientFrame::Emote {
                room: room_id,
                body,
                nonce,
            });
        }

        Command::Help => app.overlay = Some(Overlay::Help),

        Command::Users => {
            if let Some(room_id) = app.active {
                app.send(ClientFrame::Members { room: room_id });
                let count = app
                    .active_room()
                    .map(|room| room.members.len())
                    .unwrap_or_default();
                app.local_note(format!(
                    "{count} members — list refreshed in the side panel"
                ));
            }
        }

        Command::Whoami => {
            let note = match (&app.me, &app.server) {
                (Some(me), Some(server)) => format!(
                    "you are {} (@{}) — {} on {} · role {}",
                    me.display_name, me.username, me.status, server.name, me.role
                ),
                (Some(me), None) => format!("you are {} (@{})", me.display_name, me.username),
                _ => "not connected yet".to_owned(),
            };
            app.local_note(note);
        }

        Command::Rooms => app.send(ClientFrame::ListRooms),

        Command::Join(name) => {
            app.pending_activate = Some(Instant::now());
            app.send(ClientFrame::Join { name });
        }

        Command::Create {
            name,
            visibility,
            topic,
        } => {
            app.pending_activate = Some(Instant::now());
            app.send(ClientFrame::CreateRoom {
                name,
                topic,
                visibility,
            });
        }

        Command::Delete(name) => {
            let target = match name {
                Some(name) => {
                    let wanted = name.trim_start_matches('#').to_lowercase();
                    app.rooms
                        .values()
                        .find(|state| state.info.room.name == wanted)
                        .map(|state| state.info.room.id)
                }
                None => app.active,
            };
            match target {
                Some(room) => app.send(ClientFrame::DeleteRoom { room }),
                None => app.toast(ToastKind::Error, "no such room to delete"),
            }
        }

        Command::Invite(username) => {
            let Some(room_id) = app.active else {
                app.toast(ToastKind::Error, "join a room before inviting anyone");
                return;
            };
            app.send(ClientFrame::Invite {
                room: room_id,
                username,
            });
        }

        Command::Dm(username) => {
            app.pending_activate = Some(Instant::now());
            app.send(ClientFrame::OpenDm { username });
        }

        Command::Leave => {
            if let Some(room_id) = app.active {
                app.send(ClientFrame::Leave { room: room_id });
            }
        }

        Command::Nick(display_name) => {
            app.send(ClientFrame::SetDisplayName { display_name });
        }

        Command::Status { status, text } => {
            app.send(ClientFrame::SetStatus { status, text });
            app.local_note(format!("status set to {status}"));
        }

        Command::Theme(Some(name)) => match Theme::by_name_checked(&name) {
            Some(theme) => {
                app.theme = theme;
                app.config.theme = name;
                app.effects.push(Effect::PersistConfig);
                app.toast(ToastKind::Success, format!("theme: {}", app.theme.name));
            }
            None => {
                app.toast(
                    ToastKind::Error,
                    format!("no theme {name:?} — /theme lists them"),
                );
            }
        },
        Command::Theme(None) => {
            app.local_note(format!("themes: {}", Theme::roster().join(", ")));
        }

        Command::Send { to, path, zip } => {
            let path = match transfer::resolve_path(&path) {
                Ok(path) => path,
                Err(error) => {
                    app.toast(ToastKind::Error, error);
                    return;
                }
            };
            let Some(peer) = to else {
                let Some(room_id) = app.active else {
                    app.toast(ToastKind::Error, "no room selected — Ctrl+W");
                    return;
                };
                start_upload(app, room_id, path, zip);
                return;
            };
            if app.me.as_ref().is_some_and(|me| me.username == peer) {
                app.toast(ToastKind::Error, "sending files to yourself? use cp");
                return;
            }
            let dm = app.room_order.iter().copied().find(|id| {
                app.rooms
                    .get(id)
                    .is_some_and(|r| r.info.peer.as_deref() == Some(peer.as_str()))
            });
            if let Some(room_id) = dm {
                start_upload(app, room_id, path, zip);
            } else {
                app.local_note(format!(
                    "opening a secure line to @{peer} for the transfer…"
                ));
                app.pending_dm_sends.push(super::PendingDmSend {
                    peer: peer.clone(),
                    path,
                    zip,
                    at: Instant::now(),
                });
                app.send(ClientFrame::OpenDm { username: peer });
            }
        }

        Command::Get(which) => {
            let cards = app.file_cards();
            if cards.is_empty() {
                app.toast(ToastKind::Error, "no files shared in this room yet");
                return;
            }
            let picked = match which.as_deref().map(str::trim) {
                None | Some("") => cards.first(),
                Some(arg) => {
                    if let Ok(n) = arg.trim_start_matches('#').parse::<usize>() {
                        n.checked_sub(1).and_then(|i| cards.get(i))
                    } else {
                        let needle = arg.to_lowercase();
                        cards
                            .iter()
                            .find(|(_, card)| card.name.to_lowercase().contains(&needle))
                    }
                }
            };
            let Some((_, card)) = picked else {
                app.toast(ToastKind::Error, "no such file — /files lists them");
                return;
            };
            let card = card.clone();
            if app
                .transfer_for(card.id)
                .is_some_and(super::Transfer::active)
            {
                app.toast(
                    ToastKind::Info,
                    format!("{} is already coming down", card.name),
                );
                return;
            }
            let id = app.next_transfer;
            app.next_transfer += 1;
            let place = app
                .active_room()
                .map(super::RoomState::label)
                .unwrap_or_default();
            let now = Instant::now();
            app.transfers.push(super::Transfer {
                id,
                upload: false,
                name: card.landing_name().to_owned(),
                place,
                file: Some(card.id),
                stage: Stage::Receiving {
                    done: 0,
                    total: card.size,
                },
                started: now,
                stage_started: now,
                ended: None,
                wire: None,
            });
            app.effects.push(Effect::Download { id, card });
        }

        Command::Files => {
            let lines: Vec<String> = app
                .file_cards()
                .iter()
                .enumerate()
                .map(|(i, (message, card))| {
                    let extra = card
                        .entries
                        .map(|n| format!(" · {n} files"))
                        .unwrap_or_default();
                    format!(
                        "#{} {} {} · {}{extra} · from {}",
                        i + 1,
                        card.kind.badge(),
                        card.name,
                        human_bytes(card.size),
                        message.author_label()
                    )
                })
                .collect();
            if lines.is_empty() {
                app.local_note("no files shared here yet — /send <path> to share one");
            } else {
                app.local_note(format!(
                    "{} file(s) in this room — /get <#> to pull one:",
                    lines.len()
                ));
                for line in lines.into_iter().take(20) {
                    app.local_note(line);
                }
            }
        }

        Command::Admin { op, confirmed } => {
            let warning = match op {
                MaintenanceOp::PurgeFiles { older_than_days } => Some(format!(
                    "⚠ deletes every shared file {} from the node — their cards stay, /get stops working",
                    age_phrase(older_than_days)
                )),
                MaintenanceOp::PurgeMessages { older_than_days } => Some(format!(
                    "⚠ deletes every message {}, in every room, for everyone — there is no undo",
                    age_phrase(older_than_days)
                )),
                MaintenanceOp::Stats | MaintenanceOp::Compact => None,
            };
            match warning {
                Some(warning) if !confirmed => {
                    app.local_note(warning);
                    app.local_note("repeat the command with --yes to go ahead");
                }
                _ => {
                    if warning.is_some() {
                        app.local_note("⛁ maintenance running on the node…");
                    }
                    app.send(ClientFrame::Maintenance { op });
                }
            }
        }

        Command::Clear => clear_active_room(app),

        Command::Ping => {
            let nonce = app.next_nonce;
            app.next_nonce += 1;
            app.pending_pings.insert(nonce, Instant::now());
            app.send(ClientFrame::Ping { nonce });
        }

        Command::Version => {
            let server = app.server.as_ref().map_or_else(
                || "server: not connected".to_owned(),
                |s| format!("{} v{} (protocol {})", s.name, s.version, s.protocol),
            );
            app.local_note(format!("teresa client v{} · {server}", crate::VERSION));
        }

        Command::Logout => {
            app.effects.push(Effect::Logout);
            app.quit = true;
        }

        Command::Quit => app.quit = true,

        Command::Unknown(cmd) if cmd.is_empty() => {
            app.toast(ToastKind::Error, "missing argument — /help shows usage");
        }
        Command::Unknown(cmd) => {
            app.toast(ToastKind::Error, format!("unknown command {cmd} — /help"));
        }
    }
}

// --- shared behaviours -----------------------------------------------------

/// Registers an upload in the dock and asks the main loop to run it.
fn start_upload(app: &mut App, room: RoomId, path: std::path::PathBuf, zip: bool) {
    let id = app.next_transfer;
    app.next_transfer += 1;
    let is_dir = path.is_dir();
    let mut name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    if is_dir || zip {
        name.push_str(".zip");
    }
    let place = app
        .rooms
        .get(&room)
        .map(super::RoomState::label)
        .unwrap_or_default();
    let now = Instant::now();
    app.transfers.push(super::Transfer {
        id,
        upload: true,
        name,
        place,
        file: None,
        stage: if is_dir || zip {
            Stage::Packing { done: 0, total: 0 }
        } else {
            Stage::Sending {
                done: 0,
                total: std::fs::metadata(&path).map_or(0, |m| m.len()),
            }
        },
        started: now,
        stage_started: now,
        ended: None,
        wire: None,
    });
    if let Some(state) = app.rooms.get_mut(&room) {
        state.scroll = 0;
    }
    app.effects.push(Effect::Upload {
        id,
        room,
        path,
        zip,
    });
    app.dirty = true;
}

/// Applies a progress report, and announces the ending of a transfer.
fn on_transfer(app: &mut App, update: TransferUpdate) {
    app.dirty = true;
    let Some(transfer) = app.transfers.iter_mut().find(|t| t.id == update.id) else {
        return;
    };
    if transfer.ended.is_some() {
        return; // a late progress tick after the final word
    }
    if std::mem::discriminant(&transfer.stage) != std::mem::discriminant(&update.stage) {
        if let Stage::Sending { total, .. } | Stage::Receiving { total, .. } = transfer.stage {
            transfer.wire = Some((total, transfer.stage_started.elapsed()));
        }
        transfer.stage_started = Instant::now();
    }
    let took = transfer.started.elapsed();
    transfer.stage = update.stage;

    let ending = match &transfer.stage {
        Stage::Done(Outcome::Shared(card)) => {
            transfer.file = Some(card.id);
            Some((
                ToastKind::Success,
                format!(
                    "▲ {} → {} · {} in {} · sha256 {}…",
                    card.name,
                    transfer.place,
                    human_bytes(card.size),
                    clock_of(took),
                    &card.sha256[..card.sha256.len().min(8)]
                ),
            ))
        }
        Stage::Done(Outcome::Saved(path)) => Some((
            ToastKind::Success,
            format!("▼ verified ✔ saved to {}", path.display()),
        )),
        Stage::Failed(reason) => {
            Some((ToastKind::Error, format!("✖ {} — {reason}", transfer.name)))
        }
        _ => None,
    };
    if let Some((kind, text)) = ending {
        transfer.ended = Some(Instant::now());
        let note = match &transfer.stage {
            Stage::Done(Outcome::Saved(path)) => {
                Some(format!("▼ {} landed at {}", transfer.name, path.display()))
            }
            _ => None,
        };
        if let Some(note) = note {
            app.local_note(note);
        }
        app.toast(kind, text);
    }
}

/// "older than 30 days" / "regardless of age".
fn age_phrase(days: u32) -> String {
    match days {
        0 => "regardless of age".to_owned(),
        1 => "older than 1 day".to_owned(),
        n => format!("older than {n} days"),
    }
}

/// Renders a maintenance report into the active room as a small readout.
fn on_maintenance(app: &mut App, report: &MaintenanceReport) {
    let stats = report.stats;
    let node = app
        .server
        .as_ref()
        .map_or_else(|| "node".to_owned(), |s| s.name.clone());
    let done = match report.op {
        MaintenanceOp::Stats => None,
        MaintenanceOp::PurgeFiles { .. } => Some(format!(
            "removed {} file(s), freed {}",
            report.removed,
            human_bytes(report.freed_bytes)
        )),
        MaintenanceOp::PurgeMessages { .. } => Some(format!(
            "removed {} message(s), freed {}",
            report.removed,
            human_bytes(report.freed_bytes)
        )),
        MaintenanceOp::Compact => Some(format!(
            "database compacted, freed {}",
            human_bytes(report.freed_bytes)
        )),
    };
    app.local_note(format!("⛁ STORAGE · {node}"));
    app.local_note(format!(
        "  database  {:>10}  ·  {} messages · {} users",
        human_bytes(stats.db_bytes),
        stats.messages,
        stats.users
    ));
    app.local_note(format!(
        "  files     {:>10}  ·  {} shared",
        human_bytes(stats.file_bytes),
        stats.files
    ));
    app.local_note(format!(
        "  total     {:>10}",
        human_bytes(stats.db_bytes + stats.file_bytes)
    ));
    if let Some(done) = done {
        app.local_note(format!("  ✔ {done}"));
        app.toast(ToastKind::Success, format!("⛁ {done}"));
    }
}

/// `m:ss` for toasts.
fn clock_of(elapsed: Duration) -> String {
    let secs = elapsed.as_secs_f32();
    if secs < 10.0 {
        format!("{secs:.1}s")
    } else {
        let secs = elapsed.as_secs();
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn activate_room(app: &mut App, id: teresa_proto::RoomId) {
    app.active = Some(id);
    app.unseen_activity = false;

    let Some(state) = app.rooms.get_mut(&id) else {
        return;
    };
    state.scroll = 0;
    state.info.unread = 0;
    state.mentions = 0;

    // Fresh eyes on the room: sync read marker, member list, and the message
    // tail (which also heals any gap from a reconnect).
    app.send(ClientFrame::MarkRead { room: id });
    app.send(ClientFrame::Members { room: id });
    let state = app.rooms.get_mut(&id).expect("present above");
    if !state.fetching_history {
        state.fetching_history = true;
        state.fetching_older = false;
        app.send(ClientFrame::History {
            room: id,
            before: None,
            limit: HISTORY_PAGE_DEFAULT,
        });
    }
    app.dirty = true;
}

fn cycle_room(app: &mut App, step: isize) {
    if app.room_order.is_empty() {
        return;
    }
    let current = app
        .active
        .and_then(|id| app.room_order.iter().position(|r| *r == id))
        .unwrap_or(0);
    // Room counts are tiny; the isize round-trip cannot wrap in practice.
    #[allow(clippy::cast_possible_wrap)]
    let next = (current as isize + step).rem_euclid(app.room_order.len() as isize) as usize;
    activate_room(app, app.room_order[next]);
}

fn clear_active_room(app: &mut App) {
    if let Some(room) = app.active_room_mut() {
        room.messages.clear();
        room.seen.clear();
        room.history_complete = false;
        room.scroll = 0;
    }
    app.local_note("scrollback cleared");
}

fn scroll_up(app: &mut App, lines: u16) {
    let Some(active) = app.active else {
        return;
    };
    let Some(room) = app.rooms.get_mut(&active) else {
        return;
    };
    room.scroll = room.scroll.saturating_add(lines);

    // Nearing the top of what we hold: fetch the page before it.
    let held = room.messages.len();
    let near_top = usize::from(room.scroll) + 30 > held;
    if near_top && !room.history_complete && !room.fetching_history {
        room.fetching_history = true;
        room.fetching_older = true;
        let before = room.messages.first().map(|m| m.id);
        app.send(ClientFrame::History {
            room: active,
            before,
            limit: HISTORY_PAGE_DEFAULT,
        });
    }
}

fn scroll_down(app: &mut App, lines: u16) {
    if let Some(room) = app.active_room_mut() {
        room.scroll = room.scroll.saturating_sub(lines);
        if room.scroll == 0 {
            // Caught back up with the live tail.
            let id = room.info.room.id;
            app.send(ClientFrame::MarkRead { room: id });
        }
    }
}

fn signal_typing(app: &mut App) {
    let Some(room_id) = app.active else {
        return;
    };
    if app.input.text().starts_with('/') {
        return; // commands are not conversation
    }
    let due = app
        .typing_sent
        .is_none_or(|at| at.elapsed() >= TYPING_THROTTLE);
    if due {
        app.typing_sent = Some(Instant::now());
        app.send(ClientFrame::Typing {
            room: room_id,
            active: true,
        });
    }
}

fn stop_typing(app: &mut App) {
    if app.typing_sent.take().is_some() {
        if let Some(room_id) = app.active {
            app.send(ClientFrame::Typing {
                room: room_id,
                active: false,
            });
        }
    }
}

fn on_paste(app: &mut App, text: &str) {
    // Terminal paste arrives with \r line endings from some emulators.
    let normalised = text.replace('\r', "\n");
    app.input.insert_str(&normalised);
    signal_typing(app);
    app.dirty = true;
}

fn on_mouse(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => scroll_up(app, 3),
        MouseEventKind::ScrollDown => scroll_down(app, 3),
        _ => return,
    }
    app.dirty = true;
}

fn on_tick(app: &mut App) {
    let now = Instant::now();
    let mut animated = false;

    // Expire toasts.
    let before = app.toasts.len();
    app.toasts.retain(|toast| toast.until > now);
    animated |= app.toasts.len() != before || !app.toasts.is_empty();

    // Expire stale typing indicators and spent action hits.
    for state in app.rooms.values_mut() {
        let before = state.typing.len();
        state
            .typing
            .retain(|(_, _, at)| at.elapsed() < TYPING_EXPIRY);
        animated |= state.typing.len() != before || !state.typing.is_empty();

        let before = state.fx.len();
        state.fx.retain(|fx| fx.at.elapsed() < FX_DURATION);
        animated |= state.fx.len() != before || !state.fx.is_empty();
    }

    // Finished transfers bow out of the dock; abandoned DM sends give up.
    let before = app.transfers.len();
    app.transfers.retain(|t| {
        let linger = if matches!(t.stage, Stage::Failed(_)) {
            TRANSFER_FAIL_LINGER
        } else {
            TRANSFER_LINGER
        };
        t.ended.is_none_or(|at| at.elapsed() < linger)
    });
    animated |= app.transfers.len() != before || !app.transfers.is_empty();

    let expired: Vec<String> = app
        .pending_dm_sends
        .iter()
        .filter(|send| send.at.elapsed() >= DM_SEND_WINDOW)
        .map(|send| send.peer.clone())
        .collect();
    if !expired.is_empty() {
        app.pending_dm_sends
            .retain(|send| send.at.elapsed() < DM_SEND_WINDOW);
        for peer in expired {
            app.toast(
                ToastKind::Error,
                format!("could not open a line to @{peer} — transfer cancelled"),
            );
        }
    }

    // Keep animating while anything on screen moves: the boot splash, the
    // connect spinner, live typing dots.
    let booting = app.started.elapsed() < crate::ui::anim::BOOT_DURATION;
    let spinning = !matches!(app.conn, ConnState::Online { .. });
    if booting || spinning || animated {
        app.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use teresa_proto::{
        Author, JoinedRoom, MemberRole, Message, MessageId, MessageKind, Room, RoomId, ServerInfo,
        Timestamp, User, UserId, UserRole, UserStatus, Visibility,
    };

    use super::*;
    use crate::app::commands;
    use crate::config::ClientConfig;

    fn user(name: &str) -> User {
        User {
            id: UserId::new(),
            username: name.into(),
            display_name: name.into(),
            status: UserStatus::Online,
            status_text: None,
            role: UserRole::Member,
            created_at: Timestamp::now(),
        }
    }

    fn room(name: &str) -> Room {
        Room {
            id: RoomId::new(),
            name: name.into(),
            topic: None,
            kind: RoomKind::Channel,
            visibility: Visibility::Public,
            owner_id: None,
            created_at: Timestamp::now(),
            member_count: 1,
        }
    }

    fn joined(room: Room) -> JoinedRoom {
        JoinedRoom {
            room,
            role: MemberRole::Member,
            unread: 0,
            peer: None,
        }
    }

    fn chat(room: RoomId, author: &User, body: &str) -> Message {
        Message {
            id: MessageId::new(),
            room_id: room,
            author: Some(Author {
                id: author.id,
                username: author.username.clone(),
                display_name: author.display_name.clone(),
            }),
            kind: MessageKind::Chat,
            body: body.into(),
            created_at: Timestamp::now(),
        }
    }

    fn ready_app() -> (App, User, Room) {
        let mut app = App::new(ClientConfig::default());
        let me = user("neo");
        let general = room("general");
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Ready {
                user: me.clone(),
                rooms: vec![joined(general.clone())],
                server: ServerInfo {
                    name: "zion".into(),
                    version: "1.0.0".into(),
                    motd: None,
                    protocol: teresa_proto::PROTOCOL_VERSION,
                },
            })),
        );
        app.effects.clear();
        (app, me, general)
    }

    #[test]
    fn ready_activates_the_first_room_and_requests_its_state() {
        let mut app = App::new(ClientConfig::default());
        let general = room("general");
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Ready {
                user: user("neo"),
                rooms: vec![joined(general.clone())],
                server: ServerInfo {
                    name: "zion".into(),
                    version: "1".into(),
                    motd: None,
                    protocol: 1,
                },
            })),
        );

        assert_eq!(app.active, Some(general.id));
        assert!(matches!(app.conn, ConnState::Online { .. }));
        // MarkRead + Members + History all requested for the active room.
        let frames: Vec<_> = app
            .effects
            .iter()
            .filter_map(|e| match e {
                Effect::Send(f) => Some(f),
                _ => None,
            })
            .collect();
        assert!(frames
            .iter()
            .any(|f| matches!(f, ClientFrame::History { .. })));
        assert!(frames
            .iter()
            .any(|f| matches!(f, ClientFrame::Members { .. })));
        assert!(frames
            .iter()
            .any(|f| matches!(f, ClientFrame::MarkRead { .. })));
    }

    #[test]
    fn a_message_for_the_watched_room_marks_itself_read() {
        let (mut app, _me, general) = ready_app();
        let alice = user("alice");

        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Message {
                message: chat(general.id, &alice, "hello"),
            })),
        );

        let state = app.active_room().unwrap();
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.info.unread, 0);
        assert!(app
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Send(ClientFrame::MarkRead { .. }))));
    }

    #[test]
    fn duplicate_messages_are_dropped() {
        let (mut app, _me, general) = ready_app();
        let alice = user("alice");
        let message = chat(general.id, &alice, "once");

        for _ in 0..3 {
            update(
                &mut app,
                Event::Net(NetEvent::Frame(ServerFrame::Message {
                    message: message.clone(),
                })),
            );
        }
        assert_eq!(app.active_room().unwrap().messages.len(), 1);
    }

    #[test]
    fn posted_resolves_the_pending_echo() {
        let (mut app, me, general) = ready_app();

        app.input.insert_str("ship it");
        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Enter)));

        let state = app.active_room().unwrap();
        assert_eq!(state.pending.len(), 1);
        let nonce = state.pending.keys().next().unwrap().clone();

        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Posted {
                message: chat(general.id, &me, "ship it"),
                nonce,
            })),
        );

        let state = app.active_room().unwrap();
        assert!(state.pending.is_empty());
        assert_eq!(state.messages.len(), 1);
    }

    #[test]
    fn mention_in_a_background_room_counts_bells_and_toasts() {
        let (mut app, _me, general) = ready_app();
        let side = room("side");
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Joined {
                room: joined(side.clone()),
                members: vec![],
            })),
        );
        // The Joined was not requested inside the window → background room…
        // (pending_activate None because ready_app cleared effects, not intent)
        assert_eq!(app.active, Some(general.id), "screen must not be yanked");

        let alice = user("alice");
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Message {
                message: chat(side.id, &alice, "ping @neo, look"),
            })),
        );

        let state = app.rooms.get(&side.id).unwrap();
        assert_eq!(state.info.unread, 1);
        assert_eq!(state.mentions, 1);
        assert!(app.unseen_activity);
        assert!(app.effects.contains(&Effect::Bell));
        assert!(app.toasts.iter().any(|t| t.kind == ToastKind::Mention));
    }

    #[test]
    fn own_joins_switch_the_screen() {
        let (mut app, _me, _general) = ready_app();
        let side = room("side");

        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Char('/'))));
        for c in "join side".chars() {
            update(&mut app, Event::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Enter)));

        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Joined {
                room: joined(side.clone()),
                members: vec![],
            })),
        );
        assert_eq!(
            app.active,
            Some(side.id),
            "a requested join takes the screen"
        );
    }

    #[test]
    fn slash_say_escape_and_typing_signals() {
        let (mut app, _me, general) = ready_app();

        // Typing chat text signals typing, throttled to one frame.
        for c in "hey".chars() {
            update(&mut app, Event::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        let typing_frames = app
            .effects
            .iter()
            .filter(|e| {
                matches!(e, Effect::Send(ClientFrame::Typing { active: true, room })
                    if *room == general.id)
            })
            .count();
        assert_eq!(typing_frames, 1);

        // Submitting sends the text and a typing-stop.
        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Enter)));
        assert!(app
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Send(ClientFrame::Send { body, .. }) if body == "hey")));
        assert!(app
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Send(ClientFrame::Typing { active: false, .. }))));
    }

    #[test]
    fn commands_do_not_leak_typing_signals() {
        let (mut app, _me, _general) = ready_app();
        for c in "/status coding".chars() {
            update(&mut app, Event::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        assert!(
            !app.effects
                .iter()
                .any(|e| matches!(e, Effect::Send(ClientFrame::Typing { .. }))),
            "typing a command must not announce typing"
        );
    }

    #[test]
    fn theme_switching_persists_and_rejects_nonsense() {
        let (mut app, _me, _general) = ready_app();

        for c in "/theme dracula".chars() {
            update(&mut app, Event::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Enter)));
        assert_eq!(app.config.theme, "dracula");
        assert!(app.effects.contains(&Effect::PersistConfig));

        for c in "/theme nonsense".chars() {
            update(&mut app, Event::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Enter)));
        assert_eq!(app.config.theme, "dracula", "unknown theme changes nothing");
    }

    #[test]
    fn quit_needs_confirmation_and_esc_cancels() {
        let (mut app, _me, _general) = ready_app();
        update(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        );
        assert_eq!(app.overlay, Some(Overlay::ConfirmQuit));
        assert!(!app.quit);

        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Esc)));
        assert!(app.overlay.is_none());
        assert!(!app.quit);

        update(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        );
        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Enter)));
        assert!(app.quit);
    }

    #[test]
    fn auth_rejection_is_terminal_and_overlayed() {
        let (mut app, _me, _general) = ready_app();
        update(
            &mut app,
            Event::Net(NetEvent::AuthRejected {
                message: "session expired".into(),
            }),
        );
        assert!(matches!(app.conn, ConnState::AuthFailed { .. }));
        assert!(matches!(app.overlay, Some(Overlay::AuthFailed { .. })));

        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Enter)));
        assert!(app.quit);
    }

    #[test]
    fn scrolling_up_near_the_top_requests_older_history() {
        let (mut app, _me, general) = ready_app();
        let alice = user("alice");
        for i in 0..5 {
            update(
                &mut app,
                Event::Net(NetEvent::Frame(ServerFrame::Message {
                    message: chat(general.id, &alice, &format!("m{i}")),
                })),
            );
        }
        app.effects.clear();
        // ready_app's activation already has a history fetch in flight; land it.
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::History {
                room: general.id,
                messages: vec![],
                complete: false,
            })),
        );

        update(&mut app, Event::Key(KeyEvent::from(KeyCode::PageUp)));
        assert!(app.effects.iter().any(|e| matches!(
            e,
            Effect::Send(ClientFrame::History {
                before: Some(_),
                ..
            })
        )));
        assert!(app.active_room().unwrap().fetching_older);
    }

    #[test]
    fn history_pages_merge_without_duplicates_in_order() {
        let (mut app, _me, general) = ready_app();
        let alice = user("alice");
        let old_a = chat(general.id, &alice, "old a");
        let old_b = chat(general.id, &alice, "old b");
        let live = chat(general.id, &alice, "live");

        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Message {
                message: live.clone(),
            })),
        );

        // Older page arrives (with one message we somehow already hold).
        {
            let state = app.active_room_mut().unwrap();
            state.fetching_history = true;
            state.fetching_older = true;
        }
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::History {
                room: general.id,
                messages: vec![old_a.clone(), old_b.clone(), live.clone()],
                complete: true,
            })),
        );

        let bodies: Vec<_> = app
            .active_room()
            .unwrap()
            .messages
            .iter()
            .map(|m| m.body.as_str())
            .collect();
        assert_eq!(bodies, ["old a", "old b", "live"]);
        assert!(app.active_room().unwrap().history_complete);
    }

    #[test]
    fn room_deleted_falls_back_to_another_room() {
        let (mut app, _me, general) = ready_app();
        let side = room("side");
        app.pending_activate = Some(Instant::now());
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Joined {
                room: joined(side.clone()),
                members: vec![],
            })),
        );
        assert_eq!(app.active, Some(side.id));

        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::RoomDeleted { room: side.id })),
        );
        assert_eq!(app.active, Some(general.id));
        assert!(!app.rooms.contains_key(&side.id));
    }

    #[test]
    fn shoot_sends_a_targeted_emote() {
        let (mut app, _me, general) = ready_app();
        let bob = user("bob");
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Members {
                room: general.id,
                members: vec![bob],
            })),
        );

        for c in "/shoot @bob".chars() {
            update(&mut app, Event::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Enter)));

        let sent = app.effects.iter().find_map(|e| match e {
            Effect::Send(ClientFrame::Emote { body, .. }) => Some(body.clone()),
            _ => None,
        });
        let body = sent.expect("an emote frame went out");
        assert!(body.starts_with("🔫 "), "carries the wire signature");
        assert!(body.contains("@bob"), "mentions the target");
        assert_eq!(
            app.active_room().unwrap().pending.len(),
            1,
            "optimistic echo pending"
        );
    }

    #[test]
    fn an_incoming_shot_flashes_the_target_and_toasts_the_victim() {
        let (mut app, me, general) = ready_app();
        let alice = user("alice");
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Members {
                room: general.id,
                members: vec![alice.clone(), me.clone()],
            })),
        );

        // Alice shoots me, in the room I am watching.
        let mut shot = chat(
            general.id,
            &alice,
            &commands::action("shoot").unwrap().body("neo"),
        );
        shot.kind = MessageKind::Emote;
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Message { message: shot })),
        );

        let state = app.active_room().unwrap();
        assert_eq!(state.fx.len(), 1, "the hit landed");
        assert_eq!(state.fx[0].user, me.id, "on the right member");
        assert_eq!(state.fx[0].glyph, "💥");
        assert!(
            app.toasts.iter().any(|t| t.kind == ToastKind::Mention),
            "the victim is told"
        );
        assert!(app.effects.contains(&Effect::Bell));
    }

    #[test]
    fn own_shots_register_the_hit_via_the_posted_ack() {
        let (mut app, me, general) = ready_app();
        let bob = user("bob");
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Members {
                room: general.id,
                members: vec![bob.clone()],
            })),
        );

        for c in "/shoot bob".chars() {
            update(&mut app, Event::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Enter)));
        let nonce = app
            .active_room()
            .unwrap()
            .pending
            .keys()
            .next()
            .unwrap()
            .clone();

        let mut echo = chat(
            general.id,
            &me,
            &commands::action("shoot").unwrap().body("bob"),
        );
        echo.kind = MessageKind::Emote;
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Posted {
                message: echo.clone(),
                nonce,
            })),
        );
        let state = app.active_room().unwrap();
        assert_eq!(state.fx.len(), 1, "shooter sees the hit too");
        assert_eq!(state.fx[0].user, bob.id);

        // The room broadcast of the same message must not double the hit.
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Message { message: echo })),
        );
        assert_eq!(app.active_room().unwrap().fx.len(), 1);
    }

    #[test]
    fn pong_updates_rtt() {
        let (mut app, _me, _general) = ready_app();
        for c in "/ping".chars() {
            update(&mut app, Event::Key(KeyEvent::from(KeyCode::Char(c))));
        }
        update(&mut app, Event::Key(KeyEvent::from(KeyCode::Enter)));
        let nonce = *app.pending_pings.keys().next().expect("ping outstanding");

        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Pong { nonce })),
        );
        assert!(app.rtt_ms.is_some());
        assert!(app.pending_pings.is_empty());
    }

    fn submit_line(app: &mut App, line: &str) {
        execute(app, parse(line));
    }

    #[test]
    fn send_queues_an_upload_into_the_active_room_and_docks_it() {
        let (mut app, _, general) = ready_app();
        let scratch = tempfile::tempdir().unwrap();
        let file = scratch.path().join("notes.md");
        std::fs::write(&file, "# notes").unwrap();

        submit_line(&mut app, &format!("/send {}", file.display()));
        assert_eq!(
            app.effects,
            vec![Effect::Upload {
                id: 1,
                room: general.id,
                path: file,
                zip: false
            }]
        );
        assert_eq!(app.transfers.len(), 1);
        assert!(app.transfers[0].upload);
        assert_eq!(app.transfers[0].name, "notes.md");

        // A folder is named for the zip it will travel as.
        submit_line(&mut app, &format!("/send {}", scratch.path().display()));
        assert!(std::path::Path::new(&app.transfers[1].name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip")));
    }

    #[test]
    fn send_to_a_person_opens_the_dm_then_uploads_once_it_exists() {
        let (mut app, _, _) = ready_app();
        let scratch = tempfile::tempdir().unwrap();
        let file = scratch.path().join("patch.diff");
        std::fs::write(&file, "+fix").unwrap();

        submit_line(&mut app, &format!("/send @trinity {}", file.display()));
        assert_eq!(
            app.effects,
            vec![Effect::Send(ClientFrame::OpenDm {
                username: "trinity".into()
            })]
        );
        assert!(
            app.transfers.is_empty(),
            "nothing moves before the line opens"
        );
        app.effects.clear();

        let mut dm = joined(Room {
            kind: RoomKind::Dm,
            ..room("dm-x-y")
        });
        dm.peer = Some("trinity".into());
        let dm_id = dm.room.id;
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Joined {
                room: dm,
                members: vec![],
            })),
        );
        assert!(app.effects.contains(&Effect::Upload {
            id: 1,
            room: dm_id,
            path: file,
            zip: false
        }));
        assert!(app.pending_dm_sends.is_empty());
        assert_eq!(app.transfers[0].place, "@trinity");
    }

    #[test]
    fn get_pulls_the_newest_card_by_default_and_by_number() {
        let (mut app, me, general) = ready_app();
        let card = |name: &str| teresa_proto::FileCard {
            id: teresa_proto::FileId::new(),
            name: name.into(),
            size: 42,
            sha256: "ab".repeat(32),
            kind: teresa_proto::FileKind::File,
            entries: None,
        };
        let (old, new) = (card("old.txt"), card("new.txt"));
        for c in [&old, &new] {
            let mut message = chat(general.id, &me, &c.to_body());
            message.kind = MessageKind::File;
            update(
                &mut app,
                Event::Net(NetEvent::Frame(ServerFrame::Message { message })),
            );
        }
        app.effects.clear();

        submit_line(&mut app, "/get");
        assert_eq!(
            app.effects.pop(),
            Some(Effect::Download { id: 1, card: new })
        );
        submit_line(&mut app, "/get 2");
        assert_eq!(
            app.effects.pop(),
            Some(Effect::Download {
                id: 2,
                card: old.clone()
            })
        );

        // Already in flight: no duplicate transfer.
        submit_line(&mut app, "/get old");
        assert!(app.effects.is_empty());

        // Completion stamps the dock and notes where it landed.
        update(
            &mut app,
            Event::Transfer(TransferUpdate {
                id: 2,
                stage: Stage::Done(Outcome::Saved("/tmp/old.txt".into())),
            }),
        );
        assert!(app.transfers.iter().all(|t| t.id != 2 || t.ended.is_some()));
        submit_line(&mut app, "/get old");
        assert!(matches!(app.effects.pop(), Some(Effect::Download { .. })));
    }

    #[test]
    fn destructive_admin_commands_need_yes_and_reports_render() {
        let (mut app, _, _) = ready_app();
        submit_line(&mut app, "/admin messages 30");
        assert!(app.effects.is_empty(), "no --yes, nothing sent");

        submit_line(&mut app, "/admin messages 30 --yes");
        let op = MaintenanceOp::PurgeMessages {
            older_than_days: 30,
        };
        assert_eq!(
            app.effects,
            vec![Effect::Send(ClientFrame::Maintenance { op })]
        );
        app.effects.clear();

        submit_line(&mut app, "/admin");
        assert_eq!(
            app.effects,
            vec![Effect::Send(ClientFrame::Maintenance {
                op: MaintenanceOp::Stats
            })]
        );

        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Maintenance {
                report: MaintenanceReport {
                    op,
                    removed: 12,
                    freed_bytes: 8192,
                    stats: teresa_proto::StorageStats {
                        users: 4,
                        messages: 460,
                        files: 0,
                        file_bytes: 0,
                        db_bytes: 200_000,
                    },
                },
            })),
        );
        let notes: Vec<&str> = app
            .active_room()
            .unwrap()
            .messages
            .iter()
            .map(|m| m.body.as_str())
            .collect();
        assert!(notes.iter().any(|n| n.contains("STORAGE · zion")));
        assert!(notes
            .iter()
            .any(|n| n.contains("removed 12 message(s), freed 8.0 KiB")));
    }
}
