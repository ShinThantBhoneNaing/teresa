//! The slash-command language.
//!
//! Parsing is pure — text in, [`Command`] out — so every command's grammar is
//! covered by plain unit tests. What a command *does* lives in `update.rs`.

use teresa_proto::{MaintenanceOp, UserStatus, Visibility};

/// Every command a user can type. `Say` is the fallback: anything that does
/// not start with `/` is chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Plain chat text.
    Say(String),
    /// `/help` — the command reference overlay.
    Help,
    /// `/users` — member list of the current room.
    Users,
    /// `/whoami` — your own profile line.
    Whoami,
    /// `/rooms` — list channels on the server.
    Rooms,
    /// `/join <room>`
    Join(String),
    /// `/create <room> [--private|--invite-only] [topic…]`
    Create {
        /// Room name.
        name: String,
        /// Who may see and enter.
        visibility: Visibility,
        /// Optional topic text.
        topic: Option<String>,
    },
    /// `/delete [room]` — delete the current (or named) room.
    Delete(Option<String>),
    /// `/invite <user>`
    Invite(String),
    /// `/dm <user>`
    Dm(String),
    /// `/leave` — leave the current room.
    Leave,
    /// `/me <action>`
    Me(String),
    /// A targeted action from [`ACTIONS`]: `/shoot @user`, `/slap @user`, …
    Action {
        /// The verb, exactly as listed in [`ACTIONS`].
        verb: String,
        /// The target's username, sigil stripped.
        target: String,
    },
    /// `/nick <display name>`
    Nick(String),
    /// `/status <status> [text…]`
    Status {
        /// The presence to set.
        status: UserStatus,
        /// Optional free text.
        text: Option<String>,
    },
    /// `/theme [name]` — switch themes, or list them.
    Theme(Option<String>),
    /// `/send [@user] <path>` (and `/zip …`): share a file or folder here, or
    /// with one person over DM.
    Send {
        /// Recipient for a person-to-person send; `None` = the current room.
        to: Option<String>,
        /// The path as typed; resolved by the reducer.
        path: String,
        /// Compress even a single file (`/zip`). Folders always travel zipped.
        zip: bool,
    },
    /// `/get [n|name] [--to <dir>]` — download a shared file: newest, `#n`,
    /// or by name. Without `--to`, the user is asked where to save it.
    Get {
        /// Which file; `None` = the newest.
        which: Option<String>,
        /// Destination folder, skipping the save prompt.
        to: Option<String>,
    },
    /// `/downloads [dir]` — show or set the default download folder.
    Downloads(Option<String>),
    /// `/files` — list the files shared in this room.
    Files,
    /// `/admin [stats|files [days]|messages [days]|compact] [--yes]` —
    /// storage maintenance on the node (admins only).
    Admin {
        /// What to do.
        op: MaintenanceOp,
        /// `--yes` given: destructive operations may proceed.
        confirmed: bool,
    },
    /// `/admin users` — every account on the node (admins only).
    AdminUsers,
    /// `/admin remove @user [--yes]` — delete an account (admins only).
    AdminRemove {
        /// The account's handle, sigil stripped.
        username: String,
        /// `--yes` given.
        confirmed: bool,
    },
    /// `/cancel [all]` — stop the newest running transfer, or every one.
    Cancel {
        /// Every running transfer, not just the newest.
        all: bool,
    },
    /// `/clear` — clear the current room's scrollback (locally).
    Clear,
    /// `/ping` — measure round-trip time.
    Ping,
    /// `/version` — client and server versions.
    Version,
    /// `/logout` — end the session and quit.
    Logout,
    /// `/quit` (also `/exit`) — leave the app, session intact.
    Quit,
    /// A `/command` nobody recognises; the string is the whole input.
    Unknown(String),
}

