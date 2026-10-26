# brainCompany — Specifica MVP

Oct 23, 2026 · @Francesco

L'MVP dimostra una cosa sola: una memoria affidabile sul tuo codice e un agente che modifica un repository in modo controllato, con un utente, un agente e un modello sul laptop. Questa specifica è il contratto dell'MVP: dice cosa implementare adesso e come sapere che funziona, e non va ampliata; la visione completa resta nel documento di design. Aggiornata il 26/10/2026 con le correzioni di quattro revisioni esterne sulla specifica: dopo l'audit conclusivo il perimetro dell'MVP è congelato (un utente, un agente, un modello generativo alla volta, un progetto attivo alla volta). Da qui la specifica cambia solo per evidenze emerse da implementazione e test, senza nuove revisioni architetturali generali: la validazione ora è empirica.

## Scopo, confini e vincoli

L'MVP copre le fasi A–D del design; tutto il resto è rinviato finché non si misura che migliora il sistema.

| Dentro l'MVP | Fuori dall'MVP |
| --- | --- |
| Vault Markdown con schema versionato, provenienza, stati di verifica, scrittura atomica | Multiutente, ruoli, quote, equità tra utenti |
| Ricerca esatta (trigrammi), BM25, parsing tree-sitter, grafo del codice essenziale | Router con promozione automatica |
| Memoria con affermazioni, prove e relazioni | Revisore LLM sempre attivo, reranker su GPU |
| API del core e server MCP per VS Code | OCR multimodale, import di immagini e paper |
| Un modello principale locale via llama.cpp | Auto-ottimizzazione dei modelli, speculative decoding con modello draft |
| Un agente di coding con strumenti tipizzati, sandbox collaudata, worktree dedicata | Orchestratore multiagente, sentinella normativa autonoma |
| Test, linter, scanner di segreti, checkpoint | Web UI completa (basta una vista minima o la CLI) |
| Backup, restore e cancellazione testati | Accesso remoto da altri dispositivi |
| Benchmark privato per retrieval, risposte, astensione, task di coding |  |
| Memoria nel tempo: sessioni delle tue conversazioni, riassunti per giorno, settimana e mese, ricerca per periodo |  |

**Vincoli**:

- un solo utente umano, un progetto attivo per volta; si consulta un altro progetto solo su richiesta esplicita, e il cambio di contesto viene mostrato;
- elaborazione dei dati solo locale; la rete di acquisizione (dipendenze e modelli, con allowlist, versioni e hash bloccati) sta sull'host ed è separata dal runtime dell'agente, che resta offline;
- laptop con RTX 4090 da 16 GB di VRAM e 32 GB di RAM; un solo modello generativo caricato alla volta (embedding opzionali su CPU ammessi); le fasi A–C funzionano senza alcun modello caricato, che entra solo nella Fase D;
- solo licenze in una allowlist SPDX esplicita (Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC, Zlib), verificate sulla fonte del produttore; licenze multiple, dipendenze transitive e asset separati si valutano ciascuno; ogni eccezione è motivata e registrata; inventario di dipendenze e licenze salvato per ogni release;
- codice non fidato eseguito solo in una sandbox che ha superato la suite di test d'isolamento;
- ogni fase si chiude solo con l'evidenza dei suoi criteri di accettazione.

**Regola di sicurezza dell'MVP**: nessun output del modello, contenuto recuperato dal vault o dato proveniente dal repository costituisce di per sé un'autorizzazione. Le operazioni di scrittura e di esecuzione passano da strumenti tipizzati e dalle policy del core. Le operazioni sensibili richiedono un'approvazione esplicita vincolata alla proposta esatta. Il codice non fidato si esegue solo nel confine d'isolamento validato. In caso di stato ambiguo, errore di recovery o integrità non verificabile, il sistema fallisce in modo chiuso: non dichiara successo e non espone dati potenzialmente cancellati.

## Componenti e interfacce minime

Un solo processo Rust con confini di modulo rigorosi: niente servizi distribuiti, bus di messaggi o protocolli interni finché non servono.

| Crate | Responsabilità | Dipende da |
| --- | --- | --- |
| `domain` | Nota, schema, provenienza, stati di verifica, permessi, transizioni di stato | nessuno |
| `storage` | Vault Markdown, SQLite per eventi e registro di cancellazione, git, backup | `domain` |
| `index` | Trigrammi, BM25, grafo delle note, grafo del codice, embedding opzionali | `domain`, `storage` |
| `policy` | Gate, controllo dei permessi, validazione degli strumenti | `domain` |
| `agent` | Un agente, checkpoint, sandbox, client llama.cpp | `domain`, `policy`, `index` |
| `api` | REST locale e server MCP: solo trasporto | tutti i precedenti |

**API del core** (ogni chiamata passa da `policy`):

| Operazione | Ingresso | Uscita |
| --- | --- | --- |
| `search` | testo, filtri (tipo, stato, progetto, fiducia, intervallo di date), k | risultati con id, estratto, punteggio, provenienza |
| `retrieve` | id di nota o di chunk | contenuto con provenienza e stato |
| `graph_query` | id di partenza, tipi di relazione, profondità | nodi e archi con tipo e confidenza |
| `get_code_context` | simbolo o file, commit | definizione, riferimenti, chiamate, test collegati |
| `propose_write` | nota o modifica, versione attesa | esito del gate e id della proposta |
| `commit_write` | id della proposta | nuova versione, evento registrato, stato dell'indice |
| `recall_period` | espressione temporale ("due mesi fa", "a settembre") o intervallo esplicito, progetto | intervallo risolto, riassunto del periodo, sessioni e decisioni citate |

**Contratto comune a ogni operazione**: schema di ingresso e uscita versionato, errori tipizzati, permessi richiesti, timeout e limiti, comportamento in caso di retry, idempotenza, identificatore di correlazione, log senza segreti. Dettagli per operazione:

- `search`: modalità temporale esplicita, progetto di contesto, significato del punteggio;
- `retrieve`: contenuto con versione, provenienza e stato;
- `graph_query`: profondità massima e numero massimo di nodi restituiti;
- `get_code_context`: commit esplicito o risolto, chiamate statiche distinte da quelle inferite;
- `propose_write`: non modifica nulla, crea una proposta con hash del contenuto e versione attesa;
- `commit_write`: verifica approvazione, hash e versione; idempotente sullo stesso `operation_id`;
- `recall_period`: intervallo risolto, convenzione temporale usata, fonti e ambiguità segnalate.

