//! TUI snapshot tests: the renderer drawn into a headless buffer.
//!
//! These are what let the "cinematic" layer be refactored fearlessly — if a
//! change garbles the layout, a buffer assertion fails here instead of on
//! someone's screen.

use std::time::{Duration, Instant};

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use teresa_client::app::event::Event;
use teresa_client::app::update::update;
use teresa_client::app::{App, Overlay};
use teresa_client::config::ClientConfig;
use teresa_client::net::ws::NetEvent;
use teresa_client::ui;
use teresa_proto::{
    Author, JoinedRoom, MemberRole, Message, MessageId, MessageKind, Room, RoomId, RoomKind,
    ServerFrame, ServerInfo, Timestamp, User, UserId, UserRole, UserStatus, Visibility,
};

/// Renders the app at a given size and returns the screen as text rows.
fn screen(app: &App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, app))
        .expect("draw succeeds");

    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
        })
        .collect()
}

fn flat(rows: &[String]) -> String {
    rows.join("\n")
}

fn user(name: &str, status: UserStatus) -> User {
    User {
        id: UserId::new(),
        username: name.to_lowercase(),
        display_name: name.to_owned(),
        status,
        status_text: None,
        role: UserRole::Member,
        created_at: Timestamp::now(),
    }
}

fn room(name: &str) -> Room {
    Room {
        id: RoomId::new(),
        name: name.to_owned(),
        topic: Some("api work".to_owned()),
        kind: RoomKind::Channel,
        visibility: Visibility::Public,
        owner_id: None,
        created_at: Timestamp::now(),
        member_count: 2,
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
        body: body.to_owned(),
        created_at: Timestamp::now(),
    }
}

/// An app that is past its boot animation and fully "connected", with the
/// developers room active and populated.
fn ready_app() -> App {
    let mut app = App::new(ClientConfig::default());
    app.started = Instant::now() - Duration::from_secs(10);

    let me = user("Neo", UserStatus::Online);
    let alice = user("Alice", UserStatus::Coding);
    let bob = user("Bob", UserStatus::Away);
    let developers = room("developers");
    let developers_id = developers.id;

    update(
        &mut app,
        Event::Net(NetEvent::Frame(ServerFrame::Ready {
            user: me,
            rooms: vec![JoinedRoom {
                room: developers,
                role: MemberRole::Member,
                unread: 0,
                peer: None,
            }],
            server: ServerInfo {
                name: "zion".to_owned(),
                version: "1.0.0".to_owned(),
                motd: None,
                protocol: teresa_proto::PROTOCOL_VERSION,
            },
        })),
    );
    update(
        &mut app,
        Event::Net(NetEvent::Frame(ServerFrame::Members {
            room: developers_id,
            members: vec![alice.clone(), bob.clone()],
        })),
    );
    for (who, text) in [
        (&alice, "Finished the **API** 🚀"),
        (&bob, "Looking at the PR."),
    ] {
        update(
            &mut app,
            Event::Net(NetEvent::Frame(ServerFrame::Message {
                message: chat(developers_id, who, text),
            })),
        );
    }
    app.toasts.clear(); // deterministic screens
    app.effects.clear();
    app
}

#[test]
fn the_boot_splash_shows_the_logo_and_progress() {
    let mut app = App::new(ClientConfig::default());
    app.started = Instant::now(); // brand new: splash territory

    let rows = screen(&app, 100, 30);
    let text = flat(&rows);
    assert!(text.contains("██╗"), "block logo visible");
    assert!(text.contains("no system is safe"), "tagline visible");
}

#[test]
fn the_main_screen_lays_out_all_panels() {
    let app = ready_app();
    let rows = screen(&app, 120, 32);
    let text = flat(&rows);

    // Header.
    assert!(text.contains("TERESA"));
    assert!(text.contains('✦'), "the mark is in the header");
    assert!(text.contains("zion"));
    assert!(text.contains("#developers"));
    assert!(text.contains("api work"), "topic in the header");
    assert!(text.contains("Neo"), "identity in the header");

    // Rooms panel.
    assert!(text.contains("CHANNELS"));

    // Messages with rendered markdown (bold survives as text) and emoji.
    assert!(text.contains("Alice"));
    assert!(text.contains("Finished the API 🚀"));
    assert!(text.contains("Looking at the PR."));

    // Members panel with status glyphs.
    assert!(text.contains("ONLINE"));
    assert!(text.contains('▶'), "coding glyph for alice");
    assert!(text.contains('◐'), "away glyph for bob");

    // Status bar + input.
    assert!(text.contains("LINK:SECURE"));
    assert!(text.contains('❯'), "prompt glyph");
    assert!(text.contains("F1 help"));
}

