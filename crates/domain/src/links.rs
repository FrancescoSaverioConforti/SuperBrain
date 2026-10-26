use std::collections::HashSet;

use crate::note::{NoteMetadata, Relations};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Unlinked,
    Resolved,
    Pending,
    Broken,
}

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("broken link: relation {relation} points to unknown id {target_id}")]
    BrokenLink { relation: String, target_id: String },
    #[error("self-reference: relation {relation} on note {note_id} points to itself")]
    SelfReference { relation: String, note_id: String },
}

/// Each relation list paired with its field name, for iteration.
fn relation_lists(relations: &Relations) -> [(&'static str, &Vec<String>); 5] {
    [
        ("supports", &relations.supports),
        ("contradicts", &relations.contradicts),
        ("derived_from", &relations.derived_from),
        ("supersedes", &relations.supersedes),
        ("applies_to", &relations.applies_to),
    ]
}

pub fn classify_links(
    note: &NoteMetadata,
    known_ids: &HashSet<String>,
    pending_ids: &HashSet<String>,
) -> Result<LinkState, LinkError> {
    let lists = relation_lists(&note.relations);

    // Self-reference is checked first, before resolved/pending/broken, so a
    // note that both self-references and points at other valid known ids is
    // still rejected rather than silently resolved.
    for (relation, ids) in &lists {
        for target_id in ids.iter() {
            if *target_id == note.id {
                return Err(LinkError::SelfReference {
                    relation: relation.to_string(),
                    note_id: note.id.clone(),
                });
            }
        }
    }

    let mut has_any_link = false;
    let mut has_pending = false;
    let mut has_resolved = false;

    for (relation, ids) in &lists {
        for target_id in ids.iter() {
            has_any_link = true;
            if known_ids.contains(target_id) {
                has_resolved = true;
            } else if pending_ids.contains(target_id) {
                has_pending = true;
            } else {
                return Err(LinkError::BrokenLink {
                    relation: relation.to_string(),
                    target_id: target_id.clone(),
                });
            }
        }
    }

    if !has_any_link {
        return Ok(LinkState::Unlinked);
    }

    // Precedence choice for mixed resolved+pending (no broken) links: Pending
    // wins. This isn't exercised by this task's tests but is documented here
    // since a later task may depend on it.
    if has_pending {
        Ok(LinkState::Pending)
    } else {
        debug_assert!(has_resolved);
        Ok(LinkState::Resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Classification, NoteStatus, NoteType, TrustLevel};

    fn note_with_relation(relation_name: &str, target_id: &str) -> NoteMetadata {
        let mut relations = Relations::default();
        match relation_name {
            "supports" => relations.supports.push(target_id.to_string()),
            "contradicts" => relations.contradicts.push(target_id.to_string()),
            "derived_from" => relations.derived_from.push(target_id.to_string()),
            "supersedes" => relations.supersedes.push(target_id.to_string()),
            "applies_to" => relations.applies_to.push(target_id.to_string()),
            other => panic!("unknown relation name: {other}"),
        }

        let note_id = if target_id == "self-id" {
            "self-id".to_string()
        } else {
            "bug-2026-10-23-fastapi-cors".to_string()
        };

        NoteMetadata {
            id: note_id,
            schema_version: 1,
            r#type: NoteType::Bug,
            status: NoteStatus::Draft,
            trust: TrustLevel::Internal,
            classification: Classification::Internal,
            project: "project-x".to_string(),
            source: "agent:coder@prompt-v3".to_string(),
            created: "2026-10-23".to_string(),
            updated: "2026-10-23".to_string(),
            event_period: "2026-10-23".to_string(),
            relations,
        }
    }

    fn note_with_no_relations() -> NoteMetadata {
        NoteMetadata {
            id: "bug-2026-10-23-fastapi-cors".to_string(),
            schema_version: 1,
            r#type: NoteType::Bug,
            status: NoteStatus::Draft,
            trust: TrustLevel::Internal,
            classification: Classification::Internal,
            project: "project-x".to_string(),
            source: "agent:coder@prompt-v3".to_string(),
            created: "2026-10-23".to_string(),
            updated: "2026-10-23".to_string(),
            event_period: "2026-10-23".to_string(),
            relations: Relations::default(),
        }
    }

    #[test]
    fn broken_internal_link_is_rejected() {
        let note = note_with_relation("supports", "missing-note-id");
        let err = classify_links(&note, &HashSet::new(), &HashSet::new()).unwrap_err();
        assert!(matches!(err, LinkError::BrokenLink { .. }));
    }

    #[test]
    fn pending_reference_is_accepted_and_flagged() {
        let note = note_with_relation("derived_from", "external-pending-id");
        let mut pending = HashSet::new();
        pending.insert("external-pending-id".to_string());
        let state = classify_links(&note, &HashSet::new(), &pending).unwrap();
        assert_eq!(state, LinkState::Pending);
    }

    #[test]
    fn note_without_links_is_unlinked_and_accepted() {
        let note = note_with_no_relations();
        let state = classify_links(&note, &HashSet::new(), &HashSet::new()).unwrap();
        assert_eq!(state, LinkState::Unlinked);
    }

    #[test]
    fn resolved_link_to_known_id_is_accepted() {
        let note = note_with_relation("supports", "existing-id");
        let mut known = HashSet::new();
        known.insert("existing-id".to_string());
        let state = classify_links(&note, &known, &HashSet::new()).unwrap();
        assert_eq!(state, LinkState::Resolved);
    }

    #[test]
    fn self_reference_is_rejected_not_silently_resolved() {
        let note = note_with_relation("supports", "self-id"); // note.id == "self-id"
        let mut known = HashSet::new();
        known.insert("self-id".to_string());
        let err = classify_links(&note, &known, &HashSet::new()).unwrap_err();
        assert!(matches!(err, LinkError::SelfReference { .. }));
    }
}