**Server MCP per VS Code**: espone `search`, `retrieve`, `graph_query`, `get_code_context` e `recall_period` come strumenti di sola lettura. Nell'MVP il server MCP resta di sola lettura: propose\_write e commit\_write sono invocabili solo tramite un'interfaccia locale separata del core (la CLI), soggetta alle stesse policy e al canale di approvazione C2; la scelta si rivede solo dopo test espliciti della superficie di autorizzazione. Esporre uno strumento non equivale a un'interazione umana, quindi le approvazioni funzionano così:

- `propose_write` crea solo una proposta;
- l'approvazione è legata a `proposal_id`, hash del contenuto e versione attesa, e viene data da te attraverso il canale definito nel contratto C2, non dal modello;
- il core rifiuta approvazioni riutilizzate, scadute o relative a una proposta modificata dopo l'approvazione;
- un testo del modello o una nota recuperata non vale mai come conferma;
- ogni approvazione registra proposta, versione e istante;
- approvare l'avvio di un task è distinto dalle approvazioni richieste durante il task.

**Schema minimo di una nota**:

```yaml
id: bug-2026-10-23-fastapi-cors
schema_version: 1
type: bug              # code | bug | decision | pattern | research | session | summary
status: draft          # draft | test_passed | reviewed | verified | superseded | contradicted
trust: internal        # internal | official | web
classification: internal      # public | internal | sensitive (secret vietato nel vault)
project: project-x
source: agent:coder@prompt-v3
created: 2026-10-23
updated: 2026-10-23
event_period: 2026-10-23    # quando è avvenuto ciò di cui parla la nota (data o intervallo)
relations:
  supports: []
  contradicts: []
  derived_from: []
  supersedes: []
  applies_to: []
```

I campi specifici per tipo (es. per `bug`: causa, commit difettoso, commit corretto, test di regressione) si aggiungono come estensione dello schema, con la sua versione.

**Progetti e repository**: un progetto contiene uno o più repository; ogni risultato di ricerca, citazione e nodo del grafo porta `project_id`, `repository_id` e commit. "Un progetto attivo alla volta" non significa un solo repository.

## Fase A — Vault affidabile

Un vault che crea, modifica, cancella e ripristina note in modo corretto e verificabile, senza dipendere da nessun modello AI.

**Consegne**:

1. Repository con workspace Rust (`domain`, `storage`, `policy`), CI con test, mutation testing (cargo-mutants), cargo-deny, scanner di segreti e controlli di architettura.
2. Schema versionato delle note e validatore strutturale.
3. Scrittura recuperabile: file, SQLite, git e indice non condividono una transazione atomica, quindi ogni scrittura è una macchina a stati persistente (con gli stati del contratto C1: intent\_recorded → content\_persisted → journal\_committed → git\_committed → index\_current, più recovering e failed) con un operation\_id univoco. L'intento si registra nel journal prima di applicare la modifica, e il completamento solo dopo averne verificato gli effetti. Ogni passo di recovery è idempotente. Al riavvio le operazioni incomplete si riconciliano prima di esporre le API; se la riconciliazione non riesce, il sistema si ferma in uno stato dichiarato inconsistente invece di dichiarare successo.
4. Registro degli eventi e commit git sincrono per ogni scrittura esplicita del vault, con i livelli di completamento del contratto C1. Fonti di verità dichiarate: i file Markdown per il contenuto, il journal SQLite per eventi e stato delle operazioni, git per la cronologia; l'indice è derivato e ricostruibile, e non viene mai dichiarato corrente se è indietro.
5. Store cifrato dei dati personali con chiave per record e registro di cancellazione monotono e separato. Da specificare nell'implementazione: quali record e metadati si cifrano, dove stanno le chiavi avvolte, come si recupera la chiave principale e cosa succede se va persa. Dati personali sensibili e segreti non entrano nella cronologia git per impostazione predefinita; se ci entrano per errore, la procedura dichiara come si gestiscono storia, backup e copie residue. Il registro di cancellazione è conservato separatamente dai backup che potrebbe invalidare, con integrità e autenticità protette; un restore applica prima le cancellazioni più recenti e solo dopo diventa consultabile; se il registro è assente, corrotto o non verificabile, i dati potenzialmente cancellati non vengono esposti.
6. Backup cifrato su disco esterno non sempre collegato, con controllo di integrità e procedura di restore. Il disco scollegato dopo ogni backup è la copia che l'host non può modificare; il riferimento affidabile è autenticato con una chiave che non sta nei backup; la rotazione delle chiavi si definisce nel runbook prima di A3. Nessuna protezione è promessa contro un host già compromesso con i privilegi dell'utente.

**Test da scrivere per primi**:

- [ ] Una nota valida isolata viene accettata; una nota con frontmatter non valido viene rifiutata con un errore che indica il campo.
- [ ] Un link interno rotto (broken) viene rifiutato; un riferimento pending (fonte esterna, nota non ancora importata, simbolo non risolto) viene accettato e segnalato; una nota senza link (unlinked) viene accettata.
- [ ] Il gate blocca i segreti ad alta confidenza secondo una policy versionata; i dati personali seguono una policy distinta di classificazione, minimizzazione e cifratura; le eccezioni sono esplicite, motivate e registrate. Test per segreto reale, esempio fittizio, falso positivo con eccezione, e segreti in note derivate, riassunti, log e indici.
- [ ] Due scritture concorrenti sulla stessa versione: la seconda fallisce con conflitto esplicito, nessun aggiornamento perso.
- [ ] Crash simulato e letture concorrenti in ogni casella della matrice stato × guasto del contratto C1, incluso il retry dello stesso operation\_id: al riavvio il sistema recupera oppure si ferma in uno stato dichiarato inconsistente, mai con un falso successo.
- [ ] Migrazione dello schema da v1 a v2: coppie di esempi "nota vecchia → nota attesa", migrazione idempotente; reversibile quando possibile, altrimenti dichiarata con perdita (lossy) e preceduta da un backup verificato.
- [ ] Cancellazione di un dato personale: il dato non è più leggibile, e non riappare dopo il restore di un backup precedente alla cancellazione.
- [ ] Restore completo da backup su una cartella vuota: contenuto identico, hash verificati.
- [ ] Il registro di cancellazione non perde voci in seguito a un restore.