/// Parses one submitted line.
#[must_use]
pub fn parse(input: &str) -> Command {
    let trimmed = input.trim();

    let Some(rest) = trimmed.strip_prefix('/') else {
        return Command::Say(input.trim_end().to_owned());
    };

    // "//literal slash" is the escape hatch for sending text that starts
    // with a slash.
    if rest.starts_with('/') {
        return Command::Say(trimmed[1..].trim_end().to_owned());
    }

    let (name, args) = match rest.split_once(char::is_whitespace) {
        Some((name, args)) => (name, args.trim()),
        None => (rest, ""),
    };

    match name.to_lowercase().as_str() {
        "help" | "h" | "?" => Command::Help,
        "users" | "who" | "members" => Command::Users,
        "whoami" => Command::Whoami,
        "rooms" | "channels" | "list" => Command::Rooms,
        "join" | "j" => require_arg(args, |a| Command::Join(a.to_owned())),
        "create" => parse_create(args),
        "delete" => Command::Delete((!args.is_empty()).then(|| args.to_owned())),
        "invite" => require_arg(args, |a| Command::Invite(strip_at(a).to_owned())),
        "dm" | "msg" | "query" => require_arg(args, |a| Command::Dm(strip_at(a).to_owned())),
        "leave" | "part" => Command::Leave,
        "me" => require_arg(args, |a| Command::Me(a.to_owned())),
        "nick" | "name" => require_arg(args, |a| Command::Nick(a.to_owned())),
        "status" => parse_status(args),
        "theme" => Command::Theme((!args.is_empty()).then(|| args.to_lowercase())),
        "send" | "upload" | "share" => parse_send(args, false),
        "zip" => parse_send(args, true),
        "get" | "download" | "pull" => parse_get(args),
        "downloads" | "savedir" => Command::Downloads((!args.is_empty()).then(|| args.to_owned())),
        "files" | "ls" => Command::Files,
        "admin" => parse_admin(args),
        "cancel" | "stop" | "abort" | "reject" => Command::Cancel {
            all: args.trim().eq_ignore_ascii_case("all"),
        },
        "clear" | "cls" => Command::Clear,
        "ping" => Command::Ping,
        "version" | "ver" => Command::Version,
        "logout" => Command::Logout,
        "quit" | "exit" | "q" => Command::Quit,
        other => match action(other) {
            Some(spec) => require_arg(args, |a| Command::Action {
                verb: spec.verb.to_owned(),
                // Only the first word is the target; the rest is noise.
                target: strip_at(a.split_whitespace().next().unwrap_or(a)).to_lowercase(),
            }),
            None => Command::Unknown(trimmed.to_owned()),
        },
    }
}

/// A targeted action: one verb, one victim, one line of theatre.
///
/// Actions are ordinary emotes on the wire — an old client renders them as a
/// plain `/me` line and nothing breaks. The emoji prefix is the signature that
/// lets clients which *do* know the verb play the hit effect on the target.
pub struct ActionSpec {
    /// The `/verb`.
    pub verb: &'static str,
    /// The emoji that opens the emote body — the wire signature.
    pub emoji: &'static str,
    /// The emote text; `{target}` is replaced with `@username`.
    pub template: &'static str,
    /// The glyph flashed on the target's name in the member panel.
    pub hit: &'static str,
}

