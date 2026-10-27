use std::path::{Component, Path};

/// Maximum accepted note-id length in bytes. The on-disk record name is
/// `<note_id>.record`, and `atomic_write` creates a sibling temp file
/// `<note_id>.record.tmp-<pid>-<nanos>` (~40 extra bytes); 128 keeps both
/// well under the common 255-byte filename limit on every target filesystem.
pub(crate) const MAX_NOTE_ID_LEN: usize = 128;

/// Windows reserved device names. A file whose name, up to the first `.`,
/// matches one of these (case-insensitively — and every accepted id is
/// lowercase) refers to a device, not a file, on Windows.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8",
    "lpt9",
];

/// A validated note identifier: safe to join onto a vault root without any
/// risk of path traversal, and **the one canonical identity of a note**
/// across every filesystem this project supports (Windows and Unix-like,
/// including case-insensitive ones). The only way to construct one is
/// [`NoteId::parse`].
///
/// Canonical form: `[a-z0-9._-]+`, lowercase ASCII only, no leading or
/// trailing `.`, no `..`, not a Windows reserved device name, at most
/// 128 bytes (`MAX_NOTE_ID_LEN`). `parse` *validates*, it never normalizes: an
/// accepted id is stored byte-for-byte as given, and anything that is not
/// already canonical (e.g. `"NOTE-A"`) is rejected rather than folded. So
/// two distinct `NoteId` values can never name the same file, even on a
/// case-insensitive filesystem, and every identity comparison in `storage`
/// (lock-map key, link lookups, record path) uses this one value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NoteId(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoteIdError {
    #[error("note id must not be empty")]
    Empty,
    #[error("note id must not contain a path separator")]
    ContainsPathSeparator,
    #[error("note id must not contain a parent-directory reference ('..')")]
    ContainsParentRef,
    #[error("note id must be a relative path")]
    NotRelative,
    /// Uppercase ASCII is rejected, not lowercased: on a case-insensitive
    /// filesystem it would alias the lowercase id's record file.
    #[error("note id must be lowercase: found uppercase character {found:?}")]
    MustBeLowercase { found: char },
    #[error("note id may only contain [a-z0-9._-]: found {found:?}")]
    InvalidCharacter { found: char },
    #[error("note id must not start or end with '.'")]
    LeadingOrTrailingDot,
    #[error("note id {name:?} is a reserved device name on Windows")]
    ReservedName { name: String },
    #[error("note id is {actual} bytes long, maximum is {max}")]
    TooLong { max: usize, actual: usize },
}

impl NoteId {
    /// The only constructor for `NoteId`. Accepts exactly the canonical form
    /// described on [`NoteId`] and rejects everything else with a specific
    /// [`NoteIdError`] naming the reason. Never normalizes its input.
    pub fn parse(raw: &str) -> Result<Self, NoteIdError> {
        if raw.is_empty() {
            return Err(NoteIdError::Empty);
        }
        if raw.len() > MAX_NOTE_ID_LEN {
            return Err(NoteIdError::TooLong {
                max: MAX_NOTE_ID_LEN,
                actual: raw.len(),
            });
        }

        // Scan path components first so a `..` or an absolute/rooted path is
        // reported with its specific variant even when the raw string also
        // contains a `/` or `\` (e.g. "../../etc/passwd", "/etc/passwd").
        for component in Path::new(raw).components() {
            match component {
                Component::ParentDir => return Err(NoteIdError::ContainsParentRef),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(NoteIdError::NotRelative)
                }
                Component::CurDir | Component::Normal(_) => {}
            }
        }

        if raw.contains('/') || raw.contains('\\') {
            return Err(NoteIdError::ContainsPathSeparator);
        }
        if raw.contains("..") {
            return Err(NoteIdError::ContainsParentRef);
        }