* [ ] Retry della stessa richiesta dopo un timeout, e replay di una richiesta già completata: nessuna doppia applicazione.
* [ ] Restore di un backup precedente a una cancellazione: il dato non riappare in file, database, indici, embedding, cache, riassunti, log e chiavi.
* [ ] Registro di cancellazione assente o corrotto durante un restore: il sistema non espone i dati ripristinati e lo segnala.
* [ ] Un dato personale marcato sensibile non finisce in nessun commit git.

- [ ] Git non disponibile durante una scrittura: la risposta è `git_pending`, non un successo, e il commit viene ripreso.
- [ ] Registro di cancellazione valido ma obsoleto rispetto al riferimento: il restore non espone i dati potenzialmente cancellati.
- [ ] Un file con dato `sensitive` o `secret` non supera né il controllo del core né l'hook pre-commit.

* [ ] Letture concorrenti durante ogni passaggio di una scrittura: nessuna lettura tramite API vede una versione non ancora linearizzata.
* [ ] Retry dello stesso `operation_id` in ogni stato: restituisce l'esito già raggiunto, senza doppia applicazione.
* [ ] Registro di cancellazione: crash prima e dopo ogni passo del protocollo C5, rollback di uno solo tra registro e riferimento, perdita della copia offline.
* [ ] Un dato identificativo nel titolo, nel nome del file o nel messaggio di commit viene rilevato e sostituito da un identificatore opaco.

- [ ] Retry di un'operazione terminale `failed` con lo stesso `operation_id`: restituisce `failed` senza rieseguire; nuovo `operation_id` con versione attesa ancora valida: applicato; nuovo `operation_id` con versione superata: conflitto; timeout seguito da richiesta duplicata: nessuna doppia applicazione.
- [ ] Errore persistente nell'aggiornamento del riferimento durante una cancellazione: dato non consultabile, stato di riconciliazione esposto, anche dopo riavvii ripetuti, letture concorrenti e un restore durante la finestra `pending`.

**Criteri di accettazione**:

| Criterio | Soglia |
| --- | --- |
| Test della fase | 100% verdi in CI |
| Mutation testing su `domain`, `policy`, `storage` | Ogni mutante sopravvissuto esaminato e giustificato o coperto da un nuovo test |
| Restore da backup | Riuscito da zero su una macchina o cartella pulita, tempo misurato e registrato |
| Cancellazione dopo restore | Nessun dato cancellato riappare |
| Dipendenze | Nessuna vulnerabilità nota aperta applicabile; ogni eccezione (non applicabile o senza correzione disponibile) è motivata, limitata nel tempo e registrata; licenze solo nella allowlist |

## Fase B — RAG sui repository

Trovare nei tuoi repository le parti di codice e le note giuste, con citazioni verificabili, e dire "non presente" quando la risposta non c'è.

**Consegne**:

1. Indicizzatore con tree-sitter per Python, C#, C++, JavaScript/TypeScript, HTML/CSS e Rust.
2. Indice a trigrammi per ricerche esatte e BM25; la Fase B deve passare con la sola baseline deterministica; gli embedding su CPU entrano solo se migliorano il benchmark senza peggiorare latenza e RAM oltre i limiti.
3. Grafo del codice essenziale: simboli con due identificativi distinti: symbol\_snapshot\_id (repository, linguaggio, percorso, nome qualificato, firma: l'identità dentro un commit) e symbol\_lineage\_id (la continuità tra commit, confermata o ipotizzata, con metodo, evidenza e confidenza; una rinomina solo inferita non è un'identità certa), riferimenti, import, collegamenti ai test, hash del commit; chiamate risolte separate da quelle ipotizzate. Ogni arco conserva tipo, metodo di risoluzione, commit di riferimento ed evidenza; i riferimenti a righe non sono identificatori stabili.
4. Reindicizzazione incrementale per hash e invalidazione dei riferimenti a righe dopo un cambio di commit.
5. Crate `api` con `search`, `retrieve`, `graph_query`, `get_code_context` e server MCP di sola lettura per VS Code.
6. Set di riferimento in `eval/rag/`, diviso in sviluppo, calibrazione e test finale, stratificato, ed escluso dal corpus interrogabile (non dal runner, che deve leggere le risposte attese per controllarle).

**Ricerca per periodo**: filtro `time_range` su `search` con una modalità esplicita, `created_at`, `updated_at`, `event_period` oppure `any`. Il sistema sceglie la modalità dalla domanda e la mostra: "di cosa parlavamo ad agosto?" usa il periodo delle sessioni e degli eventi, "quali note ho modificato ad agosto?" usa `updated_at`. Le espressioni temporali si risolvono in un intervallo esplicito rispetto alla data di oggi, sempre mostrato con il campo usato ("ho cercato tra il 20/08 e il 03/09 sul periodo delle sessioni"). La traduzione delle espressioni in intervalli segue il contratto C4 dell'appendice. Espressioni legate a eventi ("prima della migrazione") si risolvono solo se l'evento è identificato nel vault; altrimenti il sistema chiede.

**Test da scrivere per primi**:

- [ ] La ricerca di un identificatore esatto (es. `get_user_by_id`) lo trova anche se compare una sola volta.
- [ ] Dopo la rinomina di una funzione e un nuovo commit, i vecchi riferimenti risultano invalidati e quelli nuovi presenti.
- [ ] `get_code_context` su una funzione restituisce definizione, chiamanti noti e test collegati.
- [ ] Una domanda senza risposta nel corpus produce "non presente", non un'invenzione.
- [ ] Ogni risposta cita gli id delle note o dei chunk da cui viene.
- [ ] Un file di un progetto non attivo non compare nei risultati, a meno di una richiesta esplicita di consultare quel progetto, che il sistema rende visibile.

* [ ] "Due mesi fa", con una data di riferimento passata come parametro del test, viene risolto nell'intero mese di calendario previsto da C4, mostrato nella risposta con la precisazione che non equivale agli ultimi 60 giorni.
* [ ] Con un filtro per periodo non compaiono note fuori dall'intervallo.
* [ ] Una nota creata oggi su un evento di agosto viene trovata cercando agosto (conta `event_period`, non solo la data di creazione).

