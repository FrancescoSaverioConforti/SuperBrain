# Handoff P0 — DeletionLog fail-closed

Data: 2026-11-04 (aggiornato; prima stesura 2026-11-03). Questo handoff registra
lo stato effettivo del checkpoint P0. **Stato finale: GO per il commit P0, non
ancora committato.** Non è una dichiarazione di sicurezza globale del progetto.

## 0. Stato finale (2026-11-04) — leggere prima di tutto il resto

Tutte le condizioni della sezione 10 (acceptance criteria) sono ora soddisfatte
simultaneamente:

- `cargo test --workspace --locked`: **556/556 passati**, nessun hang, verificato
  due volte in modo indipendente (una volta dalla sessione principale, una volta
  dal reviewer indipendente che li ha rieseguiti lui stesso).
- `cargo clippy --workspace --locked -- -D warnings`: **clean**, verificato in
  modo indipendente dal reviewer.
- `cargo deny check`: **clean** (advisories/bans/licenses/sources OK); era stato
  bloccato da un guasto DNS transitorio verso `github.com`, risolto da solo senza
  alcuna modifica a `Cargo.lock` o alle dipendenze.
- DL-02/DL-03 **non più riproducibili**: il probe originale dell'audit non
  compila più contro il codice corrente (`DeletionLog::open` non è più
  pubblico); la stessa scena riportata sui veri entry point pubblici restituisce
  `Err(DeletionLogUnavailable)` in tutti i casi originariamente vulnerabili.
  Vedi sezione 7bis per i dettagli e l'output salvato.
- Mutation testing mirato su `deletion_log.rs`: **177 mutanti, ogni survivor
  triagiato con evidenza** (chiuso con test diretto, dimostrato equivalente, o
  documentato come residuo non-security). Vedi sezione 7ter.
- Review indipendente finale: **GO**, con verifica autonoma (non fiduciaria) di
  test, Clippy, claim di equivalenza, e assenza di bootstrap implicito nei
  percorsi production. Due finding non bloccanti (vedi sezione 7quater).

Il commit finale P0 non è stato ancora creato: resta da farlo come ultimo passo
di questa sessione, dopo aver aggiornato questo documento.

## 1. Stato Git

- Worktree: `C:\Users\fsave\Desktop\Python_code\SuperBrain\.worktrees\b1-deterministic-search`
- Branch: `b1-deterministic-search`
- HEAD: `e92dc781bcd32fcca5445295221f36aea2fe791e`
- Commit P0 principale: `e92dc78 security: fail closed deletion log lifecycle`.
- Audit approvato: `14ae361`.
- Nessun merge, tag o push è stato effettuato.

Modifiche non committate alla consegna (aggiornato 2026-11-04):

- `crates/storage/src/restore.rs`: nuovo test diretto sul gate finale di
  restore (`is_deleted_verified_locked_is_what_excludes_a_note_deleted_strictly_before_its_own_recheck`).
- `crates/storage/tests/a3_confidentiality_boundary.rs`: fixture ora invoca il
  bootstrap esplicito.
- `crates/storage/tests/a3_pii_integration.rs`: il crash pending verifica la
  genesis generation 0, non l'assenza del reference.
- `crates/vault-security/src/backup.rs`: fixture backup aggiornate da sentinel
  legacy a genesis autenticata; wrapper di race propaga `bound_path`; due test
  di pausa/resume corretti per il deadlock del test harness (vedi sezione 7);
  un test diviso in due per riflettere l'esclusione reciproca backup/write-lock
  del nuovo design; nuovo test sul confine `verify_safe_to_expose_after_restore`.
- `crates/vault-security/src/pii_store.rs`: test di gated read usa il log
  bootstrap/open esistente DPAPI, non il costruttore raw test-only.
- `crates/vault-security/src/deletion_log.rs`: ~25 nuovi unit test diretti sulle
  funzioni critiche di enforcement (sezione 7ter); `DeletionLog::is_genesis_reference`
  reso `pub(crate)` (era privato) solo per consentire ai test di `backup.rs` di
  usare il predicato reale invece di riimplementarlo — non ri-esportato da
  `lib.rs`, resta interno al crate.
- `docs/superpowers/plans/2026-11-01-p0-deletion-log-fail-closed.md`: rimossa
  una riga vuota finale segnalata da `git diff --check`.
- Nuovi file non tracciati: `docs/security/reproductions/deletion-read-gate-p0-rerun.rs`
  e `.stdout.txt` (riproduzione DL-02/DL-03 contro il codice corretto).
