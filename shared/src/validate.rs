//! Input rules shared by both sides of the wire.
//!
//! The client validates so it can show a red field before a round trip; the
//! server validates because a client is not trustworthy. Both call the same
//! functions here, so the two can never disagree about what a legal username
//! is.
//!
//! Length limits count **characters**, not bytes. A limit expressed in bytes
//! would let a name of eight emoji fail while eight ASCII letters pass, which
//! is not a rule anyone can explain to a user.

use crate::error::ProtoError;

/// Inclusive length bounds for a username.
pub const USERNAME_LEN: (usize, usize) = (2, 32);
/// Inclusive length bounds for a display name.
pub const DISPLAY_NAME_LEN: (usize, usize) = (1, 48);
/// Inclusive length bounds for a password, in characters.
pub const PASSWORD_LEN: (usize, usize) = (8, 256);
/// Inclusive length bounds for a room name, excluding any leading `#`.
pub const ROOM_NAME_LEN: (usize, usize) = (2, 32);
/// Maximum length of a room topic.
pub const TOPIC_MAX: usize = 200;
/// Maximum length of a single message body.
pub const MESSAGE_MAX: usize = 4096;
/// Maximum length of a status text set via `/status`.
pub const STATUS_TEXT_MAX: usize = 64;

/// Normalises and validates a username.
///
/// Usernames are case-insensitive, so they are folded to lowercase here and
/// stored that way. `Alice` and `alice` are the same account; allowing both to
/// exist is a phishing vector, not a feature.
///
/// # Errors
/// Returns [`ProtoError::Length`] or [`ProtoError::Charset`] if the trimmed,
/// lowercased input is not 2–32 characters of `a-z`, `0-9`, `_`, `-`, `.`
/// starting with an alphanumeric.
pub fn username(raw: &str) -> Result<String, ProtoError> {
    let normalised = raw.trim().to_lowercase();
    check_len("username", &normalised, USERNAME_LEN)?;

    // Enforced separately from the body rule so that `_alice` and `-alice`
    // cannot be minted to impersonate `alice` in a sorted member list.
    if !normalised
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
    {
        return Err(ProtoError::Charset {
            field: "username",
            reason: "must start with a letter or digit",
        });
    }
    if !normalised
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(ProtoError::Charset {
            field: "username",
            reason: "may contain only a-z, 0-9, underscore, hyphen and dot",
        });
    }

    Ok(normalised)
}

/// Normalises and validates a display name.
///
/// Unlike a username this may contain spaces, punctuation and non-ASCII text —
/// it is a label, not an identifier. Control characters are rejected because a
/// display name is rendered directly into the terminal, and an embedded escape
/// sequence would let one user repaint another user's screen.
///
/// # Errors
/// Returns [`ProtoError::Length`] or [`ProtoError::Charset`] on a name outside
/// 1–48 characters or containing control characters.
pub fn display_name(raw: &str) -> Result<String, ProtoError> {
    let normalised = raw.trim().to_owned();
    check_len("display name", &normalised, DISPLAY_NAME_LEN)?;
    reject_control_chars("display name", &normalised)?;
    Ok(normalised)
}

/// Validates a password.
///
/// Deliberately permissive about content and strict only about length:
/// composition rules ("one uppercase, one symbol") measurably push people
/// toward `Password1!` and away from passphrases. Argon2id handles the rest.
///
/// The password is never trimmed — leading and trailing spaces are legitimate
/// characters, and silently stripping them would break password managers.
///
/// # Errors
/// Returns [`ProtoError::Length`] outside 8–256 characters, or
/// [`ProtoError::Charset`] if it contains control characters.
pub fn password(raw: &str) -> Result<(), ProtoError> {
    check_len("password", raw, PASSWORD_LEN)?;
    reject_control_chars("password", raw)?;
    Ok(())
}