- [ ] Una domanda temporale ambigua, o legata a un evento non identificato, produce una richiesta di chiarimento.
- [ ] "Di cosa parlavamo ad agosto?" e "quali note ho modificato ad agosto?" usano modalità temporali diverse, mostrate nella risposta.
- [ ] Contaminazione: il sistema non recupera le risposte attese né il testo delle domande di valutazione attraverso alcun indice o memoria, comprese copie indirette in cache, log, sessioni, riassunti, note importate e indicizzazioni precedenti; le etichette di riferimento sono leggibili solo dal runner. Il test vale anche dopo un'esecuzione precedente del benchmark: percorsi del runner isolati, cache di valutazione invalidate, hash del corpus registrato, e le sessioni di valutazione non diventano mai memoria ordinaria.
- [ ] Riferimenti al codice corretti dopo rinomina, rebase e cambio di commit.
- [ ] Conferma MCP rifiutata se la proposta è stata modificata dopo l'approvazione, o se l'approvazione è riutilizzata o scaduta.
- [ ] Una nota con istruzioni che tentano di autorizzare una scrittura non ottiene alcuna scrittura.

**Cosa misura la Fase B**: solo la ricerca deterministica e, se presenti, risposte estrattive con citazioni, con formato e regole fissati prima del benchmark. Le metriche di generazione libera, astensione del modello e latenza al primo token si applicano nella Fase D, quando entra il modello generativo. I due protocolli usano set e soglie versionati separatamente.

**Criteri di accettazione** (ogni soglia si fissa e si motiva sul set di sviluppo prima di guardare il test finale; qualità della ricerca, supporto delle citazioni, correttezza della risposta e astensione si misurano separatamente; prima del test finale soglie e configurazione si congelano registrando commit, data, versione e hash del dataset, protocollo e motivazione; se una soglia cambia dopo aver visto il test finale, quel risultato non vale più come prova indipendente):

| Criterio | Misura |
| --- | --- |
| Ricerca | Recall@10 e nDCG@10 sul test finale, con intervalli di confidenza e soglia minima motivata |
| Citazioni | Correttezza dei riferimenti recuperati: un risultato è corretto solo se contiene davvero l'elemento cercato, non se parla dello stesso tema; copertura; protocollo di annotazione, campionamento e gestione dei casi dubbi definiti prima; soglia minima |
| Astensione | Valutate insieme: "nessuna evidenza" corretto sulle domande senza risposta, falsi "nessuna evidenza" sulle domande risolvibili e recall su queste ultime, così nessuna soglia si soddisfa restituendo sempre zero risultati |
| Latenza | Tempo al primo risultato da VS Code, P50 e P95 |
| Uso reale | Usi il server MCP in VS Code sul lavoro di tutti i giorni per almeno due settimane |

## Fase C — Memoria con prove

Bug, decisioni e pattern diventano conoscenza riutilizzabile solo con prove riproducibili, e una prova smentita segnala tutto ciò che ne dipende.

**Consegne**:

1. Tipi `bug`, `decision`, `pattern` con i campi specifici e gli stati `test_passed`, `reviewed`, `verified`, `superseded`, `contradicted`.
2. Relazioni `supports`, `contradicts`, `derived_from`, `supersedes`, `applies_to` e propagazione delle revisioni.
3. Provenienza e taint tracking: ogni dato derivato eredita il livello di fiducia più basso delle sue fonti.
4. Requisiti di `verified` per tipo applicati dal gate.

**Provenienza e verifica sono proprietà diverse**: `trust` indica da dove viene un dato, non se è vero; `status` indica a che punto è il processo di verifica. Una fonte ufficiale può essere autentica ma non applicabile, superata o interpretata male; una nota interna può contenere una prova riproducibile. Quindi:

- l'ordine di fiducia è definito in modo formale (`web` < `internal` < `official`), non confrontando stringhe;
- un dato derivato eredita il livello più basso delle sue fonti e conserva la catena di derivazione fino alla fonte originale, al chunk o al file e commit;
- nessuna nota diventa `verified` perché proviene da una fonte `official`;
- i conflitti tra fonti e prove sono rappresentati in modo esplicito, non risolti scegliendo la fonte di livello più alto.

**Criteri di `verified` per tipo**:

| Tipo | Criterio |
| --- | --- |
| `bug` | Commit difettoso, commit corretto e test di regressione riproducibile che fallisce sul primo e passa sul secondo |
| `decision` | Decisione esplicita, motivazione, ambito, data, eventuale decisione che la sostituisce |
| `pattern` | Esempi reali, limiti di applicabilità, test o casi d'uso |
| `research` | Fonti con data, affermazioni sostenute da ciascuna fonte, incertezze dichiarate |
| `session`, `summary` | Derivazione completa dalle fonti; non diventano mai prova tecnica per il solo fatto di essere coerenti o ripetuti |

Quando una prova viene contraddetta si segnalano le dipendenze senza cancellare la storia: una decisione superata non viene restituita come attuale, ma resta consultabile come storia.

**Memoria nel tempo**, fondamentale per i progetti lunghi ripresi a distanza di mesi:

- **Sessioni delle tue conversazioni**: a fine sessione (da VS Code o CLI) una nota `session` con cosa hai chiesto, cosa è stato deciso, cosa è rimasto aperto e quali note sono state toccate; nella Fase C la bozza si assembla in modo deterministico dagli eventi (richieste, note toccate, decisioni registrate) e tu la completi o confermi; una bozza scritta da un modello arriva solo dopo la Fase D.
- **Riassunti per periodo**: note `summary` giornaliere, settimanali e mensili per progetto, assemblate in modo deterministico dalle sessioni confermate (decisioni, questioni aperte, note toccate), senza dipendere da un modello generativo; la generazione con un LLM è opzionale, si abilita solo dopo la Fase D e non cambia provenienza né stato di verifica delle fonti. Sono collegate alle sessioni con `derived_from`; un riassunto non è mai una prova, rimanda sempre alle sessioni originali. Ogni riassunto conserva fonti, decisioni esplicite, questioni aperte, ipotesi non confermate, versione del generatore e stato di aggiornamento; se la rigenerazione fallisce dopo una cancellazione, risulta non aggiornato e non viene presentato come corrente. La tua conferma attesta che il resoconto è accettabile, non che ogni affermazione tecnica sia verificata.
- **Ripresa di un progetto**: "riprendi project-x" restituisce l'ultimo riassunto mensile, le decisioni ancora valide, le questioni aperte e i checkpoint non chiusi, con le citazioni.
- **Cancellazione**: le sessioni sono memoria personale; si possono cancellare, e la cancellazione si propaga ai riassunti che ne derivano.