/// The action roster, in `/help` order.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        verb: "shoot",
        emoji: "🔫",
        template: "shoots {target} down — headshot 💥",
        hit: "💥",
    },
    ActionSpec {
        verb: "slap",
        emoji: "🐟",
        template: "slaps {target} around a bit with a large trout",
        hit: "🐟",
    },
    ActionSpec {
        verb: "hug",
        emoji: "🫂",
        template: "wraps {target} in a warm hug",
        hit: "🫂",
    },
    ActionSpec {
        verb: "highfive",
        emoji: "✋",
        template: "high-fives {target}",
        hit: "✋",
    },
    ActionSpec {
        verb: "hack",
        emoji: "💻",
        template: "breaches {target}'s mainframe — ACCESS GRANTED 🔓",
        hit: "🔓",
    },
    ActionSpec {
        verb: "nuke",
        emoji: "☢",
        template: "drops a tactical nuke on {target} — nothing remains",
        hit: "☢",
    },
    ActionSpec {
        verb: "coffee",
        emoji: "☕",
        template: "slides a fresh coffee over to {target}",
        hit: "☕",
    },
    ActionSpec {
        verb: "fire",
        emoji: "🔥",
        template: "sets {target}'s codebase on fire",
        hit: "🔥",
    },
    ActionSpec {
        verb: "rekt",
        emoji: "💀",
        template: "leaves {target} absolutely rekt",
        hit: "💀",
    },
];

impl ActionSpec {
    /// The emote body sent over the wire.
    #[must_use]
    pub fn body(&self, target: &str) -> String {
        format!(
            "{} {}",
            self.emoji,
            self.template.replace("{target}", &format!("@{target}"))
        )
    }
}

/// Looks a verb up in the roster.
#[must_use]
pub fn action(verb: &str) -> Option<&'static ActionSpec> {
    ACTIONS.iter().find(|spec| spec.verb == verb)
}

/// Recognises an incoming emote as an action and extracts the hit effect:
/// the glyph to flash and the target's username (lowercase, sigil stripped).
#[must_use]
pub fn action_fx(body: &str) -> Option<(&'static str, String)> {
    let spec = ACTIONS.iter().find(|spec| {
        body.strip_prefix(spec.emoji)
            .is_some_and(|r| r.starts_with(' '))
    })?;
    let handle = body.split_whitespace().find_map(|w| w.strip_prefix('@'))?;
    // Keep only legal username characters, so "@bob's" targets "bob".
    let target: String = handle
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .collect::<String>()
        .to_lowercase();
    (!target.is_empty()).then_some((spec.hit, target))
}

/// Commands the completer should offer after a `/`.
pub const COMMAND_NAMES: &[&str] = &[
    "help",
    "users",
    "whoami",
    "rooms",
    "join",
    "create",
    "delete",
    "invite",
    "dm",
    "leave",
    "me",
    "shoot",
    "slap",
    "hug",
    "highfive",
    "hack",
    "nuke",
    "coffee",
    "fire",
    "rekt",
    "nick",
    "status",
    "theme",
    "send",
    "zip",
    "get",
    "downloads",
    "files",
    "admin",
    "cancel",
    "clear",
    "ping",
    "version",
    "logout",
    "quit",
];

fn require_arg(args: &str, build: impl FnOnce(&str) -> Command) -> Command {
    if args.is_empty() {
        // Surfaced as unknown so update() shows usage help for it.
        Command::Unknown(String::new())
    } else {
        build(args)
    }
}

fn strip_at(name: &str) -> &str {
    name.strip_prefix('@').unwrap_or(name)
}

fn parse_create(args: &str) -> Command {
    if args.is_empty() {
        return Command::Unknown(String::new());
    }
    let mut name = None;
    let mut visibility = Visibility::Public;
    let mut topic_words = Vec::new();

    for word in args.split_whitespace() {
        match word {
            "--private" => visibility = Visibility::Private,
            "--invite-only" | "--secret" => visibility = Visibility::InviteOnly,
            other if name.is_none() => name = Some(other.to_owned()),
            other => topic_words.push(other),
        }
    }

    match name {
        Some(name) => Command::Create {
            name,
            visibility,
            topic: (!topic_words.is_empty()).then(|| topic_words.join(" ")),
        },
        None => Command::Unknown(String::new()),
    }
}

fn parse_send(args: &str, zip: bool) -> Command {
    let (to, path) = match args.strip_prefix('@') {
        Some(rest) => match rest.split_once(char::is_whitespace) {
            Some((user, path)) => (Some(user.to_lowercase()), path.trim()),
            None => (Some(rest.to_lowercase()), ""),
        },
        None => (None, args),
    };
    if path.is_empty() {
        return Command::Unknown(String::new());
    }
    Command::Send {
        to,
        path: path.to_owned(),
        zip,
    }
}