/// Normalises and validates a room name.
///
/// A leading `#` is accepted and stripped, so `/join #backend` and
/// `/join backend` mean the same thing.
///
/// # Errors
/// Returns [`ProtoError::Length`] or [`ProtoError::Charset`] if the result is
/// not 2–32 characters of `a-z`, `0-9`, `_`, `-`.
pub fn room_name(raw: &str) -> Result<String, ProtoError> {
    let normalised = raw.trim().trim_start_matches('#').to_lowercase();
    check_len("room name", &normalised, ROOM_NAME_LEN)?;

    if !normalised
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err(ProtoError::Charset {
            field: "room name",
            reason: "may contain only a-z, 0-9, underscore and hyphen",
        });
    }
    // `dm:` is how direct-message rooms are named internally. A channel by that
    // name could otherwise collide with, or be mistaken for, a private
    // conversation.
    if normalised.starts_with("dm-") || normalised.starts_with("dm_") {
        return Err(ProtoError::Charset {
            field: "room name",
            reason: "the dm- prefix is reserved for direct messages",
        });
    }

    Ok(normalised)
}

/// Normalises and validates a room topic. An all-whitespace topic becomes `None`.
///
/// # Errors
/// Returns [`ProtoError::Length`] above 200 characters, or
/// [`ProtoError::Charset`] if it contains control characters.
pub fn topic(raw: &str) -> Result<Option<String>, ProtoError> {
    let normalised = raw.trim();
    if normalised.is_empty() {
        return Ok(None);
    }
    check_len("topic", normalised, (1, TOPIC_MAX))?;
    reject_control_chars("topic", normalised)?;
    Ok(Some(normalised.to_owned()))
}

/// Validates a message body.
///
/// Newlines and tabs survive — multi-line messages and pasted code are the
/// point — but other control characters are stripped at the edges and rejected
/// within, so no one can inject terminal escapes into someone else's session.
///
/// # Errors
/// Returns [`ProtoError::Empty`] for a blank body, [`ProtoError::Length`] above
/// 4096 characters, or [`ProtoError::Charset`] for embedded control characters.
pub fn message_body(raw: &str) -> Result<String, ProtoError> {
    // Trailing blank lines are an artefact of pasting, never intent.
    let normalised = raw.trim_end();
    if normalised.trim().is_empty() {
        return Err(ProtoError::Empty { field: "message" });
    }
    let len = normalised.chars().count();
    if len > MESSAGE_MAX {
        return Err(ProtoError::Length {
            field: "message",
            min: 1,
            max: MESSAGE_MAX,
            actual: len,
        });
    }
    if normalised
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(ProtoError::Charset {
            field: "message",
            reason: "control characters other than newline and tab are not allowed",
        });
    }
    Ok(normalised.to_owned())
}

/// Validates the free-text half of a user status.
///
/// # Errors
/// Returns [`ProtoError::Length`] above 64 characters, or
/// [`ProtoError::Charset`] if it contains control characters.
pub fn status_text(raw: &str) -> Result<Option<String>, ProtoError> {
    let normalised = raw.trim();
    if normalised.is_empty() {
        return Ok(None);
    }
    check_len("status", normalised, (1, STATUS_TEXT_MAX))?;
    reject_control_chars("status", normalised)?;
    Ok(Some(normalised.to_owned()))
}

/// Character-count bounds check shared by every rule above.
fn check_len(
    field: &'static str,
    value: &str,
    (min, max): (usize, usize),
) -> Result<(), ProtoError> {
    let actual = value.chars().count();
    if actual < min || actual > max {
        return Err(ProtoError::Length {
            field,
            min,
            max,
            actual,
        });
    }
    Ok(())
}