**Test da scrivere per primi**:

- [ ] Un `bug` non diventa `verified` se manca il test che fallisce sul commit difettoso e passa su quello corretto.
- [ ] Marcare una prova come `contradicted` segnala per revisione tutte le note che la usano in `supports`.
- [ ] Un riassunto costruito da una nota `trust: web` e una `internal` risulta `web`.
- [ ] Nessuna nota diventa `verified` per la sola presenza di metadati validi o di molti collegamenti.
- [ ] Una decisione `supersedes` un'altra: la vecchia passa a `superseded` e non viene più restituita come attuale.

* [ ] "Di cosa parlavamo tra il 20/08 e il 03/09?" restituisce le sessioni giuste di quel periodo, con riassunto e citazioni.
* [ ] Il riassunto mensile cita tutte le sessioni da cui deriva, e nessuna sessione fuori dal mese.
* [ ] "Riprendi project-x" dopo mesi di pausa elenca decisioni valide (non quelle superate), questioni aperte e checkpoint non chiusi.
* [ ] Cancellando una sessione, i riassunti che ne derivano vengono rigenerati senza il suo contenuto.

- [ ] Una fonte `official` contraddetta da una prova riproducibile genera un conflitto visibile, non una prevalenza automatica della fonte.
- [ ] Una decisione superata non compare tra quelle attuali ma resta consultabile chiedendo la storia.
- [ ] Se la rigenerazione di un riassunto fallisce dopo una cancellazione, il riassunto risulta non aggiornato.

**Criteri di accettazione**: tutti i test della fase verdi; su un campione di bug reali dei tuoi repository, ogni nota `verified` ha la coppia di commit e il test riproducibile; nessuna promozione basata sulla confidenza di un modello.

## Fase D — Un agente di coding

Un agente che completa task reali sui tuoi repository lavorando a checkpoint, senza poter toccare i test e tornando a uno stato consistente dopo un errore.

**Consegne**:

1. Sandbox Linux isolata che supera la suite di test d'isolamento (prerequisito bloccante): rete spenta, limiti di CPU, RAM, disco, processi e tempo; nessun montaggio di home, credenziali git, chiavi del vault, socket Docker o cartelle condivise scrivibili non indispensabili. Prima di implementare l'agente si sceglie il confine d'isolamento e si scrive un threat model minimo: codice ostile da contenere, dati da proteggere, capacità negate. Il blocco della rete si applica al confine d'isolamento, non per convenzione dell'agente. Test, script di build, hook git e installatori sono codice non fidato. Superare la suite dimostra il rispetto dei test definiti, non la sicurezza assoluta; la Fase D resta bloccata finché l'isolamento non è validato.
2. Worktree git dedicata per ogni task; commit a ogni checkpoint verde.
3. Checkpoint con test prima del codice: test scritti dal goal e rivisti; i test di accettazione stanno fuori dalla worktree modificabile, girano tramite un runner controllato dall'orchestratore, e i loro hash e la loro provenienza si verificano prima e dopo il task; test di accettazione, test scritti dall'agente e test preesistenti restano separati; rosso, verde, refactoring; mutation testing.
4. Strumenti tipizzati negati per default (leggere file, scrivere nella worktree, eseguire i test, cercare nel cervello); comandi mai passati come stringhe alla shell; eseguibili e directory di lavoro da allowlist, argomenti validati, timeout e limiti di output e risorse, perché anche un elenco di argomenti è rischioso se permette programmi o opzioni arbitrarie: il modello non sceglie il binario da eseguire; gli strumenti di scrittura dell'agente sono interfacce interne del core, soggette a policy, sandbox e approvazioni, e non sono esposti dal server MCP di sola lettura; installazione di pacchetti nuovi solo con tua approvazione.
5. Stato del task con `last_committed_checkpoint` e `current_attempt`; il tentativo incompleto resta come artefatto recuperabile ma non valido. Un checkpoint contiene commit git, hash dei file, stato dei test, configurazione del modello, log essenziali e stato del task; la ripresa verifica la coerenza della worktree prima di continuare.
6. Limiti per task (tempo, token, chiamate, tentativi, scritture, dimensione di output e log): a budget esaurito l'agente si arresta, lascia la worktree recuperabile e produce un report riproducibile. Interruttore di emergenza nel core: annullare la richiesta al modello non basta, quindi cancella il task, termina l'intero process group o container, blocca nuove chiamate agli strumenti e registra l'esito, entro un tempo massimo di arresto e con un comportamento definito se l'arresto fallisce.
7. Benchmark end-to-end per scegliere quantizzazione e configurazione del modello: per ogni artefatto URL canonico del produttore, hash, licenza della versione esatta, formato, quantizzazione e data di acquisizione, distinguendo i pesi del produttore dai GGUF convertiti o quantizzati da terzi; si misurano picco di VRAM e RAM, velocità, latenza al primo token, qualità sui task reali e stabilità su task lunghi, lasciando margine per il sistema.

**Test da scrivere per primi**:

- [ ] Catalogo versionato di scenari d'isolamento, verificabili uno per uno, almeno: tentativi di evasione, lettura di file fuori dal progetto, accesso a socket, processi figli, riempimento del disco, persistenza dopo la distruzione.
- [ ] L'agente che prova a modificare un test bloccato riceve un errore e il tentativo viene registrato.
- [ ] Crash simulato a metà checkpoint: la ripresa parte dall'ultimo checkpoint consolidato e il tentativo incompleto è recuperabile.
- [ ] Superato il numero massimo di tentativi (partenza: 3–4), il task si ferma e arriva in coda a te con il referto.
- [ ] Una richiesta di installare un pacchetto non presente nel lockfile si blocca in attesa di approvazione.
- [ ] Una nota avvelenata nel RAG con istruzioni non riesce a far eseguire un'azione all'agente.
- [ ] L'interruttore di emergenza ferma l'agente anche se il modello è bloccato.

