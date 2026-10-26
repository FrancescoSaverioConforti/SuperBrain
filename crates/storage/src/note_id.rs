use std::path::{Component, Path};

/// A validated note identifier: safe to join onto a vault root without any
/// risk of path traversal. The only way to construct one is [`NoteId::parse`],
/// which rejects empty strings, path separators, `..` components, and
/// absolute paths before any [`std::path::Path`] is built from untrusted input.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NoteId(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoteIdError {
    #[error("note id must not be empty")]
    Empty,
    #[error("note id must not contain a path separator")]
    ContainsPathSeparator,
    #[error("note id must not contain a parent-directory reference")]
    ContainsParentRef,
    #[error("note id must be a relative path")]
    NotRelative,
}

impl NoteId {
    /// The only constructor for `NoteId`. Rejects anything that is not a
    /// single, plain relative path segment: empty strings, embedded `/` or
    /// `\`, any `..` component, and any absolute path (including Windows
    /// drive/UNC prefixes).
    pub fn parse(raw: &str) -> Result<Self, NoteIdError> {
        if raw.is_empty() {
            return Err(NoteIdError::Empty);
        }

        // Scan path components first so a `..` or an absolute/rooted path is
        // reported with its specific variant even when the raw string also
        // contains a `/` or `\` (e.g. "../../etc/passwd", "/etc/passwd").
        let mut normal_segments = 0u32;
        for component in Path::new(raw).components() {
            match component {
                Component::ParentDir => return Err(NoteIdError::ContainsParentRef),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(NoteIdError::NotRelative)
                }
                Component::CurDir => {}
                Component::Normal(_) => normal_segments += 1,
            }
        }

        // No `..` and no absolute/rooted prefix: the only remaining way this
        // can be unsafe is more than one plain segment, i.e. an embedded
        // separator joining two or more normal path parts.
        if normal_segments > 1 || raw.contains('/') || raw.contains('\\') {
            return Err(NoteIdError::ContainsPathSeparator);
        }

        Ok(NoteId(raw.to_string()))
    }

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
}