fn parse_get(args: &str) -> Command {
    let (which, to) = match args.split_once("--to") {
        Some((which, to)) => (which.trim(), Some(to.trim())),
        None => (args.trim(), None),
    };
    match to {
        Some("") => Command::Unknown(String::new()),
        _ => Command::Get {
            which: (!which.is_empty()).then(|| which.to_owned()),
            to: to.map(str::to_owned),
        },
    }
}

/// Days kept when `/admin files` names no threshold.
pub const DEFAULT_FILE_DAYS: u32 = 30;
/// Days kept when `/admin messages` names no threshold.
pub const DEFAULT_MESSAGE_DAYS: u32 = 90;

fn parse_admin(args: &str) -> Command {
    let mut words: Vec<&str> = args.split_whitespace().collect();
    // Forgiving on purpose: a confirmation typed slightly wrong must not
    // silently turn into "nothing happened".
    let is_yes = |w: &str| {
        matches!(
            w.trim_start_matches('-').to_lowercase().as_str(),
            "yes" | "y" | "confirm"
        )
    };
    let confirmed = words.iter().any(|w| is_yes(w));
    words.retain(|w| !is_yes(w));
    let days = |default| match words.get(1) {
        None => Some(default),
        Some(w) => w.trim_end_matches('d').parse().ok(),
    };
    match words.first().copied() {
        Some("users" | "accounts") => return Command::AdminUsers,
        Some("remove" | "delete" | "rm") => {
            return match words.get(1) {
                Some(user) => Command::AdminRemove {
                    username: strip_at(user).to_lowercase(),
                    confirmed,
                },
                None => Command::Unknown(String::new()),
            };
        }
        _ => {}
    }
    let op = match words.first().copied() {
        None | Some("stats" | "status" | "df") => Some(MaintenanceOp::Stats),
        Some("files") => days(DEFAULT_FILE_DAYS)
            .map(|older_than_days| MaintenanceOp::PurgeFiles { older_than_days }),
        Some("messages" | "history") => days(DEFAULT_MESSAGE_DAYS)
            .map(|older_than_days| MaintenanceOp::PurgeMessages { older_than_days }),
        Some("compact" | "vacuum") => Some(MaintenanceOp::Compact),
        Some(_) => None,
    };
    match op {
        Some(op) => Command::Admin { op, confirmed },
        None => Command::Unknown(format!("/admin {args}")),
    }
}

