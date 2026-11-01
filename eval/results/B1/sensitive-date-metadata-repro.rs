use index::{document::Document,trigram::TrigramIndex,bm25::Bm25Index};
use storage::NoteId;
fn main() {
    for field in ["created", "updated", "event_period"] {
        let marker = format!("DATE_METADATA_CANARY_{}_41", field.to_uppercase());
        let value = |name:&str| if name==field {marker.as_str()} else {"2026-10-30"};
        let yaml = format!("id: note-metadata\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: sensitive\nproject: project-x\nsource: EXCLUDED_SOURCE_CANARY\ncreated: {}\nupdated: {}\nevent_period: {}\nrelations: {{}}\n",value("created"),value("updated"),value("event_period"));
        let metadata = domain::parse_frontmatter(&yaml).expect("schema accepts arbitrary string date");
        let record = format!("---\n{yaml}---\npii://note-metadata");
        let gate=policy::Gate::new(policy::SecretPolicy::new(1));
        let decision=gate.evaluate(&metadata,&record,Ok(domain::LinkState::Unlinked),vault_security::EncryptedStoreAvailability::Available);
        assert!(matches!(decision,policy::GateDecision::Pass));
        let document=Document::from_record_bytes(record.as_bytes()).unwrap();
        let id=NoteId::parse("note-metadata").unwrap();
        let mut trigram=TrigramIndex::new();trigram.insert(id.clone(),&document.text_searchable);
        let mut bm25=Bm25Index::new();bm25.insert(id,&document.text_searchable);
        let exact=trigram.query_exact(&marker).len();let ranked=bm25.query_ranked(&marker,10).len();
        println!("field={field} schema=accepted gate=Pass class=Sensitive canary_searchable={} trigram_hits={exact} bm25_hits={ranked}",document.text_searchable.contains(&marker));
        assert!(document.text_searchable.contains(&marker));assert_eq!((exact,ranked),(1,1));
        assert!(!document.text_searchable.contains("EXCLUDED_SOURCE_CANARY"));
        assert!(!document.text_searchable.contains("pii://"));
    }
}