        for c in raw.chars() {
            if c.is_ascii_uppercase() {
                return Err(NoteIdError::MustBeLowercase { found: c });
            }
            if !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')) {
                return Err(NoteIdError::InvalidCharacter { found: c });
            }
        }

        if raw.starts_with('.') || raw.ends_with('.') {
            return Err(NoteIdError::LeadingOrTrailingDot);
        }

        let stem = raw.split('.').next().unwrap_or(raw);
        if WINDOWS_RESERVED_NAMES.contains(&stem) {
            return Err(NoteIdError::ReservedName {
                name: raw.to_string(),
            });
        }

        Ok(NoteId(raw.to_string()))
    }

    /// The canonical string form. This is the single representation used for
    /// the record file name, the per-note lock key, and link lookups.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{NoteId, NoteIdError};

    #[test]
    fn valid_note_id_is_accepted() {
        assert!(NoteId::parse("bug-2026-10-23-fastapi-cors").is_ok());
    }

    #[test]
    fn empty_note_id_is_rejected() {
        assert!(matches!(NoteId::parse(""), Err(NoteIdError::Empty)));
    }

    #[test]
    fn note_id_with_parent_dir_component_is_rejected() {
        assert!(matches!(
            NoteId::parse("../../etc/passwd"),
            Err(NoteIdError::ContainsParentRef)
        ));
    }

    #[test]
    fn note_id_with_path_separator_is_rejected() {
        assert!(matches!(
            NoteId::parse("sub/dir-note"),
            Err(NoteIdError::ContainsPathSeparator)
        ));
        assert!(matches!(
            NoteId::parse("sub\\dir-note"),
            Err(NoteIdError::ContainsPathSeparator)
        ));
    }

    #[test]
    fn absolute_path_as_note_id_is_rejected() {
        assert!(matches!(
            NoteId::parse("/etc/passwd"),
            Err(NoteIdError::NotRelative)
        ));
    }

    // --- A1 final review, Finding C2: one canonical, filesystem-safe identity ---

    #[test]
    fn uppercase_note_id_is_rejected_not_lowercased_or_accepted() {
        // On case-insensitive filesystems (Windows, default macOS) "NOTE-A"
        // and "note-a" are the same file. Accepting both silently aliased
        // two ids onto one record (reproduced data loss). Uppercase must be
        // a validation error, never silently folded to lowercase.
        assert_eq!(
            NoteId::parse("NOTE-A"),
            Err(NoteIdError::MustBeLowercase { found: 'N' })
        );
    }

    #[test]
    fn mixed_case_variants_of_a_valid_id_are_rejected() {
        assert_eq!(
            NoteId::parse("Note-A"),
            Err(NoteIdError::MustBeLowercase { found: 'N' })
        );
        assert_eq!(
            NoteId::parse("note-A"),
            Err(NoteIdError::MustBeLowercase { found: 'A' })
        );
        assert_eq!(
            NoteId::parse("bug-2026-10-23-FastAPI-cors"),
            Err(NoteIdError::MustBeLowercase { found: 'F' })
        );
    }

    #[test]
    fn accepted_id_is_returned_byte_for_byte_with_no_normalization() {
        for raw in ["note-a", "bug-2026-10-23-fastapi-cors", "release-1.2.3", "a_b", "0"] {
            assert_eq!(NoteId::parse(raw).unwrap().as_str(), raw);
        }
    }

    #[test]
    fn whitespace_anywhere_in_note_id_is_rejected() {
        assert_eq!(
            NoteId::parse(" note-a"),
            Err(NoteIdError::InvalidCharacter { found: ' ' })
        );
        assert_eq!(
            NoteId::parse("note-a "),
            Err(NoteIdError::InvalidCharacter { found: ' ' })
        );
        assert_eq!(
            NoteId::parse("note a"),
            Err(NoteIdError::InvalidCharacter { found: ' ' })
        );
        assert_eq!(
            NoteId::parse("note-a\t"),
            Err(NoteIdError::InvalidCharacter { found: '\t' })
        );
    }

    #[test]
    fn characters_outside_lowercase_ascii_charset_are_rejected() {
        // Non-ASCII (including letters whose case folding is
        // filesystem-dependent) and Windows-reserved punctuation.
        for (raw, bad) in [
            ("n\u{f6}te", '\u{f6}'),
            ("\u{c4}", '\u{c4}'),
            ("note*", '*'),
            ("note?", '?'),
            ("note|x", '|'),
            ("note\"x", '"'),
            ("note<x", '<'),
            ("note\0x", '\0'),
            ("note+x", '+'),
        ] {
            assert_eq!(
                NoteId::parse(raw),
                Err(NoteIdError::InvalidCharacter { found: bad }),
                "input {raw:?}"
            );
        }
    }

    #[test]
    fn leading_or_trailing_dot_is_rejected() {
        // Windows silently strips trailing dots, so "trail." would alias
        // "trail"; a leading dot makes a hidden file / the "." entry.
        for raw in ["trail.", ".hidden", "."] {
            assert_eq!(
                NoteId::parse(raw),
                Err(NoteIdError::LeadingOrTrailingDot),
                "input {raw:?}"
            );
        }
    }

    #[test]
    fn double_dot_anywhere_is_rejected_as_parent_ref() {
        assert_eq!(NoteId::parse(".."), Err(NoteIdError::ContainsParentRef));
        assert_eq!(NoteId::parse("a..b"), Err(NoteIdError::ContainsParentRef));
    }

    #[test]
    fn windows_reserved_device_names_are_rejected_with_or_without_extension() {
        for raw in ["nul", "con", "prn", "aux", "com1", "com0", "lpt9", "nul.txt", "con.2026"] {
            assert!(
                matches!(NoteId::parse(raw), Err(NoteIdError::ReservedName { .. })),
                "input {raw:?} must be rejected as a reserved device name"
            );
        }
        // Not reserved: the reserved word is only a prefix of the stem.
        assert!(NoteId::parse("null").is_ok());
        assert!(NoteId::parse("console-log").is_ok());
        assert!(NoteId::parse("com10").is_ok());
    }

    #[test]
    fn overlong_note_id_is_rejected_and_max_length_is_accepted() {
        let max = "a".repeat(super::MAX_NOTE_ID_LEN);
        assert!(NoteId::parse(&max).is_ok());
        let over = "a".repeat(super::MAX_NOTE_ID_LEN + 1);
        assert_eq!(
            NoteId::parse(&over),
            Err(NoteIdError::TooLong {
                max: super::MAX_NOTE_ID_LEN,
                actual: super::MAX_NOTE_ID_LEN + 1
            })
        );
    }
}