#[test]
fn narrow_terminals_collapse_the_side_panels() {
    let app = ready_app();
    let text = flat(&screen(&app, 60, 20));

    // Chat still works; the member panel is gone rather than crushed.
    assert!(text.contains("Looking at the PR."));
    assert!(!text.contains("ONLINE —"));
}

#[test]
fn every_theme_renders_every_screen_without_panicking() {
    for theme in [
        "matrix",
        "cyberpunk",
        "whoami",
        "fsociety",
        "amber",
        "ghost",
        "dracula",
        "nord",
        "gruvbox",
        "catppuccin",
    ] {
        let mut app = ready_app();
        app.theme = teresa_client::ui::theme::Theme::by_name(theme);
        let _ = screen(&app, 120, 32);
        let _ = screen(&app, 80, 24);
        let _ = screen(&app, 20, 6); // hostile smallness

        // And the indexed (256-colour) variant of each.
        app.theme = app.theme.to_indexed();
        let _ = screen(&app, 80, 24);
    }
}

#[test]
fn unread_and_mention_badges_appear_in_the_rooms_panel() {
    let mut app = ready_app();
    let side = room("ops");
    let side_id = side.id;
    update(
        &mut app,
        Event::Net(NetEvent::Frame(ServerFrame::Joined {
            room: JoinedRoom {
                room: side,
                role: MemberRole::Member,
                unread: 0,
                peer: None,
            },
            members: vec![],
        })),
    );
    // A mention lands in the background room.
    let alice = user("Alice", UserStatus::Online);
    update(
        &mut app,
        Event::Net(NetEvent::Frame(ServerFrame::Message {
            message: chat(side_id, &alice, "hey @neo look at this"),
        })),
    );

    let text = flat(&screen(&app, 120, 32));
    assert!(text.contains("#ops"), "background room listed");
    assert!(text.contains("@1"), "mention badge shown");
}

#[test]
fn the_typing_indicator_line_appears() {
    let mut app = ready_app();
    let room_id = app.active.expect("active room");
    let alice_id = app.rooms[&room_id].members[0].id;
    update(
        &mut app,
        Event::Net(NetEvent::Frame(ServerFrame::Typing {
            room: room_id,
            user: alice_id,
            display_name: "Alice".to_owned(),
            active: true,
        })),
    );

    let text = flat(&screen(&app, 120, 32));
    assert!(text.contains("Alice is typing"), "got: (omitted)");
}

#[test]
fn overlays_render_over_the_app() {
    let mut app = ready_app();

    app.overlay = Some(Overlay::Help);
    let text = flat(&screen(&app, 120, 40));
    assert!(text.contains("MANUAL"));
    assert!(text.contains("/join <room>"));
    assert!(text.contains("Ctrl+W"));

    app.overlay = Some(Overlay::Profile);
    let text = flat(&screen(&app, 120, 32));
    assert!(text.contains("OPERATIVE FILE"));
    assert!(text.contains("@neo"));

    app.overlay = Some(Overlay::RoomSwitcher {
        filter: String::new(),
        selected: 0,
    });
    let text = flat(&screen(&app, 120, 32));
    assert!(text.contains("SWITCHBOARD"));
    assert!(text.contains("#developers"));

    app.overlay = Some(Overlay::ConfirmQuit);
    let text = flat(&screen(&app, 120, 32));
    assert!(text.contains("cut the line?"));
}

#[test]
fn the_emoji_picker_draws_its_grid_and_survives_a_narrow_terminal() {
    let mut app = ready_app();

    app.overlay = Some(Overlay::EmojiPicker {
        filter: "fire".to_owned(),
        selected: 0,
    });
    let rows = screen(&app, 120, 32);
    let text = flat(&rows);
    assert!(text.contains("EMOJI"));
    assert!(text.contains("🔥"), "the searched glyph is on screen");
    assert!(text.contains(":fire:"), "the selected entry names itself");
    assert!(text.contains("fire"), "keywords explain the match");

    // Every row must still be exactly the terminal width: a mis-measured
    // emoji cell would shear the grid and this is what catches it.
    for row in &rows {
        assert_eq!(
            row.chars().count(),
            120,
            "a row changed width — check the cell padding maths"
        );
    }

    // A filter matching nothing says so rather than drawing an empty box.
    app.overlay = Some(Overlay::EmojiPicker {
        filter: "zzzznotathing".to_owned(),
        selected: 0,
    });
    let text = flat(&screen(&app, 120, 32));
    assert!(text.contains("no emoji by that name"));

    // Narrow terminals clamp the modal instead of panicking.
    app.overlay = Some(Overlay::EmojiPicker {
        filter: String::new(),
        selected: 0,
    });
    let text = flat(&screen(&app, 40, 14));
    assert!(text.contains("EMOJI"));
}