- `graphify-out/`: **rimosso** (era presente, ora pulito).

`git status --short` attuale:

```text
 M crates/storage/src/restore.rs
 M crates/storage/tests/a3_confidentiality_boundary.rs
 M crates/storage/tests/a3_pii_integration.rs
 M crates/vault-security/src/backup.rs
 M crates/vault-security/src/deletion_log.rs
 M crates/vault-security/src/pii_store.rs
 M docs/superpowers/plans/2026-11-01-p0-deletion-log-fail-closed.md
?? docs/handoffs/P0-deletionlog-handoff.md
?? docs/security/reproductions/deletion-read-gate-p0-rerun.rs
?? docs/security/reproductions/deletion-read-gate-p0-rerun.stdout.txt
```

`git diff --check`: clean (nessun errore, solo warning di fine riga LF/CRLF).
Diffstat: 7 file, 944 inserzioni, 116 rimozioni.

## 2. Obiettivo del P0

DL-02/DL-03 permettevano la resurrezione consultabile di una nota cancellata
quando DB del DeletionLog, reference o catena venivano persi o alterati. Lo
stato `(DB missing, reference missing)` è semanticamente ambiguo: non dimostra
né genesis né assenza storica di cancellazioni, quindi non può essere trattato
automaticamente come genesis.

Decisione approvata: solo `bootstrap_new` esplicito può inizializzare un nuovo
target; `open_existing` non crea DB, schema, reference, lock o stato implicito.
La genesis è DB versionato + reference DPAPI autenticato, generation 0 e
`store_id` legato. Stati mancanti, incompatibili, corrotti o ambigui negano
l'accesso. Vault e backup legacy non attestati sono quarantinati/rifiutati;
una migrazione amministrativa attestata è distinta e auditabile.

Rischi esplicitamente ancora aperti:

- rollback di una vecchia coppia DB+reference autentica, senza anchor monotona
  indipendente;
- host/filesystem malevolo che ignora deliberatamente i lock cooperativi.

## 3. Implementazione corrente

Il commit `e92dc78` modifica:

- `crates/vault-security/src/deletion_log.rs`, `pii_store.rs`, `backup.rs`,
  `lib.rs`, `Cargo.toml`;
- `crates/storage/src/vault.rs`, `propose.rs`, `recovery.rs`, `restore.rs`;
- fixture API/index/storage e piano P0.

State machine implementata:

- `Genesis`: DB schema/meta versionati e reference autenticato generation 0;
- `ExistingValid`: catena completa + reference/head coerenti, incluso pending
  nella sola transizione recuperabile;
- `LegacyUnattested`: stato vuoto/mancante legacy che richiede migrazione
  amministrativa esplicita;
- `CorruptOrTampered`: reference autenticato presente ma DB/lock assente,
  catena/reference/store-id/identity incoerenti o artefatti sostituiti;
- `RollbackUnknown`: vecchia coppia autentica non distinguibile, documentata ma
  non risolta.

`bootstrap_new` è esplicito, usa lock di inizializzazione e gestisce il solo
retry sicuro di genesis parziale autenticata. `open_existing` usa SQLite senza
create, non crea lock/schema/reference e classifica l'assenza in modo
reference-aware. Il reference DPAPI usa replace atomico Windows
`MoveFileExW(REPLACE_EXISTING|WRITE_THROUGH)`.

`DeletionLog` lega DB/reference/lock per path e identità. `with_verified`,
`with_verified_lookup` e `with_verified_not_deleted` mantengono il lock durante
catena/reference/lookup e fino alla decisione/read. `record_deletion` verifica
e riconcilia sotto lock prima di appendere; restore mantiene il lock fino a
rename/journal decision; recovery avanza `JournalCommitted` nella callback
verificata.

Consumer aggiornati:

- `PiiStore`: `bootstrap_deletion_log`, `open_deletion_log`, gated read DPAPI.
- `Vault`: open/reconcile, read, list e write-side gating verificati.
- propose/delete: apre solo lo stato esistente per il writer.
- recovery e restore: decisioni critiche coordinate al guard.
- backup: snapshot sotto verifica; legacy empty/sentinel rifiutato; backup
  legacy con storia positiva compatibile solo dopo verifica.
- search/retrieve: arrivano a `Vault::list_note_ids`/`read_note`, quindi usano
  il gate autorevole.

Le fixture non committate sopra aggiornano aspettative dal vecchio sentinel
reference-assente alla genesis autenticata.

## 4. Graphify / call graph