fn parse_status(args: &str) -> Command {
    let (status_word, text) = match args.split_once(char::is_whitespace) {
        Some((w, rest)) => (w, rest.trim()),
        None => (args, ""),
    };
    match status_word.parse::<UserStatus>() {
        Ok(status) => Command::Status {
            status,
            text: (!text.is_empty()).then(|| text.to_owned()),
        },
        Err(_) => Command::Unknown(format!("/status {args}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_chat() {
        assert_eq!(parse("hello world"), Command::Say("hello world".into()));
        assert_eq!(parse("  spaced  "), Command::Say("  spaced".into()));
    }

    #[test]
    fn double_slash_escapes_a_literal_slash() {
        assert_eq!(parse("//tableflip"), Command::Say("/tableflip".into()));
    }

    #[test]
    fn commands_are_case_insensitive_with_aliases() {
        assert_eq!(parse("/HELP"), Command::Help);
        assert_eq!(parse("/j backend"), Command::Join("backend".into()));
        assert_eq!(parse("/part"), Command::Leave);
        assert_eq!(parse("/exit"), Command::Quit);
        assert_eq!(parse("/?"), Command::Help);
    }

    #[test]
    fn dm_and_invite_strip_the_mention_sigil() {
        assert_eq!(parse("/dm @alice"), Command::Dm("alice".into()));
        assert_eq!(parse("/invite @bob"), Command::Invite("bob".into()));
        assert_eq!(parse("/dm alice"), Command::Dm("alice".into()));
    }

    #[test]
    fn create_understands_visibility_flags_and_topics() {
        assert_eq!(
            parse("/create warroom"),
            Command::Create {
                name: "warroom".into(),
                visibility: Visibility::Public,
                topic: None
            }
        );
        assert_eq!(
            parse("/create vault --invite-only the inner circle"),
            Command::Create {
                name: "vault".into(),
                visibility: Visibility::InviteOnly,
                topic: Some("the inner circle".into())
            }
        );
        assert_eq!(
            parse("/create ops --private"),
            Command::Create {
                name: "ops".into(),
                visibility: Visibility::Private,
                topic: None
            }
        );
    }

    #[test]
    fn status_parses_the_status_then_free_text() {
        assert_eq!(
            parse("/status coding deep in the parser"),
            Command::Status {
                status: UserStatus::Coding,
                text: Some("deep in the parser".into())
            }
        );
        assert_eq!(
            parse("/status afk"),
            Command::Status {
                status: UserStatus::Away,
                text: None
            }
        );
        assert!(matches!(parse("/status napping"), Command::Unknown(_)));
    }

    #[test]
    fn me_keeps_its_text_verbatim() {
        assert_eq!(
            parse("/me fixes the build"),
            Command::Me("fixes the build".into())
        );
    }

    #[test]
    fn missing_required_arguments_surface_as_unknown() {
        assert!(matches!(parse("/join"), Command::Unknown(_)));
        assert!(matches!(parse("/dm"), Command::Unknown(_)));
        assert!(matches!(parse("/create"), Command::Unknown(_)));
        assert!(matches!(parse("/nick"), Command::Unknown(_)));
    }

    #[test]
    fn theme_with_and_without_a_name() {
        assert_eq!(parse("/theme"), Command::Theme(None));
        assert_eq!(
            parse("/theme Matrix"),
            Command::Theme(Some("matrix".into()))
        );
    }

    #[test]
    fn nonsense_commands_are_unknown() {
        assert!(matches!(parse("/frobnicate now"), Command::Unknown(_)));
    }

    #[test]
    fn actions_parse_with_or_without_the_sigil() {
        assert_eq!(
            parse("/shoot @bob"),
            Command::Action {
                verb: "shoot".into(),
                target: "bob".into()
            }
        );
        assert_eq!(
            parse("/slap Bob trailing words ignored"),
            Command::Action {
                verb: "slap".into(),
                target: "bob".into()
            }
        );
        assert!(matches!(parse("/shoot"), Command::Unknown(_)));
    }

    #[test]
    fn action_bodies_carry_the_signature_and_the_mention() {
        let spec = action("shoot").expect("shoot is on the roster");
        let body = spec.body("bob");
        assert!(body.starts_with("🔫 "));
        assert!(body.contains("@bob"));
    }

    #[test]
    fn incoming_action_emotes_yield_a_hit_effect() {
        let body = action("shoot").unwrap().body("bob");
        assert_eq!(action_fx(&body), Some(("💥", "bob".to_owned())));

        // The possessive in the hack template must not leak into the target.
        let body = action("hack").unwrap().body("neo");
        assert_eq!(action_fx(&body), Some(("🔓", "neo".to_owned())));

        // A plain /me emote is not an action.
        assert_eq!(action_fx("fixes the build"), None);
        // The emoji alone, without a target, is not an action either.
        assert_eq!(action_fx("🔫 everything"), None);
    }

    #[test]
    fn every_action_round_trips_through_its_own_body() {
        for spec in ACTIONS {
            let body = spec.body("trinity");
            let (hit, target) = action_fx(&body)
                .unwrap_or_else(|| panic!("/{} body did not detect as an action", spec.verb));
            assert_eq!(hit, spec.hit, "/{}", spec.verb);
            assert_eq!(target, "trinity", "/{}", spec.verb);
        }
    }

    #[test]
    fn send_takes_an_optional_recipient_and_a_path_with_spaces() {
        assert_eq!(
            parse("/send ~/st's project/notes.md"),
            Command::Send {
                to: None,
                path: "~/st's project/notes.md".into(),
                zip: false
            }
        );
        assert_eq!(
            parse("/send @Bob ./dist"),
            Command::Send {
                to: Some("bob".into()),
                path: "./dist".into(),
                zip: false
            }
        );
        assert_eq!(
            parse("/zip @alice dump.sql"),
            Command::Send {
                to: Some("alice".into()),
                path: "dump.sql".into(),
                zip: true
            }
        );
        assert!(matches!(parse("/send"), Command::Unknown(_)));
        assert!(matches!(parse("/send @bob"), Command::Unknown(_)));
    }

    #[test]
    fn admin_subcommands_and_confirmation() {
        assert_eq!(
            parse("/admin"),
            Command::Admin {
                op: MaintenanceOp::Stats,
                confirmed: false
            }
        );
        assert_eq!(
            parse("/admin files"),
            Command::Admin {
                op: MaintenanceOp::PurgeFiles {
                    older_than_days: DEFAULT_FILE_DAYS
                },
                confirmed: false
            }
        );
        assert_eq!(
            parse("/admin messages 180d --yes"),
            Command::Admin {
                op: MaintenanceOp::PurgeMessages {
                    older_than_days: 180
                },
                confirmed: true
            }
        );
        assert_eq!(
            parse("/admin files 0 -y"),
            Command::Admin {
                op: MaintenanceOp::PurgeFiles { older_than_days: 0 },
                confirmed: true
            }
        );
        assert!(matches!(parse("/admin files soon"), Command::Unknown(_)));
        assert_eq!(parse("/admin users"), Command::AdminUsers);
        assert_eq!(
            parse("/admin remove @Bob --yes"),
            Command::AdminRemove {
                username: "bob".into(),
                confirmed: true
            }
        );
        assert!(matches!(parse("/admin remove"), Command::Unknown(_)));
        for yes in ["yes", "-yes", "--YES", "confirm", "-y"] {
            assert_eq!(
                parse(&format!("/admin remove mytx {yes}")),
                Command::AdminRemove {
                    username: "mytx".into(),
                    confirmed: true
                },
                "{yes}"
            );
        }
        assert!(matches!(parse("/admin reboot"), Command::Unknown(_)));
    }

    #[test]
    fn get_and_files() {
        assert_eq!(
            parse("/get"),
            Command::Get {
                which: None,
                to: None
            }
        );
        assert_eq!(
            parse("/get 3"),
            Command::Get {
                which: Some("3".into()),
                to: None
            }
        );
        assert_eq!(
            parse("/get 2 --to ~/st's project/inbox"),
            Command::Get {
                which: Some("2".into()),
                to: Some("~/st's project/inbox".into())
            }
        );
        assert_eq!(
            parse("/get --to ~/Desktop"),
            Command::Get {
                which: None,
                to: Some("~/Desktop".into())
            }
        );
        assert!(matches!(parse("/get 2 --to"), Command::Unknown(_)));
        assert_eq!(parse("/downloads"), Command::Downloads(None));
        assert_eq!(
            parse("/downloads ~/inbox"),
            Command::Downloads(Some("~/inbox".into()))
        );
        assert_eq!(parse("/files"), Command::Files);
    }

    #[test]
    fn every_advertised_command_parses_to_something_known() {
        for name in COMMAND_NAMES {
            let parsed = parse(&format!("/{name} arg"));
            assert!(
                !matches!(parsed, Command::Say(_)),
                "/{name} fell through to chat"
            );
        }
    }
}