#[test]
fn the_connection_veil_reports_reconnection() {
    let mut app = ready_app();
    update(
        &mut app,
        Event::Net(NetEvent::Disconnected {
            reason: "connection lost".to_owned(),
            retry_in: Duration::from_secs(5),
        }),
    );

    let text = flat(&screen(&app, 120, 32));
    assert!(text.contains("LINK LOST"));
    assert!(text.contains("connection lost"));
    assert!(text.contains("Ctrl+R"));
}

#[test]
fn myanmar_text_renders_in_messages_and_input() {
    let mut app = ready_app();
    let room_id = app.active.expect("active room");
    let alice = user("Alice", UserStatus::Online);
    update(
        &mut app,
        Event::Net(NetEvent::Frame(ServerFrame::Message {
            message: chat(room_id, &alice, "မင်္ဂလာပါ **အားလုံး** 🇲🇲"),
        })),
    );
    for c in "ကောင်းတယ်".chars() {
        update(
            &mut app,
            Event::Key(ratatui::crossterm::event::KeyEvent::from(
                ratatui::crossterm::event::KeyCode::Char(c),
            )),
        );
    }

    let text = flat(&screen(&app, 120, 32));
    assert!(
        text.contains('မ'),
        "burmese message text survives rendering"
    );
    assert!(text.contains("အားလုံး"), "markdown bold keeps burmese intact");
    assert!(text.contains('က'), "burmese input echoes in the prompt");
}

#[test]
fn a_shot_flashes_on_the_target_in_the_member_panel() {
    let mut app = ready_app();
    let room_id = app.active.expect("active room");
    let alice = app.rooms[&room_id].members[0].clone();
    let bob = app.rooms[&room_id].members[1].clone();

    let shot = teresa_client::app::commands::action("shoot")
        .expect("shoot exists")
        .body(&bob.username);
    update(
        &mut app,
        Event::Net(NetEvent::Frame(ServerFrame::Message {
            message: Message {
                kind: MessageKind::Emote,
                ..chat(room_id, &alice, &shot)
            },
        })),
    );

    let text = flat(&screen(&app, 120, 32));
    assert!(text.contains("🔫"), "the action line is in the scrollback");
    // Once in the emote body ("headshot 💥") and once next to bob's name.
    assert!(
        text.matches("💥").count() >= 2,
        "the hit glyph flashes in the member panel"
    );
}

#[test]
fn code_blocks_render_with_gutter_and_language_tag() {
    let mut app = ready_app();
    let room_id = app.active.expect("active room");
    let alice = user("Alice", UserStatus::Online);
    update(
        &mut app,
        Event::Net(NetEvent::Frame(ServerFrame::Message {
            message: chat(room_id, &alice, "```python\nprint(\"hello\")\n```"),
        })),
    );

    let text = flat(&screen(&app, 120, 36));
    assert!(text.contains("⌈python⌉"), "language tag");
    assert!(text.contains("print"), "code text survives");
    assert!(text.contains('▌'), "gutter drawn");
}

#[test]
fn typed_input_appears_in_the_prompt_and_submits() {
    let mut app = ready_app();
    for c in "hello world".chars() {
        update(
            &mut app,
            Event::Key(ratatui::crossterm::event::KeyEvent::from(
                ratatui::crossterm::event::KeyCode::Char(c),
            )),
        );
    }
    let text = flat(&screen(&app, 120, 32));
    assert!(text.contains("hello world"), "input echoed in the prompt");

    update(
        &mut app,
        Event::Key(ratatui::crossterm::event::KeyEvent::from(
            ratatui::crossterm::event::KeyCode::Enter,
        )),
    );
    // The optimistic echo renders with the sending spinner.
    let text = flat(&screen(&app, 120, 32));
    assert!(text.contains("hello world"), "pending echo visible");
}