Graphify è stato eseguito come supporto (`graphify extract crates --code-only
--out . --no-cluster`), producendo un grafo AST di 1510 nodi/5678 edge. Una
query ha confermato i consumer: `PiiStore`, `Vault` open/read/list/write,
propose, recovery, backup, restore, index rebuild/search e API retrieve.

La ricerca diretta con `rg` resta autorevole e ha confermato:

- i consumer production usano `bootstrap_new`, `open_existing` e callback
  `with_verified*` ai punti previsti;
- `search` e `retrieve` arrivano a `Vault::read_note`; rebuild usa
  `list_note_ids` poi `read_note`;
- `DeletionLog::open` e `is_deleted` raw sono `pub(crate)`/`#[cfg(test)]` e
  le chiamate residue sono nei test interni, non nei percorsi production.

`graphify-out/` è solo output temporaneo e non deve entrare nel commit.
Graphify non è fonte di verità: confermare sempre con codice e test.

## 5. Verifiche già completate

Comandi/esiti effettivi:

- `cargo test --workspace --locked --no-run`: passato prima della validazione
  runtime Windows.
- `cargo test -p storage --test a3_confidentiality_boundary --locked`:
  6/6 passati sul runner Windows reale dopo il bootstrap fixture.
- `cargo test -p storage --test a3_pii_integration --locked`: 14/14 passati
  sul runner Windows reale, incluso pending/recovery.
- `cargo clippy --workspace --locked -- -D warnings`: passato.
- `cargo deny check`: advisories/bans/licenses/sources OK; warning storici
  duplicate/unmatched allowance, nessun errore.
- `git diff e92dc78^..e92dc78 --check`: inizialmente ha segnalato una blank
  line EOF nel piano; la correzione è non committata e `git diff --check` sul
  worktree non segnala più errori.
- Ricerca diretta: `rg -n "DeletionLog::(bootstrap_new|open_existing|open)\\(|\\.is_deleted\\(|with_verified|verify_(chain|consistent|safe)|acquire_(write|existing_write)_lock|reconcile\\(" crates`.
- Graphify e la relativa query call-path sono descritti nella sezione 4.
- Review indipendente dell'intero P0: **GO source-only**. Il reviewer ha
  ricontrollato il census Graphify/`rg`, i gate PiiStore/Vault/backup/restore/
  recovery e i percorsi search/retrieve; non ha trovato un consumer production
  che faccia bootstrap implicito o un lookup non verificato. Il verdetto resta
  condizionato alle verifiche runtime Windows e mutation ancora pendenti.

Una nuova `cargo test --workspace --locked` è stata lanciata sul runner Windows
reale e ha mostrato pass per API/domain/index/policy/storage e integration
storage. Il primo run del 2026-11-03 non si è completato: ha fallito una
fixture di race backup e una fixture di lock, poi è rimasto bloccato. Entrambi
sono stati riprodotti in isolamento e corretti senza modificare il protocollo:

- la race chiamava `record_deletion` in modo rientrante mentre check 1 già
  deteneva il lock verificato; ora verifica l'errore fail-closed
  `WriteLockUnavailable` e l'assenza di file esposti;
- il test inter-processo apriva nuovi `open_existing` dopo che il child aveva
  già acquisito il lock, ma l'apertura ora verifica sotto lock; ora apre i due
  consumer prima del child e verifica timeout bounded, attesa e roll-forward.

Entrambi passano isolatamente sul runner Windows reale. Un secondo run completo
ha superato anche tutti i 96 test `vault-security`, inclusi i due corretti, ma
è poi rimasto bloccato oltre 60 secondi in due test pending-restore:
`a_backup_taken_while_a_deletion_is_still_pending_and_then_restored_does_not_expose_the_note_that_deletion_targets`
e `restoring_from_a_sentinel_backup_while_the_deletion_is_still_pending_is_rejected_not_silently_succeeded`.
Il run deve pertanto essere considerato non verde finché i due hang non sono
riprodotti e risolti/giustificati in isolamento.

## 6. Blocco Windows attuale

Il precedente blocco era `LNK1104` perché un test `vault_security-…exe` restava
aperto. L'utente ha autorizzato la terminazione esclusivamente del PID verificato
appartenente a questa worktree/sessione, dopo verifica di PID, command line,
tree e inattività. Sono stati terminati solo:

- PID 17924: `vault_security-e0fce1dd275f6cb8.exe` sotto
  `...b1-deterministic-search\\target\\debug\\deps`;