* [ ] Tentativo di modificare runner, configurazione dei test, hook git o file importati dai test: rilevato dagli hash e il task viene respinto.
* [ ] Interruttore di emergenza con un comando bloccato e processi figli attivi: tutta la catena termina entro il tempo massimo.
* [ ] Budget esaurito: task arrestato, worktree recuperabile, report prodotto.
* [ ] Modello assente o output malformato: il core resta consistente e il task si ferma con un errore chiaro.
* [ ] Una nota avvelenata tenta di far leggere file fuori dal progetto: l'accesso viene negato.

- [ ] Dalla sandbox: invocazione diretta di `bc approve`, accesso al socket di approvazione, replay di un'approvazione già consumata: tutti respinti.

**Criteri di accettazione**:

| Criterio | Misura |
| --- | --- |
| Task reali | Su un set di task presi dai tuoi repository: quota completata, tentativi medi, tempo totale per task |
| Test intatti | Nessuna modifica a test di accettazione, runner, configurazione e hook, verificata con gli hash; un task si accetta solo dopo il controllo di diff, test, lint e scanner, non per i soli test passati |
| Recupero | Ogni crash simulato si chiude in uno stato consistente |
| Sicurezza | Suite d'isolamento e test con note avvelenate superati |
| Risorse | Budget numerici di VRAM e RAM, con margine per il sistema, fissati prima del benchmark e rispettati nelle misure; stabilità su task lunghi |
| Risposte generate | Fedeltà delle citazioni per singola affermazione, astensioni corrette, risposte non supportate, astensioni eccessive, latenza al primo token; set e soglie versionati separatamente da quelli della Fase B |

## Appendice contrattuale

Sei contratti da chiudere prima del codice che ne dipende: C1 e C3 prima di A1, C5 prima di A3, C4 prima di B1, C6 prima di C1, C2 prima di D1.

### C1 — Quando una scrittura è completata

| Livello | Significato | Dopo un crash |
| --- | --- | --- |
| `content_persisted` | File Markdown scritto e sincronizzato su disco | Riconciliato col journal |
| `journal_committed` | Evento registrato nel journal con hash del contenuto | Punto di linearizzazione: da qui la scrittura esiste |
| `git_committed` | Commit git creato con l'`operation_id` | Rieseguito in modo idempotente |
| `index_current` | Indice aggiornato a questa versione | Ricostruito; finché è indietro le risposte lo dichiarano |

**Stati di una scrittura**: `intent_recorded` → `content_persisted` → `journal_committed` → `git_committed` → `index_current`, più `recovering` e `failed`.

- L'intento si registra e si sincronizza nel journal prima di toccare il contenuto.
- Il contenuto si scrive in un file temporaneo, si sincronizza, poi, dopo aver salvato in modo durabile una before-image della versione precedente, si rinomina sul file definitivo e si sincronizza la cartella; l'ordine esatto delle sincronizzazioni (file, cartella, journal) si fissa e si testa sulla piattaforma scelta, senza promettere un'atomicità distribuita.
- La nuova versione non è visibile tramite API finché non raggiunge `journal_committed`, punto di linearizzazione logica; le letture concorrenti vedono solo versioni committate: le API leggono la versione indicata dal journal, verificata per hash (il file definitivo oppure la before-image), mai il file più recente per percorso. Uno strumento esterno che legge direttamente il vault può vedere un file non ancora linearizzato: è un limite dichiarato, e la riconciliazione al riavvio lo risolve. Tutti i componenti interni (indicizzatore, backup, server MCP) leggono solo versioni committate tramite il core, mai con una scansione libera dei percorsi durante una scrittura.
- `commit_write` restituisce successo solo dopo `git_committed`; se git fallisce restituisce `git_pending`, che non è uno stato persistito ma il codice di risposta per un'operazione rimasta a journal\_committed senza git\_committed, ripresa al riavvio. L'indice è derivato, può aggiornarsi in modo asincrono ed è riportato a parte (`index_current` oppure `index_stale`). L'API non dichiara mai uno stato più forte di quello raggiunto.

| Stato al crash | Recovery |
| --- | --- |
| `intent_recorded` | Roll-back: si scarta il temporaneo, l'operazione risulta `failed`, il chiamante può riprovare solo con un nuovo operation\_id |
| `content_persisted` | Si verifica l'hash: se coincide roll-forward fino a `journal_committed`, altrimenti si ripristina la before-image e se ne verifica l'hash; se anche questa non è affidabile, stato inconsistente senza esporre una versione incerta |
| `journal_committed` o `git_pending` | Roll-forward: commit git idempotente con l'`operation_id` |
| `git_committed` | Si ricostruisce l'indice se è indietro |

Un retry con lo stesso `operation_id` restituisce l'esito già raggiunto, senza riapplicare. I commit dell'agente avvengono alla chiusura dei checkpoint.

**`failed` è terminale**: un retry con lo stesso `operation_id` restituisce lo stesso fallimento senza riapplicare la modifica. Per riprovare un'operazione fallita il chiamante crea un nuovo `operation_id` e ripete la verifica della versione attesa. Un'operazione recuperabile (ad esempio rimasta a `journal_committed`) continua invece con il proprio identificatore originario.

**Before-image**: prima del rename il core conserva in modo durabile la versione precedente (in alternativa, file di contenuto immutabili con un puntatore versionato alla versione corrente). Il journal registra hash e riferimenti di entrambe le versioni. La before-image si elimina solo dopo `git_committed`.

**Matrice stato × guasto** (unica fonte dei test di crash della Fase A):

| Momento del guasto | Esito atteso |
| --- | --- |
| Dopo `intent_recorded`, prima della scrittura | Operazione `failed`, nessun file toccato |
| Dopo il rename, prima di `journal_committed` | Hash della nuova versione verificato: roll-forward se valido, altrimenti ripristino esatto della before-image (stesso hash e versione) |
| Nuova versione con hash non valido | Ripristino della before-image |
| Durante il ripristino della before-image | Il ripristino riparte in modo idempotente; se la before-image è illeggibile, stato inconsistente |
| Dopo `journal_committed`, prima di `git_committed` | Roll-forward del commit git |
| Dopo `git_committed`, prima di `index_current` | Ricostruzione dell'indice, nel frattempo `index_stale` |
| Lettura concorrente in ciascuno dei momenti sopra | L'API restituisce sempre la versione committata, anche se il file definitivo è già sostituito |
| Retry dello stesso `operation_id` in ciascuno stato | Esito già raggiunto, nessuna doppia applicazione |

