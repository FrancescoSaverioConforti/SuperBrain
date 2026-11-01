use std::{collections::HashSet, sync::Arc, time::Duration};
use index::indexer_impl::{RealIndexer,rebuild_index};
use storage::{Indexer, Vault, Version, NoteId};
use uuid::Uuid;
struct Shared(Arc<RealIndexer>);
impl Indexer for Shared {
 fn mark_current(&self,id:&str,v:u64,b:&[u8]) {self.0.mark_current(id,v,b)}
 fn state_for(&self,id:&str)->domain::IndexState {self.0.state_for(id)}
}
fn note(source:&str,body:&str)->String {
 format!("---\nid: note-race\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: project-x\nsource: {source}\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-10-30\nrelations: {{}}\n---\n{body}")
}
fn main() {
 for (iteration, delay) in [1,5,10,20,40,80].into_iter().enumerate() {
  let root=std::env::temp_dir().join(format!("superbrain-review-race-vault-{}",Uuid::new_v4()));
  std::fs::create_dir_all(&root).unwrap();
  {let repo=git2::Repository::init(&root).unwrap();let s=git2::Signature::now("Test","test@localhost").unwrap();let oid=repo.index().unwrap().write_tree().unwrap();let t=repo.find_tree(oid).unwrap();repo.commit(Some("HEAD"),&s,&s,"initial",&t,&[]).unwrap();}
  let pii_root=root.join(".pii-store");
  let pii=vault_security::PiiStore::open(pii_root.clone(),&pii_root.join("master.key")).unwrap();
  let idx=Arc::new(RealIndexer::new());
  let vault=Arc::new(Vault::open_with_pii_store_and_indexer(root.clone(),&root.join("journal.sqlite"),&pii,Box::new(Shared(idx.clone()))).unwrap());
  let gate=policy::Gate::new(policy::SecretPolicy::new(1));
  let old=note(&"oldsource ".repeat(200_000),"oldonlyneedle");
  assert!(matches!(storage::propose_and_commit(&vault,&old,Version(0),Uuid::new_v4(),&HashSet::new(),&HashSet::new(),&gate,Some(&pii)).unwrap(),storage::CommitResult::Committed(_)));
  let rv=vault.clone();let ri=idx.clone();
  let rebuild=std::thread::spawn(move || rebuild_index(&rv,&ri).unwrap());
  std::thread::sleep(Duration::from_millis(delay));
  let new=note("newsource","newonlyneedle");
  assert!(matches!(storage::propose_and_commit(&vault,&new,Version(1),Uuid::new_v4(),&HashSet::new(),&HashSet::new(),&gate,Some(&pii)).unwrap(),storage::CommitResult::Committed(_)));
  rebuild.join().unwrap();
  let current=vault.read_note(&NoteId::parse("note-race").unwrap()).unwrap();
  let oldhits=idx.bm25_ranked("oldonlyneedle",10).len();let newhits=idx.bm25_ranked("newonlyneedle",10).len();
  println!("iteration={iteration} delay_ms={delay} committed_version={} old_hits={oldhits} new_hits={newhits} state={:?}",current.version.0,idx.state_for("note-race"));
  if current.version==Version(2)&&oldhits==1&&newhits==0 {let facade=index::Index::new(&vault,&idx,"project-x","2026-10-30".parse().unwrap()); let response=facade.search("newonlyneedle",Default::default(),10).unwrap(); println!("new_term_search={:?}",response.outcome); assert!(matches!(response.outcome,index::search::SearchOutcome::NotPresent)); println!("REPRODUCED: rebuild overwrote v2 with v1 and reports Current");return;}
 }
 panic!("race not observed");
}