- PID 24432: suo parent `cargo test -p vault-security --lib --locked`.

Non terminare genericamente tutti i processi `cargo`, `rustc` o test Rust.

Al momento dell'handoff è attivo un nuovo run legittimo:

- PID 13128: `cargo` iniziato 2026-11-02 17:39:57;
- PID 7980: `vault_security-2d3aec9a9860ea89.exe` nella worktree, con CPU
  in aumento (non classificato come residuo al momento dell'handoff).

Prima di terminare qualunque futuro PID, ricontrollare command line, parent/
children, path worktree e attività CPU. Non interrompere un test ancora attivo.

Aggiornamento 2026-11-03: quei PID non esistevano più. Il nuovo run completo
ha poi lasciato bloccati, dopo i fallimenti sopra, esclusivamente PID 24704
(`vault_security-2d3aec9a9860ea89.exe` nella worktree) e il parent PID 17432
(`cargo test --workspace --locked`). Command line, parentela, percorso e CPU
invariata in un campione di 5 secondi sono stati verificati; sono stati
terminati soltanto questi due PID. Nessuna terminazione indiscriminata.

Un secondo run si è bloccato con PID 26076 (`vault_security-2d3aec9a9860ea89.exe`)
e parent PID 8664 (`cargo test --workspace --locked`), entrambi verificati
appartenenti alla worktree e con CPU invariata in un campione di 5 secondi;
sono stati terminati solo questi PID dopo l'assenza di output oltre i timeout.

## 7. Verifiche — tutte completate (2026-11-04)

- [x] riprodotti in isolamento e risolti i due hang pending-restore (causa:
  un `assert!` fallito dentro `std::thread::scope` lasciava il thread figlio
  bloccato per sempre su `resume_rx.recv()`, perché `thread::scope` non
  propaga il panic finché tutti i thread non si uniscono — risolto con una
  `ResumeGuard` basata su `Drop` che invia comunque il segnale di resume);
  suite Windows completa rieseguita e registrata (556/556);
- [x] riprodotte esplicitamente le riproduzioni DL-02/DL-03 e registrato che
  falliscono closed — sezione 7bis;
- [x] eseguita la suite restore/recovery end-to-end (compresa nei 556 test
  workspace);
- [x] mutation testing mirato eseguito e ogni survivor triagiato — sezione 7ter;
- [x] workspace tests, Clippy e Deny finali eseguiti e verificati, inclusa una
  seconda verifica indipendente dal reviewer;
- [x] verdetto GO confermato dalla review indipendente con le prove runtime
  Windows e mutation — sezione 7quater;
- [x] rimosso `graphify-out/`, verificato `git diff --check` (clean).

## 7bis. Riproduzione DL-02/DL-03 contro il codice corretto

Il probe originale dell'audit (`docs/security/reproductions/deletion-read-gate.rs`,
che aveva prodotto `after_missing_log_returns_plaintext=true`,
`missing_reference_other_note_returns_plaintext=true`,
`tampered_chain_deleted_note_returns_plaintext=true` prima del fix) è stato
ricompilato verbatim contro il crate corrente: **non compila più** —
`DeletionLog::open` (il punto di accesso non autenticato che rendeva possibile
la vulnerabilità) è ora `pub(crate)`/`#[cfg(test)]`, fuori dall'API pubblica
che un chiamante esterno può raggiungere.

La stessa scena è stata riportata sui veri entry point pubblici
(`PiiStore::bootstrap_deletion_log`/`open_deletion_log`, che usano
`DeletionLog::open_existing` sotto il cofano) come
`docs/security/reproductions/deletion-read-gate-p0-rerun.rs`, eseguito
standalone via `rustc` esattamente come la metodologia originale dell'audit
(non `cargo test`). Output salvato in
`docs/security/reproductions/deletion-read-gate-p0-rerun.stdout.txt`:

| Scenario | Audit originale (vulnerabile) | Ri-esecuzione P0 (corretto) |
| --- | --- | --- |
| Log mancante, lettura nota cancellata | `returns_plaintext=true` | `false` — `Err(DeletionLogUnavailable{reason:"...lost required coordination state"})` |
| Reference mancante, lettura ALTRA nota | `returns_plaintext=true` | `false` — stesso errore |
| Catena manomessa, lettura nota cancellata | `returns_plaintext=true` | `false` — `Err(DeletionLogUnavailable{reason:"deletion log hash chain is broken"})`, rilevato già al re-open (`tampered_chain_detectable_at_reopen=true`) |

