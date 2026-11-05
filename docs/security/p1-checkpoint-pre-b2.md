# P1 security checkpoint — pre-B2 classification

Data: 2026-11-05. Base: `docs/security/security-audit.md` (HEAD `21f0e56`,
P0 DL-02/DL-03 closed at `3ca2203`, B1 merged to `main` at `d217f93b`, tag
`b1-complete`). Nessuna correzione implementata da questo documento: solo
analisi e classificazione. `docs/Spec_mvp.md` non modificato.

B2 (da `Spec_mvp.md` §Fase B, punto 3 e riga 429) introduce: indicizzatore
tree-sitter multi-linguaggio, grafo del codice (`symbol_snapshot_id`,
`symbol_lineage_id`, archi tipizzati con metodo/evidenza/commit),
reindicizzazione incrementale per hash e invalidazione dopo cambio commit,
API `graph_query`/`get_code_context`, server MCP di sola lettura. Tocca
quindi: ingestion di nuovo contenuto non fidato (codice sorgente di repo
arbitrari), identity/versioning cross-commit, filesystem traversal su intere
repository, e API che espongono dati a un client MCP esterno — esattamente
le superfici che il checkpoint deve pesare.

## Metodo seguito

Per ciascun finding: riletto `security-audit.md`, letto il codice reale
citato (non la sola riga dell'audit), verificato se il meccanismo P0 già
chiuso (DL-02/DL-03) lo copre, valutato l'uso diretto/indiretto da B2 e
l'eventuale amplificazione. Le verifiche puntuali sul codice corrente:

- `crates/vault-security/src/pii_store.rs`: ogni metodo pubblico (`stage`,
  `commit_staged`, `get`, `get_unless_deleted`, `restore_before_image`,
  `delete_before_image`, `has_before_image`) prende `note_id: &str` e lo
  interpola direttamente in `live_path`/`staging_path`/`before_image_path`
  senza validazione — la confinamento è delegato per intero al chiamante
  (`storage::NoteId`). Nessun uso di `Aad`/AAD in `aead_encrypt`/
  `aead_decrypt`/`decrypt_buffer`: il tag autentica solo i byte, non
  identità/versione/formato.
- `crates/vault-security/src/backup.rs::collect_vault_files`: cammina la
  vault con `path.is_dir()` (segue symlink, nessun `symlink_metadata`,
  nessun visited-set/cycle guard) e legge ogni file regolare, mai tramite
  `Vault::read_note` né verifica contro il journal.
- `crates/storage/src/vault.rs`: `ensure_resolves_inside_root` blocca
  containment via canonicalizzazione; i test `sec_001_*`/`sec_002_*`/
  `sec_003_*` (righe ~4206–4563) mostrano che il **percorso di lettura**
  (`read_note`, hash check sulla riga journal) chiude le varianti dove la
  nota ha già una riga committata, ma il **percorso di scrittura**
  (`write_record_unchecked`'s pre-read per il confronto di versione) resta
  aperto per TOCTOU (SEC-001) e per l'aliasing cross-nota (SEC-002/003) —
  commenti inline nei test lo dichiarano esplicitamente "fully open"/
  "remains fully open".
- `note_lock` (riga 1001) è un `Mutex` in-processo per note id, non un lock
  cross-processo: il modello multi-writer resta Not Tested fuori dal
  perimetro specifico già chiuso per il DeletionLog.
- Il piano P0 (`docs/superpowers/plans/2026-11-01-p0-deletion-log-fail-closed.md`)
  dichiara esplicitamente il proprio limite: "Replaying an older, mutually
  authentic DB+reference pair remains possible without an independent
  monotonic anchor" — quindi DL-04 (anchor anti-rollback) resta aperto anche
  dopo la chiusura di DL-02/DL-03.

Nessun nuovo finding Critical/Important è emerso durante questa rilettura
che non fosse già nella matrice dell'audit: la classificazione sotto è una
ri-prioritizzazione rispetto a B2, non un nuovo audit.

## Tabella di classificazione

| Finding | Stato attuale | B2 dependency | Failure mode se ignorato | Categoria | Motivazione | Fix/decisione minima richiesta | Deve chiudersi prima di B2? |
| --- | --- | --- | --- | --- | --- | --- | --- |
| DL-02/DL-03 (DeletionLog read gate) | **Chiuso** (`3ca2203`, open_existing/bootstrap_new separati, verified lookup sotto lock) | Diretta: ogni `get_code_context`/`graph_query` che risolve a una nota cancellata deve rispettare lo stesso gate | Dato cancellato tornerebbe consultabile tramite il grafo | — (già chiuso) | Verificato nel codice merged; non riaprire | Nessuno — monitorare che B2 riusi il reader gated, non un path raw | No (già fatto) |
| DL-04 (anti-rollback anchor) | Partially Mitigated → resta aperto anche post-P0 | Indiretta: B2 non introduce replay, ma eredita lo stesso rischio su ogni nota che indicizza | Replay di coppia DB+reference autentica vecchia resuscita contenuto/cancellazioni pre-B2 | PRE-B2 DESIGN DECISION | Non bloccante per la logica di B2 in sé (B2 legge tramite lo stesso gate, non introduce un secondo canale); ma è una decisione architetturale di cui B2 eredita il rischio senza poterlo mitigare da solo | Decidere ancora monotona indipendente (TPM/servizio/counter firmato) oppure accettare esplicitamente il modello FS-trusted come limite documentato | No, ma la decisione va presa prima di aggiungere nuove superfici di lettura (B2 ne aggiunge) |
| Temporal/schema validation (CF-02/CF-03/ST-07) | Partially Mitigated: fix search (`3c6b1bb`) chiude l'esposizione in SearchResponse; validazione **in scrittura** non ancora autorizzata | Indiretta: B2 non scrive date Sensitive, ma `schema_version` non-checked (ST-07, cast troncante) e metadati non sanitizzati sono dello stesso schema che B2 eredita per ogni nota collegata a simboli | Un futuro campo di B2 (es. commit hash, timestamp lineage) rischia la stessa falla: String non validata promossa a dato strutturato | PRE-B2 DESIGN DECISION | Non blocca B2 direttamente (B2 non introietta i tre campi Sensitive), ma la causa radice — "allowlist di nomi senza tipi validati" — è esattamente il pattern che B2 rischia di ripetere per `symbol_lineage_id`/hash di commit se non c'è un confine Raw→Validated condiviso | Fissare la grammatica canonica e il confine DTO raw/validato PRIMA di definire i tipi di B2 (`symbol_snapshot_id`, hash commit), così B2 li costruisce già tipizzati invece di ripetere il debito | Sì, come decisione di tipo prima di definire gli schemi di B2 — non come fix retroattivo dei 3 campi |
| PiiStore raw API confinement (FS-05) | Vulnerable | Nessuna diretta: B2 non cifra via PiiStore (codice sorgente non è PII), ma se B2 introduce un adapter che tocca lo stesso store (es. per nascondere snippet sensibili in futuro) erediterebbe il buco | Path traversal fuori root tramite `&str` non validato, se un futuro adapter lo inoltra | DEFERRED (per B2 specificamente) | B2 come scope descritto in Spec_mvp §Fase B non chiama `PiiStore`; il rischio è reale ma non è nel percorso critico di B2 | Documentare il contratto "ogni chiamante esterno a `storage` deve passare da NoteId validato, mai raw `&str`" cosi che un futuro adapter B2/B3 non lo violi per errore | No per B2 stesso; sì prima di qualunque codice che colleghi B2/B3 a PiiStore |
| AEAD AAD binding (CR-02) | Vulnerable | Nessuna diretta: B2 non usa l'envelope PII | DEFERRED | Stesso ragionamento di FS-05: fuori dal perimetro codice di B2 | Nessuno per B2; tenere la migrazione AAD come debito esplicito per quando PII e grafo potrebbero toccarsi (es. redazione di snippet) | No |
| Backup/restore provenance (BK-01/BK-02/DL-07) | Vulnerable (BK-01/BK-02), Partially Mitigated (DL-07) | **Diretta e amplificata**: B2 aggiunge molti più file (indici per-repo, snapshot di grafo, cache tree-sitter) sotto o vicino alla vault root; `collect_vault_files` già legge "ogni file regolare" senza distinguere committato da piantato, e segue symlink senza cycle-guard | Un backup/restore promuove a "vault" dati mai passati dal gate (incl. futuri artefatti di grafo/indice), oppure un symlink-cycle piantato in una repo indicizzata causa DoS o escape durante il backup | **BLOCKER B2** | B2 moltiplica la superficie che `collect_vault_files` attraversa (intere repository esterne collegate/indicizzate) e il tipo di contenuto (codice arbitrario, potenzialmente con symlink/cicli legittimi in repo reali) rende lo scenario BK-02 non più solo teorico ma plausibile con input ordinario, non solo malevolo | Minimo: far derivare la raccolta backup da una fonte autorevole committata (journal/Vault), non da una camminata raw; aggiungere walk su `symlink_metadata` con rifiuto/esclusione esplicita di symlink e limite di profondità prima che B2 inizi a indicizzare repository esterne reali | **Sì** |
| SEC-001 (TOCTOU path check → write) | Vulnerable (write-side "fully open" per commento inline nei test) | **Diretta**: la reindicizzazione incrementale di B2 "per hash e invalidazione dopo cambio commit" scrive/aggiorna frequentemente — più scritture concorrenti aumentano la finestra di race | Una race vinta da un writer concorrente (anche cooperativo, non necessariamente malevolo, se B2 introduce più processi/thread di reindicizzazione) sostituisce silenziosamente il target di scrittura | **BLOCKER B2** | L'audit nota che il rischio sale con "writer FS concorrente"; B2 introduce esattamente un nuovo motivo di scritture frequenti e potenzialmente concorrenti (invalidazione incrementale su ogni commit) | Minimo: I/O handle-relative (apri il file/dir per handle prima di operare, non per path) sul percorso di scrittura, o quantomeno un lock cross-process esplicito sul write path prima che B2 aggiunga un secondo scrittore frequente | **Sì** |
| SEC-002 (cross-note write-path aliasing) | Vulnerable (write-path "remains fully open" per commento inline) | **Diretta**: stesso ragionamento di SEC-001 — B2 scrive per ogni simbolo/commit, aumentando il numero di note/entry coinvolte e quindi la probabilità statistica di collisione nome/alias se un domain futuro di B2 riusa lo stesso note-id-space o un adapter non passa da NoteId validato | Scrittura su una nota aliasa silenziosamente un'altra nota tramite link pre-esistente, corrompendo versione/contenuto senza errore visibile | **BLOCKER B2** | Stesso codice (`write_record_unchecked`) di SEC-001; non ha senso chiuderne uno senza l'altro, ed entrambi condividono il fix minimo (containment sul path di scrittura, non solo di lettura) | Stesso fix minimo di SEC-001: containment/handle-relative anche sul percorso di scrittura, non solo sul read path già protetto da hash | **Sì** (insieme a SEC-001) |
| SEC-003 (hard-link aliasing) | Partially Mitigated: sotto-caso "nota già scritta" chiuso dall'hash check; sotto-caso "mai scritta" chiuso dal journal lookup; **nessuna delle due prova il percorso di scrittura pre-read** | Diretta, stesso canale di SEC-001/002 (write-path pre-read) | Hard-link piantato su un note-id mai scritto aliasa contenuto esterno al momento della prima scrittura (non testato direttamente, ma lo stesso meccanismo di SEC-001/002 si applica) | PRE-B2 DESIGN DECISION (non blocker separato: si chiude con lo stesso fix di SEC-001/002) | Non è un buco indipendente da richiudere separatamente — condivide la causa e il fix con SEC-001/002; elevarlo a blocker a parte duplicherebbe il lavoro | Nessuno aggiuntivo: la fix SEC-001/002 deve esplicitamente coprire anche l'hard-link (canonicalizzazione non vede hard-link, serve link-count/handle check, non solo containment di path) | No come item separato — ma il fix SEC-001/002 deve includerlo esplicitamente nei criteri di accettazione |
| Single-writer vs multi-writer (ST-03) | Not Tested | **Diretta**: B2 "reindicizzazione incrementale" suggerisce un secondo processo (watcher/CI/VS Code extension) che scrive sullo stesso Vault/journal mentre il processo principale è aperto | Due istanze Vault aprono lo stesso journal durante write/recovery: CAS per-operazione protegge, ma nessun lock globale testato — rollback silenzioso di lavoro altrui | PRE-B2 DESIGN DECISION | Non è un bug dimostrato (Not Tested, non Vulnerable) ma B2 è il primo punto del progetto che rende concreto un secondo scrittore plausibile (reindicizzazione per hash dopo ogni commit, potenzialmente da un hook/watcher separato dal processo MCP principale) | Decidere esplicitamente: B2 gira nello stesso processo/Vault-handle del resto (single-writer enforced), oppure si implementa e testa un lock cross-process prima di introdurre un secondo scrittore | Sì come decisione (non necessariamente come implementazione completa) — va scelta l'architettura prima di scrivere il watcher/reindexer di B2 |
| DoS / resource budgeting (IX-04/IX-05) | Vulnerable (IX-04 misurato), Not Tested (IX-05) | **Diretta e amplificata**: l'indicizzazione di repository di codice (potenzialmente grandi) più il grafo (archi per ogni riferimento/import/chiamata) moltiplica sia la dimensione del corpus sia la complessità delle query (`graph_query` può attraversare un numero di archi non limitato) | Query search o graph_query su corpus grande esaurisce CPU/RAM/tempo (già misurato fino a 100k record, ~40 min per query su search; graph_query rischia lo stesso senza budget) | **BLOCKER B2** (per l'API `graph_query`/`get_code_context` specificamente, non per l'indicizzazione in sé) | L'audit già classifica IX-04 Vulnerable con misure reali; B2 introduce un secondo tipo di query (traversal di grafo) strutturalmente più incline a fan-out esponenziale di una ricerca testuale — esporlo senza budget è peggio della baseline già vulnerabile | Minimo: limite di profondità/numero-archi su `graph_query` e budget di tempo/record su reindicizzazione incrementale, prima di esporre queste API anche solo al server MCP locale | **Sì**, prima di esporre `graph_query`/`get_code_context` via MCP (può restare aperto se B2 resta solo libreria interna senza API esposta, ma lo scope B2 include esplicitamente il server MCP) |
| Metadata classification policy (CF-06/CF-03) | Partially Mitigated/Vulnerable | Indiretta: B2 non introduce nuovi livelli di classification, ma `symbol_lineage_id` "confermata o ipotizzata" è una nuova forma di asserzione di fiducia simile a `official`/`verified` | Un lineage "ipotizzato" potrebbe essere trattato come "confermato" da un consumer che non distingue i due, analogamente a come oggi nulla impedisce una dichiarazione `official` non autenticata | PRE-B2 DESIGN DECISION | Spec_mvp già richiede esplicitamente di distinguere "confermata o ipotizzata" con metodo/evidenza/confidenza — è un requisito di design B2, non un fix di sicurezza preesistente da chiudere | Nessun fix preesistente richiesto; assicurarsi che il design B2 di `symbol_lineage_id` tratti "ipotizzato" come stato distinto e non-promuovibile senza evidenza, sullo stesso principio di CF-06 | No come blocker diretto — ma la decisione architetturale va presa nel design doc di B2, non durante l'implementazione |
| Supply-chain / Windows CI / fuzzing (SC-01/SC-02/SC-03) | Partially Mitigated/Not Implemented | Indiretta: B2 aggiunge dipendenze tree-sitter (parser multi-linguaggio, superficie FFI/unsafe potenzialmente ampia) | Advisory ignorato o parser non sicuro introduce vulnerabilità memory-safety non coperta dal solo CI Ubuntu | DEFERRED | Non blocca l'inizio della pianificazione B2; va riconsiderato quando le dipendenze tree-sitter concrete sono scelte (cargo deny le vedrà comunque) | Nessuno ora; ripetere `cargo deny check` e valutare CI multi-piattaforma quando le crate tree-sitter sono aggiunte | No, da ripetere quando le dipendenze concrete entrano in Cargo.toml |
| MCP/agent capability boundaries e prompt injection (AP-01..AP-06) | Not Implemented | Diretta ma differita: B2 include "server MCP di sola lettura" nello scope, quindi la superficie esiste dal momento in cui B2 espone l'API, non dall'inizio della pianificazione | Query/risposte di `graph_query`/`get_code_context` via MCP senza host identity/capability separation, o testo di codice sorgente con istruzioni che un futuro agente potrebbe seguire | PRE-B2 DESIGN DECISION (diventa BLOCKER al momento di esporre il server MCP, non al momento di pianificare l'indicizzazione) | L'audit stesso lo marca Not Implemented e "P1 prima di esposizione"; B2 come scope di *pianificazione* (grafo, parsing, API interne) non richiede ancora il server MCP attivo — ma il design deve prevederlo da subito, non aggiungerlo dopo | Minimo: nessuna implementazione del server MCP prima che read-only transport, host identity e allowlist capability siano definiti; l'API Rust interna (`graph_query`, `get_code_context`) può procedere senza bloccarsi su questo | No per iniziare la pianificazione/implementazione interna di B2; **sì** prima di accendere il server MCP di B2 |

## Verdetti richiesti esplicitamente

1. **PiiStore raw API confinement (FS-05)** — **DEFERRED** per B2. B2 come
   descritto in Spec_mvp non chiama `PiiStore`; il codice sorgente indicizzato
   non è PII e non passa da questo store. Intervento minimo: non un fix di
   codice ora, ma una riga nel contratto d'interfaccia ("nessun chiamante
   esterno a `storage` tocca `PiiStore` con un `&str` non passato da
   `NoteId::parse`") da rispettare se B2/B3 toccheranno mai questo store.

2. **Backup/restore provenance (BK-01/BK-02/DL-07)** — **BLOCKER B2**. B2
   aumenta sia il volume sia la natura del contenuto che `collect_vault_files`
   attraversa senza distinguere committato da piantato e senza cycle-guard sui
   symlink. Intervento minimo: raccolta backup derivata da una fonte
   autorevole (journal/Vault), non da una camminata raw; walk con
   `symlink_metadata` e rifiuto esplicito dei symlink prima che B2 inizi a
   collegare repository esterne reali al backup.

3. **Temporal/schema validation (CF-02/CF-03/ST-07)** — **PRE-B2 DESIGN
   DECISION**, non blocker diretto. B2 non scrive i tre campi Sensitive, ma
   eredita lo stesso pattern di causa radice (String non validata promossa a
   dato strutturato) per i propri campi nuovi (hash di commit, identificatori
   di lineage). Intervento minimo: fissare il confine Raw→Validated e la
   grammatica canonica PRIMA di definire i tipi `symbol_snapshot_id`/
   `symbol_lineage_id`, non dopo.

4. **AEAD AAD binding (CR-02)** — **DEFERRED**. Fuori dal perimetro di codice
   che B2 tocca (nessun uso di `PiiStore`/envelope PII nello scope B2
   descritto). Nessun intervento richiesto ora.

5. **SEC-001/002/003** — **BLOCKER B2** per SEC-001/SEC-002 (stesso
   meccanismo, stesso fix); SEC-003 si chiude come parte dello stesso lavoro,
   non come item separato. Motivazione: la reindicizzazione incrementale "per
   hash e invalidazione dopo cambio commit" di B2 introduce esattamente il
   pattern di scritture frequenti che amplifica la finestra TOCTOU già aperta
   sul write-path. Intervento minimo: containment/handle-relative I/O anche
   sul percorso di scrittura (`write_record_unchecked`'s pre-read), non solo
   sul percorso di lettura già protetto dall'hash check; i criteri di
   accettazione del fix devono includere esplicitamente la variante hard-link.

6. **Single-writer vs multi-writer (ST-03)** — **PRE-B2 DESIGN DECISION**,
   non blocker nel senso di "bug dimostrato" (è Not Tested, non Vulnerable).
   Motivazione: B2 è il primo punto del progetto dove un secondo scrittore
   concorrente (watcher/hook di reindicizzazione) diventa plausibile, non
   solo teorico. Intervento minimo: decidere esplicitamente se B2 opera nello
   stesso Vault-handle/processo (single-writer enforced da architettura, zero
   nuovo codice di lock) o se introduce un secondo scrittore — nel secondo
   caso, il lock cross-process deve esistere e avere test di barriera PRIMA
   che quel secondo scrittore venga scritto.

7. **DoS/resource budgeting (IX-04/IX-05)** — **BLOCKER B2** per le API
   esposte (`graph_query`, `get_code_context`), non per il lavoro interno di
   parsing/indicizzazione. Motivazione: `graph_query` è strutturalmente più
   incline a fan-out esponenziale di una ricerca testuale già misurata come
   vulnerabile; esporla senza budget peggiora una vulnerabilità nota.
   Intervento minimo: limite di profondità/numero-archi su `graph_query` e
   budget di tempo/record sulla reindicizzazione incrementale, da avere prima
   di esporre queste API anche solo al server MCP locale (non necessariamente
   prima di iniziare il parsing/grafo come lavoro interno).

## Elenco BLOCKER B2

- Backup/restore provenance: BK-01, BK-02 (e DL-07 di conseguenza)
- SEC-001 (TOCTOU write-path) + SEC-002 (cross-note write-path aliasing),
  con SEC-003 incluso nello stesso fix
- DoS/resource budgeting per le API esposte: IX-04, IX-05 — limitato alla
  superficie `graph_query`/`get_code_context`/server MCP, non al parsing
  interno

## Elenco PRE-B2 DESIGN DECISION

- DL-04: ancora anti-rollback indipendente o accettazione esplicita del
  modello FS-trusted
- Temporal/schema validation (CF-02/CF-03/ST-07): confine Raw→Validated e
  grammatica canonica, da fissare prima dei tipi `symbol_snapshot_id`/
  `symbol_lineage_id`
- Single-writer vs multi-writer (ST-03): architettura del reindexer
  incrementale
- Metadata classification policy (CF-06/CF-03) applicata a
  `symbol_lineage_id` "confermato/ipotizzato"
- MCP/agent capability boundaries (AP-01..AP-06): non blocca la
  pianificazione/implementazione interna, ma blocca l'accensione del server
  MCP di B2

## Elenco DEFERRED

- PiiStore raw API confinement (FS-05) — fuori perimetro codice B2
- AEAD AAD binding (CR-02) — fuori perimetro codice B2
- Supply-chain/Windows CI/fuzzing (SC-01/SC-02/SC-03) — da ripetere quando le
  dipendenze tree-sitter concrete entrano in Cargo.toml, non ora

## Ordine raccomandato dei fix

1. SEC-001/002/003 (write-path containment) — stesso codice
   (`write_record_unchecked`), stesso fix, massimo impatto su B2 perché
   B2 moltiplica le scritture concorrenti fin dal primo commit incrementale.
2. Backup/restore provenance (BK-01/BK-02) — blocca l'uso sicuro del backup
   su repository esterne reali appena B2 inizia a indicizzarle.
3. DoS/resource budgeting su `graph_query`/`get_code_context` — può
   procedere in parallelo ai primi due poiché riguarda il design dell'API,
   non lo storage; deve essere chiuso prima di esporre l'API, non prima di
   scriverne la logica interna.
4. Decisioni di design (non implementazione obbligatoria prima di B2):
   DL-04, grammatica temporale/Raw→Validated, single-writer vs multi-writer,
   policy lineage confermato/ipotizzato — da fissare nel design doc di B2
   prima di congelare gli schemi, non necessariamente da implementare subito.
5. MCP capability boundaries — da chiudere solo al momento di accendere il
   server MCP, non all'inizio della pianificazione B2.

## GO/NO-GO a iniziare la pianificazione B2

**GO condizionato.** La *pianificazione* di B2 (design doc, schema del
grafo, contratti `graph_query`/`get_code_context`, scelta tree-sitter) può
iniziare subito: nessuno dei blocker individuati impedisce il lavoro di
design, e le design-decision pre-B2 vanno comunque prese durante quella fase,
non prima di essa.

L'*implementazione* che toccherebbe scrittura concorrente sul Vault
(reindicizzazione incrementale), raccolta/backup di repository esterne, o
l'esposizione di `graph_query`/`get_code_context` oltre test locali, deve
attendere la chiusura dei tre BLOCKER elencati sopra (SEC-001/002/003,
BK-01/BK-02, budget su graph_query). Procedere con l'implementazione di B2
senza quei fix riuserebbe consapevolmente superfici già classificate
Vulnerable, ora con un moltiplicatore di probabilità (più scritture, più
contenuto raccolto, query strutturalmente più costose) che l'audit originale
non aveva ancora a disposizione.