### C2 — Canale di approvazione umana

- Un **adapter di approvazione** nel core genera la richiesta con `proposal_id`, hash esatto, versione attesa, operazione concreta e progetto attivo; l'approvazione ha identità del chiamante, scadenza (proposta: 10 minuti), consumo singolo e audit trail.
- **Canale attendibile nell'MVP**: una CLI locale del core (`bc approve <proposal_id>`, che mostra il diff prima di confermare), che il modello non può pilotare.
- La conferma nativa del client MCP in VS Code si ammette solo dopo aver verificato che il modello non può generarla né falsificarla; fino ad allora vale solo la CLI.
- Un testo "approvato" scritto dal modello o trovato in una risorsa del RAG non autorizza nulla.
- Approvare l'avvio di un task non autorizza automaticamente modifiche o installazioni successive: ciascuna segue la sua policy e, se sensibile, chiede una propria approvazione.

**Confine di fiducia della CLI**: la CLI di approvazione gira sull'host con l'account dell'utente, fuori dalla sandbox. Il processo dell'agente non può eseguire `bc approve`, né raggiungere il canale di conferma, i token di approvazione o i relativi socket: non sono montati né raggiungibili dalla sandbox. Il core verifica ogni approvazione attraverso un canale e un'identità separati da quelli usati dall'agente (ad esempio un socket locale con controllo delle credenziali del processo chiamante). Test: invocazione diretta della CLI dalla sandbox, accesso al socket, replay di un'approvazione già consumata.

**Identità del processo non è presenza umana**: un controllo sulle credenziali del processo non distingue una tua conferma da un'invocazione automatica di un altro processo con il tuo stesso account. Per questo `bc approve` richiede una conferma interattiva nel terminale (mostra il diff e chiede di digitare una parte dell'hash della proposta), e registra il canale usato. Threat model dichiarato: l'account utente sull'host è considerato fidato; un malware che gira con i tuoi privilegi è fuori dalla garanzia del canale di approvazione e si mitiga con la copia di backup non modificabile dall'host e con la scansione delle dipendenze.

### C3 — Classificazione dei dati e git

- Classi: `public`, `internal`, `sensitive` (dati personali), `secret` (chiavi, token, password).
- Confini a cui si applica: file, frontmatter e metadati, allegati, file generati, cache, log, indici, embedding, riassunti, backup.
- `sensitive` e `secret` non entrano mai nei percorsi tracciati da git: i dati personali stanno solo nello store cifrato con riferimento `pii://`, i segreti non si salvano nel vault.
- Doppio controllo, perché `.gitignore` non basta: nel core sul percorso di scrittura, e in un hook pre-commit con scanner di segreti e dati personali; in più una scansione periodica della cronologia, con data registrata.
- Se un segreto entra comunque nella cronologia: revoca e rotazione immediata, bonifica della cronologia (es. git-filter-repo), dei backup e delle copie note, e registrazione dell'incidente.

**Anche i metadati**: titoli delle note, slug, nomi di file, percorsi, messaggi di commit, estratti, provenienza, log e messaggi d'errore possono contenere dati identificativi. Si classificano e si sanificano come il contenuto; i riferimenti sensibili usano identificatori opachi. I campi restituiti da `search`, `retrieve`, `graph_query` e dagli errori sono verificati da test dedicati.

**Un solo campo canonico**: la classificazione è il campo `classification: public | internal | sensitive` del frontmatter, validato dal gate; `secret` è una classe della policy ma è vietata nel vault, quindi una nota classificata `secret` viene sempre rifiutata. `trust` resta una proprietà indipendente dalla sensibilità.

### C4 — Date relative

Intervalli chiusi all'inizio e aperti alla fine, nel fuso Europe/Rome, mostrati sempre con il fuso e con il campo temporale usato. Quando la convenzione può sorprendere, la risposta la dichiara ("due mesi fa = intero mese di agosto, non gli ultimi 60 giorni"). La data di riferimento è un parametro, anche nei test. Se l'intento è ambiguo, il sistema chiede conferma.

| Espressione | Intervallo (oggi 26/10/2026) |
| --- | --- |
| "ad agosto", "nel mese di agosto" | 01/08/2026 – 01/09/2026, l'agosto più recente non futuro; se il vault copre più anni con contenuti in agosto, chiede l'anno |
| "il mese scorso" | L'intero mese di calendario precedente: settembre 2026 |
| "due mesi fa" | L'intero mese di calendario di due mesi prima: agosto 2026 |
| "intorno al 23 agosto" | dal 16/08 incluso al 31/08 escluso, cioè sette giorni prima e dopo |
| "la settimana scorsa" | Da lunedì a domenica della settimana ISO precedente |
| "prima della migrazione" | Solo se l'evento è identificato nel vault, altrimenti chiede |

Test: fine mese, anno omesso, cambio d'anno, anno bisestile, limiti inclusivi ed esclusivi, cambio dell'ora legale (come quello del 25/10/2026).

### C5 — Un dato cancellato non riappare

- Il registro di cancellazione è una catena di voci con hash e un numero di generazione monotono.
- L'ultima generazione e il suo hash sono custoditi anche in un riferimento affidabile indipendente dai backup (portachiavi del sistema e copia offline insieme alla chiave di recupero).
- Al restore il registro si confronta con il riferimento: se è assente, corrotto, non verificabile oppure valido ma più vecchio (generazione inferiore), i dati potenzialmente cancellati non vengono esposti.
- Test: registro assente, corrotto, valido ma obsoleto, e restore di un backup precedente a una cancellazione.

**Protocollo di aggiornamento**: registro e riferimento non si aggiornano in modo atomico, quindi:

1. si scrive e si sincronizza la nuova generazione del registro, marcata come in sospeso;
2. si aggiorna e si sincronizza il riferimento affidabile;
3. si marca la generazione come completata; solo ora la cancellazione risulta completata.