/// Rejects any control character, including the escape that starts an ANSI
/// sequence. This is the single most important rule in this module: everything
/// it guards is rendered straight into a terminal.
fn reject_control_chars(field: &'static str, value: &str) -> Result<(), ProtoError> {
    if value.chars().any(char::is_control) {
        return Err(ProtoError::Charset {
            field,
            reason: "control characters are not allowed",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_fold_to_lowercase() {
        assert_eq!(username("  Neo  ").unwrap(), "neo");
        assert_eq!(username("ALICE").unwrap(), "alice");
    }

    #[test]
    fn usernames_must_start_alphanumeric() {
        // Otherwise `_alice` sorts next to `alice` and reads as the same person.
        assert!(matches!(
            username("_alice"),
            Err(ProtoError::Charset { .. })
        ));
        assert!(username("a1ice").is_ok());
        assert!(username("0day").is_ok());
    }

    #[test]
    fn usernames_reject_spaces_and_symbols() {
        assert!(username("ne o").is_err());
        assert!(username("ne@o").is_err());
        assert!(username("ne/o").is_err());
    }

    #[test]
    fn usernames_enforce_length_bounds() {
        assert!(matches!(username("a"), Err(ProtoError::Length { .. })));
        assert!(username(&"a".repeat(32)).is_ok());
        assert!(matches!(
            username(&"a".repeat(33)),
            Err(ProtoError::Length { .. })
        ));
    }

    #[test]
    fn display_names_keep_case_and_unicode() {
        assert_eq!(display_name("  Neo Anderson ").unwrap(), "Neo Anderson");
        assert_eq!(display_name("Алиса").unwrap(), "Алиса");
        assert_eq!(display_name("🐧 penguin").unwrap(), "🐧 penguin");
    }

    #[test]
    fn display_names_reject_terminal_escapes() {
        // A name of "\x1b[2J" would clear every other user's screen.
        assert!(matches!(
            display_name("evil\x1b[2Jname"),
            Err(ProtoError::Charset { .. })
        ));
    }

    #[test]
    fn passwords_are_not_trimmed() {
        // A password manager may legitimately generate leading whitespace.
        assert!(password("  spaced  ").is_ok());
    }

    #[test]
    fn passwords_enforce_a_floor_but_no_composition_rules() {
        assert!(matches!(password("short"), Err(ProtoError::Length { .. })));
        assert!(password("correct horse battery staple").is_ok());
        assert!(password("00000000").is_ok());
    }

    #[test]
    fn passwords_count_characters_not_bytes() {
        // Eight emoji is eight characters, even though it is 32 bytes.
        assert!(password("🔥🚀💻🐧⚡🔥🚀💻").is_ok());
    }

    #[test]
    fn room_names_strip_the_hash() {
        assert_eq!(room_name("#Backend").unwrap(), "backend");
        assert_eq!(room_name("backend").unwrap(), "backend");
    }

    #[test]
    fn room_names_reserve_the_dm_prefix() {
        assert!(room_name("dm-alice").is_err());
        assert!(room_name("dm_alice").is_err());
        // A room that merely begins with those two letters is fine.
        assert!(room_name("dmarc").is_ok());
    }

    #[test]
    fn topics_collapse_blank_to_none() {
        assert_eq!(topic("   ").unwrap(), None);
        assert_eq!(topic(" ship it ").unwrap(), Some("ship it".into()));
        assert!(topic(&"x".repeat(TOPIC_MAX + 1)).is_err());
    }

    #[test]
    fn messages_keep_newlines_and_tabs() {
        let body = message_body("fn main() {\n\tprintln!(\"hi\");\n}").unwrap();
        assert!(body.contains('\n') && body.contains('\t'));
    }

    #[test]
    fn messages_trim_only_trailing_whitespace() {
        // Leading indentation is meaningful when pasting code.
        assert_eq!(message_body("    indented\n\n\n").unwrap(), "    indented");
    }

    #[test]
    fn messages_reject_blank_and_oversized_bodies() {
        assert!(matches!(
            message_body("   \n "),
            Err(ProtoError::Empty { .. })
        ));
        assert!(message_body(&"x".repeat(MESSAGE_MAX)).is_ok());
        assert!(matches!(
            message_body(&"x".repeat(MESSAGE_MAX + 1)),
            Err(ProtoError::Length { .. })
        ));
    }

    #[test]
    fn messages_reject_embedded_escapes() {
        assert!(matches!(
            message_body("nice \x1b]0;pwned\x07 try"),
            Err(ProtoError::Charset { .. })
        ));
    }

    #[test]
    fn status_text_collapses_blank_to_none() {
        assert_eq!(status_text("  ").unwrap(), None);
        assert_eq!(
            status_text(" deep in a bug ").unwrap(),
            Some("deep in a bug".into())
        );
    }
}
