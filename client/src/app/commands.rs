//! The slash-command language.
//!
//! Parsing is pure — text in, [`Command`] out — so every command's grammar is
//! covered by plain unit tests. What a command *does* lives in `update.rs`.

use teresa_proto::{UserStatus, Visibility};

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
        "clear" | "cls" => Command::Clear,
        "ping" => Command::Ping,
        "version" | "ver" => Command::Version,
        "logout" => Command::Logout,
        "quit" | "exit" | "q" => Command::Quit,
        _ => Command::Unknown(trimmed.to_owned()),
    }
}

/// Commands the completer should offer after a `/`.
pub const COMMAND_NAMES: &[&str] = &[
    "help", "users", "whoami", "rooms", "join", "create", "delete", "invite", "dm", "leave", "me",
    "nick", "status", "theme", "clear", "ping", "version", "logout", "quit",
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
