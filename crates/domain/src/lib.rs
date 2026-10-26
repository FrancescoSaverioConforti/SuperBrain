mod links;
mod note;
mod schema;

pub use links::{classify_links, LinkError, LinkState};
pub use note::{parse_frontmatter, NoteMetadata, Relations};
pub use schema::{Classification, NoteStatus, NoteType, TrustLevel, ValidationError};

#[test]
fn crate_builds() { assert!(true); }