Riconciliazione: se il registro è avanti di una generazione rispetto al riferimento, con catena valida e marcatore in sospeso, si completa l'aggiornamento del riferimento (roll-forward); qualsiasi altro disallineamento non spiegabile porta il core in modalità di riconciliazione, senza esporre dati potenzialmente cancellati finché non si ristabilisce un riferimento verificabile. Il portachiavi del sistema e la copia offline sono indipendenti dai backup, ma non immuni a rollback o perdita: si confrontano tra loro e prevale la generazione più alta con catena valida. Se si perde il riferimento, il restore resta bloccato finché non ricostruisci il registro riapplicando manualmente le cancellazioni note, con una procedura documentata nel runbook. Questo non recupera la chiave principale: disponibilità dei dati e protezione dalla loro ricomparsa sono problemi distinti. La chiave si recupera solo dal materiale di recupero protetto; se si perdono sia la chiave principale sia il materiale di recupero, i dati cifrati sono irrecuperabili.

**Casi aggiuntivi della riconciliazione**:

- registro `pending` e riferimento con la stessa generazione e lo stesso hash (crash tra il passo 2 e il passo 3): il core verifica integrità e applicazione della cancellazione e completa la marcatura in modo idempotente;
- il dato diventa non consultabile dal passo 1: durante tutta la finestra `pending` le letture non lo restituiscono;
- la copia offline è un checkpoint periodico con data e generazione note, non un riferimento aggiornato a ogni cancellazione: può essere più vecchia del riferimento primario senza che questo provi un rollback, purché sia integra e coerente con la catena; se il riferimento primario risulta più vecchio del registro oltre il caso `pending`, o se la differenza non è riconciliabile, il restore resta bloccato.

**Cancellazione in sospeso che non si completa**: se una cancellazione resta `pending` perché il riferimento affidabile non è aggiornabile o verificabile, il dato rimane non consultabile e il core espone uno stato di riconciliazione, senza dichiarare completata la cancellazione. Riavvii e restore non rendono di nuovo visibile il dato finché il protocollo non si chiude in modo verificabile. L'API distingue "non disponibile, cancellazione in corso" da "cancellato".

Test: crash dopo il passo 2, registro e riferimento pari ma `pending`, copia offline più vecchia ma integra, riferimento primario più vecchio del registro, due cancellazioni consecutive con crash tra le due, lettura concorrente durante una cancellazione.

Test: crash prima e dopo ogni passo, rollback di uno solo dei due elementi, perdita della copia offline, restore con un registro più recente del backup.

### C6 — Stati indipendenti di una nota

Il gate registra cinque proprietà separate: validità dello schema, validità dei link, presenza delle prove, stato di verifica (`status`), eleggibilità nel retrieval. Una nota strutturalmente valida non diventa `verified` in automatico, e non viene rifiutata solo perché non ha relazioni.

**Stati dei riferimenti**: `unlinked` (nessun link, ammesso), `resolved` (punta a qualcosa che esiste), `pending` (fonte esterna, nota non ancora importata, simbolo di codice non risolto: ammesso e segnalato), `broken` (link interno verso una nota che dovrebbe esistere e non c'è: bloccante). La policy può variare per tipo di relazione; l'assenza di link non si confonde mai con un link invalido.

## Ordine di implementazione

Si parte dalla Fase A senza caricare alcun modello: il modello entra solo quando vault, recovery e policy sono già testabili senza AI. Prima di ogni passo si chiudono i contratti dell'appendice da cui dipende.

1. **A1 — Schema e storage**: validazione, scrittura concorrente e conflitti.
2. **A2 — Recovery**: journal a stati, crash injection, indice ricostruibile. Si avvia con la regola sui retry di C1 e la matrice stato × guasto già tradotta in test.
3. **A3 — Privacy**: cifratura, registro di cancellazione, backup e restore. Si avvia con la regola sulle cancellazioni in sospeso di C5 e il runbook di restore e rotazione delle chiavi scritto.
4. **B1 — Ricerca deterministica**: trigrammi, BM25, citazioni, filtri e modalità temporali.
5. **B2 — Grafo del codice**: simboli, commit, relazioni, invalidazione.
6. **B3 — Valutazione**: set di sviluppo, calibrazione e test finale separati, soglie fissate.
7. **C1 — Provenienza e stati**: relazioni, gate, invalidazione delle dipendenze, conflitti.
8. **C2 — Memoria nel tempo**: sessioni, riassunti derivati, cancellazione a cascata.
9. **D1 — Sandbox e runner**: confine d'isolamento scelto e validato prima di esporre strumenti di coding.
10. **D2 — Agente minimo**: task piccoli, diff, test controllati, checkpoint, report.
11. **D3 — Benchmark del modello**: task reali con margine hardware.

Niente UI completa prima che API e flussi siano affidabili: CLI e VS Code bastano, purché le approvazioni sensibili siano esplicite e verificabili.

## Evidenza e benchmark

Ogni criterio di accettazione produce un artefatto salvato in `eval/results/`, così una fase si chiude con prove e non con un'impressione.

Per ogni esecuzione si registrano: versione e commit del codice, modello con URL canonico e hash, quantizzazione e metodo, versione del prompt, parametri di runtime, hardware, dataset e sua versione, seed quando applicabile, risultati grezzi, criteri di accettazione e verdetto; per le scansioni di vulnerabilità anche la data, oltre all'esito. Ogni criterio di accettazione cita il test riproducibile che lo verifica e l'artefatto nella cartella eval/results/ che ne conserva l'esito.

I risultati si confrontano sempre con la configurazione precedente; un miglioramento medio non basta se peggiora una classe critica.

## Decisioni da chiudere prima di iniziare

- [ ] **Ambiente della sandbox per la Fase D**: Linux nativo, una VM Linux sul laptop, oppure gVisor in WSL2 solo se supera la suite d'isolamento; da chiudere prima di D1, con il threat model minimo.
- [ ] **Modello principale candidato**: Qwen3.6-35B-A3B in IQ3 in VRAM o in Q4 con esperti in RAM; si decide col benchmark della Fase D, e nelle fasi A–C non serve.
- [ ] **Repository di prova**: quali tuoi repository usare per il set di riferimento e per i task dell'agente.
- [ ] **Disco di backup**: quale disco esterno usare e dove conservare la chiave di recupero offline.
- [ ] **Web UI minima**: se nell'MVP basta la CLI più VS Code, o serve già una vista del grafo.