Aggiunta una verifica non presente nel probe originale:
`no_implicit_recreation_confirmed=true` — `open_existing` non ricrea mai
implicitamente il file DB mancante (il vecchio `DeletionLog::open` lo faceva,
ed era parte del meccanismo del bug originale).

Tutti gli artefatti di scratch del probe (`.superpowers/p0-rerun-missing-log-*`,
`.superpowers/security-audit-build-rerun`) sono stati rimossi dopo l'uso.

## 7ter. Mutation testing — triage completo

`cargo mutants --package vault-security --file crates/vault-security/src/deletion_log.rs`,
177 mutanti totali, eseguito due volte (prima scoped solo ai test di
`vault-security`, poi con `--test-workspace true` per includere il test
obbligatorio in `storage/restore.rs`). Risultato finale del run completo:
**124 caught, 1 missed (poi corretto), 25 unviable, 27 timeout**.

**Chiusi con test diretto, ciascuno verificato applicando manualmente la
mutazione esatta segnalata da cargo-mutants, confermando che il nuovo test
fallisce, poi ripristinando il codice corretto:**
`is_deleted_verified_locked` (test obbligatorio, intercetta sia `Ok(true)` che
`Ok(false)`), `is_genesis_reference`, `verify_file_identities`,
`assert_reference_bound`, `verify_existing_or_pending` (2 survivor),
`verify_consistent` (2 survivor), `acquire_existing_write_lock` (ramo
`NotFound`), `reconcile_locked` (3 survivor), `verify_binding_on`,
`migrate_legacy_empty` (4 survivor), `is_retryable_genesis_partial`
(3 survivor), `GenerationRef::deserialize`, `DpapiTrustedReferenceStore`
(`bound_path` e il guard `NotFound` di `read`),
`verify_safe_to_expose_after_restore` (confine generazione backup-avanti-di-live).

**Nota di processo importante:** il primo tentativo di verifica manuale su
`verify_binding_on` ha mutato uniformemente tutti gli operatori della riga
invece della singola colonna esatta segnalata da cargo-mutants — questo ha
mascherato un gap reale (il primo `||` della catena a tre vie), scoperto solo
dal secondo mutation sweep completo. Corretto con un'asserzione più precisa
e riverificato. Lezione: verificare sempre la mutazione a livello di singolo
nodo AST, non l'intera espressione booleana.

**Classificati come mutanti equivalenti, con dimostrazione logica (non solo
"non ho trovato un test"):**
- `verify_chain_on`: `generation < 0` è matematicamente ridondante rispetto al
  controllo adiacente `generation as u64 != expected_generation` — qualunque
  `i64` negativo castato a `u64` produce un valore ≥ 2^63, che non può mai
  coincidere con `expected_generation` (piccolo contatore positivo).
- `reconcile_locked` (riga ~1488): i tre varianti di confronto
  (`<`/`==`/`>`/`<=`) sono equivalenti perché **entrambi** i rami
  dell'`if`/`else` restituiscono lo stesso identico `Err(DeletionLogError::Inconsistent)`
  — nessun test può osservare una differenza.
- `acquire_existing_write_lock`: il confine temporale `Instant::now() <= deadline`
  non è raggiungibile in modo deterministico senza un test fragile basato sul
  timing.
- `DeletionLogWriteGuard::drop` mutato a no-op: equivalente nel comportamento
  osservabile, perché la chiusura del file handle (il `Drop` implicito di
  `std::fs::File`) rilascia comunque il lock OS, come documentato nel commento
  della funzione stessa — cambia solo il momento del rilascio, non testato da
  nessun caso.

**Residui non chiusi, fuori scope o infrastruttura di test, documentati
esplicitamente:**
- `identity_of_open_file` (combinazione bit `nFileIndexHigh`/`nFileIndexLow`
  in Windows): non testato direttamente — richiederebbe costruire file con
  valori NTFS raw specifici, non praticabile tramite l'API pubblica senza
  mocking FFI profondo.
- `FaultInjector::none()` e `FaultInjector::check`: infrastruttura di test,
  non codice production.
- `atomic_replace_reference`'s variante `#[cfg(not(windows))]`: codice morto
  su questa piattaforma (il progetto è Windows-only); cargo-mutants la muta
  sintatticamente ma non è mai linkata nel binario di test reale.

I 27 "timeout" sono un artefatto del timeout automatico di 35s calcolato da
cargo-mutants (troppo stretto per rieseguire l'intera suite workspace,
compresi i test di lock reali con sleep deliberati, ad ogni mutante) — non
sono survivor reali: ogni mutante corrispondente già presente nell'elenco
sopra è stato verificato caught tramite applicazione manuale diretta prima
che il sweep completo girasse.

