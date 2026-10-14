//! Shared files.
//!
//! A file is two things: the bytes, which travel over plain HTTP so they can
//! stream at wire speed without base64 or a 4 KiB frame ceiling, and a
//! [`FileCard`], which travels as an ordinary message of kind
//! [`crate::MessageKind::File`] so that history, unread badges and fan-out
//! need no second code path. The card is the message body, as JSON.

use serde::{Deserialize, Serialize};

use crate::error::ProtoError;
use crate::ids::FileId;

/// Longest file name accepted, in characters.
pub const FILE_NAME_MAX: usize = 255;

/// What the sender shared — which decides what a receiver does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// A single file, sent as-is.
    #[default]
    File,
    /// A directory, packed into a zip on the way out and unpacked on the way in.
    Folder,
    /// A single file the sender chose to compress (`/zip`); kept as a `.zip`.
    Zip,
}

impl FileKind {
    /// The lowercase wire name, also used as the storage form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
            Self::Zip => "zip",
        }
    }

    /// The badge drawn on the card.
    #[must_use]
    pub const fn badge(self) -> &'static str {
        match self {
            Self::File => "FILE",
            Self::Folder => "DIR·ZIP",
            Self::Zip => "ZIP",
        }
    }
}

impl std::str::FromStr for FileKind {
    type Err = ProtoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "file" => Ok(Self::File),
            "folder" => Ok(Self::Folder),
            "zip" => Ok(Self::Zip),
            _ => Err(ProtoError::Charset {
                field: "file kind",
                reason: "expected one of: file, folder, zip",
            }),
        }
    }
}

/// Everything a receiver needs to show and fetch a shared file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCard {
    /// Download handle: `GET /api/v1/files/{id}`.
    pub id: FileId,
    /// Name as stored — for a folder, the folder's name plus `.zip`.
    pub name: String,
    /// Exact size of the stored bytes.
    pub size: u64,
    /// Lowercase hex SHA-256 of the stored bytes, computed by the server as
    /// they arrived; receivers verify against it.
    pub sha256: String,
    /// File, folder or archive.
    pub kind: FileKind,
    /// For folders: how many files were packed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<u32>,
}

impl FileCard {
    /// The message body that carries this card.
    ///
    /// # Panics
    /// Never: the card is plain strings and integers.
    #[must_use]
    pub fn to_body(&self) -> String {
        serde_json::to_string(self).expect("file card serialises")
    }

    /// Reads a card back out of a [`crate::MessageKind::File`] body.
    ///
    /// # Errors
    /// [`ProtoError::Decode`] if the body is not a card.
    pub fn from_body(body: &str) -> Result<Self, ProtoError> {
        serde_json::from_str(body).map_err(|e| ProtoError::Decode(e.to_string()))
    }

    /// The name a receiver should end up with on disk: folders lose the
    /// `.zip` they gained in transit.
    #[must_use]
    pub fn landing_name(&self) -> &str {
        match self.kind {
            FileKind::Folder => self.name.strip_suffix(".zip").unwrap_or(&self.name),
            FileKind::File | FileKind::Zip => &self.name,
        }
    }
}

/// Human byte counts in binary units: `512 B`, `48.2 MiB`.
#[must_use]
#[allow(clippy::cast_precision_loss)] // display only; 3 significant digits
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Validates a file name as a single, inert path component.
///
/// The name ends up on a receiver's disk, so anything that could climb out of
/// the download directory or confuse a terminal is refused rather than
/// repaired: separators, `.`/`..`, control characters.
///
/// # Errors
/// [`ProtoError::Empty`], [`ProtoError::Length`] or [`ProtoError::Charset`].
pub fn file_name(raw: &str) -> Result<String, ProtoError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ProtoError::Empty { field: "file name" });
    }
    let len = name.chars().count();
    if len > FILE_NAME_MAX {
        return Err(ProtoError::Length {
            field: "file name",
            min: 1,
            max: FILE_NAME_MAX,
            actual: len,
        });
    }
    if name == "." || name == ".." {
        return Err(ProtoError::Charset {
            field: "file name",
            reason: "must name a file, not a directory",
        });
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '\0'))
    {
        return Err(ProtoError::Charset {
            field: "file name",
            reason: "path separators and control characters are not allowed",
        });
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(kind: FileKind, name: &str) -> FileCard {
        FileCard {
            id: FileId::new(),
            name: name.into(),
            size: 50_536_448,
            sha256: "9f".repeat(32),
            kind,
            entries: (kind == FileKind::Folder).then_some(12),
        }
    }

    #[test]
    fn cards_round_trip_through_a_message_body() {
        for kind in [FileKind::File, FileKind::Folder, FileKind::Zip] {
            let card = card(kind, "dump.sql");
            assert_eq!(FileCard::from_body(&card.to_body()).unwrap(), card);
        }
        assert!(FileCard::from_body("just chat").is_err());
    }

    #[test]
    fn folders_land_without_their_transit_zip() {
        assert_eq!(card(FileKind::Folder, "src.zip").landing_name(), "src");
        assert_eq!(
            card(FileKind::Zip, "log.txt.zip").landing_name(),
            "log.txt.zip"
        );
        assert_eq!(card(FileKind::File, "a.zip").landing_name(), "a.zip");
    }

    #[test]
    fn file_kinds_round_trip_through_their_storage_form() {
        for kind in [FileKind::File, FileKind::Folder, FileKind::Zip] {
            assert_eq!(kind.as_str().parse::<FileKind>().unwrap(), kind);
        }
        assert!("tarball".parse::<FileKind>().is_err());
    }

    #[test]
    fn file_names_cannot_escape_the_download_directory() {
        assert_eq!(file_name("  report.pdf ").unwrap(), "report.pdf");
        assert_eq!(file_name("မြန်မာ.txt").unwrap(), "မြန်မာ.txt");
        for bad in [
            "",
            "  ",
            ".",
            "..",
            "../etc/passwd",
            "a/b",
            "a\\b",
            "c:x",
            "a\x1b[2Jb",
        ] {
            assert!(file_name(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(file_name(&"x".repeat(FILE_NAME_MAX + 1)).is_err());
    }

    #[test]
    fn byte_counts_read_like_a_human_wrote_them() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(50_536_448), "48.2 MiB");
        assert_eq!(human_bytes(300 * 1024 * 1024), "300 MiB");
    }
}
