mod links;
mod migration;
mod note;
mod schema;
mod write_state;

pub use links::{classify_links, LinkError, LinkState};
pub use migration::{Migration, MigrationError, MigrationOutcome};
pub use note::{parse_frontmatter, NoteMetadata, Relations};
pub use schema::{Classification, NoteStatus, NoteType, TrustLevel, ValidationError};
pub use write_state::{valid_transition, WriteState};

#[test]
fn crate_builds() { assert!(true); }