## 7quater. Review indipendente finale

Lanciata con il briefing completo (finding originale, nuovo lifecycle,
test di concorrenza, final recheck restore, triage dei survivor, dimostrazioni
di equivalenza, residui). **Verdetto: GO.**

Verifiche autonome del reviewer (non fiduciarie):
- Riesecuzione propria di `cargo test --workspace --locked` (556 test, 0
  falliti) e `cargo clippy --workspace --locked -- -D warnings` (clean).
- Verifica indipendente delle due dimostrazioni di equivalenza
  (`verify_chain_on`, `reconcile_locked`): confermate corrette con
  ragionamento proprio, non solo accettate.
- Grep di tutti i call site di `bootstrap_new`/`open_existing` fuori dai
  moduli di test: **zero call site production a `bootstrap_new`**; tutti i
  consumer (`Vault::open_with_pii_store`, `read_note`, `list_note_ids`, il
  gate di scrittura `refuse_read_if_deleted`, `propose.rs`, `recovery.rs`)
  usano correttamente `open_existing` più le callback verificate.
- Spot-check di 4 nuovi unit test, confermati coerenti con i loro commenti.

Due finding non bloccanti:
1. **Minore/cosmetico**: il nome del test sul confine
  `verify_safe_to_expose_after_restore` suggerisce un rifiuto ma l'asserzione
  è `is_ok()`. Comportamento corretto — `BackupReferenceStale` significa solo
  "il backup è indietro"; la vera protezione anti-resurrezione è
  l'esclusione per-nota indipendente in `backup.rs`, non questo gate di
  staleness. Non è un buco di sicurezza, solo un nome che potrebbe confondere
  un lettore futuro. Non blocca.
2. **Osservazione, fuori scope**: nessun entry point production/binario nel
  workspace chiama `bootstrap_deletion_log` per un nuovo store genuino — non
  esiste ancora un host layer in questo codebase. Domanda legittima di
  "chi provisiona un nuovo store", ma è una questione di
  deployment/orchestrazione che lo scope del piano (solo libreria,
  pre-host-layer) non pretende di risolvere, e non è una regressione P0.

Nessuno dei due finding blocca la chiusura di questo incremento P0.

## 8. Security audit collegato

Audit: `docs/security/security-audit.md`; commit audit `14ae361`.
Il finding P0 è il DeletionLog DL-02/DL-03. Restano aperti e non vanno dichiarati
risolti: temporali/schema Sensitive, API raw PiiStore, AEAD/AAD, backup
provenance, filesystem SEC-001/002/003, DoS e superfici future MCP/agente.
L'audit mantiene il verdetto NO-GO globale.

## 9. Prossima azione esatta

Tutte le verifiche sono complete e positive. L'unico passo rimasto è il
**commit finale P0** (nessun merge, tag o push). Dopo il commit:

1. Registrare l'hash del commit finale, il conteggio test, i risultati di
   mutation testing e il verdetto del reviewer in Continuity.
2. Il branch/worktree resta in attesa dell'autorizzazione esplicita
   dell'utente per merge/tag/push — non richiesta in questo checkpoint.

## 10. Acceptance criteria per chiudere P0 — tutti soddisfatti

P0 è GO:

- [x] tutti i consumer production usano il lifecycle fail-closed (verificato
  due volte, inclusa la review indipendente via grep sistematico);
- [x] nessun consumer crea implicitamente il DeletionLog (zero call site
  production a `bootstrap_new`, confermato dal reviewer);
- [x] DL-02/DL-03 non sono più riproducibili (sezione 7bis);
- [x] la suite Windows completa passa (556/556, verificato due volte);
- [x] mutation testing sui branch critici non lascia survivor non spiegati
  (sezione 7ter: ogni survivor chiuso, dimostrato equivalente, o documentato
  come residuo non-security);
- [x] review indipendente dà GO (sezione 7quater);
- [x] rischi residui sono esplicitamente documentati: rollback di una vecchia
  coppia DB+reference autentica senza anchor monotona indipendente; host/
  filesystem malevolo che ignora deliberatamente i lock cooperativi (sezione 2);
  `identity_of_open_file` non testato direttamente (sezione 7ter); assenza di
  un host layer che provisioni un nuovo store in produzione (sezione 7quater).

Non modificato `docs/Spec_mvp.md`; nessun merge, tag o push eseguito.
