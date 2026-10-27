# Debito tecnico

Registro dei rischi noti e accettati, non ancora risolti, tracciati con un identificativo stabile per poterli ritrovare tra fasi e sessioni.

## SEC-001 — Protezione filesystem contro symlink race e TOCTOU

- **Stato:** DEFERRED — rischio noto, non risolto.
- **Fase di origine:** A1 — Schema e storage.
- **Scadenza architetturale:** rivalutazione obbligatoria durante A2, prima di dichiarare completato lo storage affidabile. Verifica ulteriore prima della Fase D, quando sarà introdotta l'esecuzione di codice non fidato.
- **Criterio di chiusura:** il debito può essere marcato RESOLVED soltanto dopo implementazione, test avversari superati e revisione indipendente. Se la protezione completa viene rinviata ulteriormente, occorre una nuova valutazione esplicita del rischio e del modello di minaccia — non un rinvio silenzioso.

### Contesto

In A1 sono stati introdotti controlli anti-symlink in `crates/storage/src/vault.rs` (`record_path_checked` / `ensure_resolves_inside_root`): il percorso `.record` candidato viene canonicalizzato (usando `symlink_metadata` solo per decidere se canonicalizzare il percorso stesso o il suo genitore) e il risultato deve iniziare con la radice canonicalizzata della vault. Questo è un controllo "verifica poi usa" (check-then-act): impedisce alcuni accessi tramite symlink già presenti al momento del controllo, ma non elimina le vulnerabilità Time-of-Check to Time-of-Use (TOCTOU) — il filesystem può cambiare tra il momento della verifica e il momento dell'uso effettivo (lettura, scrittura, rename). Questo limite è stato esplicitamente accettato per A1 perché il modello di minaccia della fase è dichiarato non-avversario e a processo singolo — non perché il rischio sia stato eliminato.

### Intervento richiesto (A2)

- **Non limitarsi ad aggiungere altri controlli sul percorso prima dell'I/O.** Tre tentativi successivi in A1 (il controllo originale del Task 6, poi la correzione I-C, poi la scoperta di SEC-002) hanno ciascuno chiuso un caso specifico lasciandone aperto un altro — il pattern "canonicalizza poi usa" non converge. L'intervento corretto è: aprire il file senza seguire i collegamenti (`O_NOFOLLOW`/`openat` stile directory-fd-relative su Unix, `FILE_FLAG_OPEN_REPARSE_POINT` su Windows), poi verificare l'**handle aperto stesso** — file regolare, contatore di hard link uguale a 1, percorso finale dentro la radice — non il percorso prima di aprirlo. Questo stesso meccanismo chiude anche SEC-002 e SEC-003 (vedi sotto), perché tutti e tre condividono la stessa causa: un controllo sul percorso non può vedere cosa succederà quando il percorso verrà effettivamente apert
o.
- Implementare protezioni specifiche per Windows, considerando anche Linux (i due sistemi hanno primitive e garanzie diverse su symlink, hard link e handle).
- Impedire che sostituzioni concorrenti dei file (symlink o hard link, vedi SEC-003) consentano letture o scritture fuori dalla vault o aliasing tra note diverse dentro la vault.
- Introdurre test avversari con modifiche concorrenti ai percorsi, ai symlink e agli hard link (non solo collegamenti statici piantati prima del test, come nei test attuali di A1).
- Verificare che recovery, backup e restore rispettino gli stessi confini di sicurezza — questi componenti arrivano in A2/A3 e devono essere progettati con lo stesso vincolo fin dall'inizio, non corretti successivamente.

### Riferimenti incrociati

- Continuity: decisione [d5b9] (questo debito), vincolo [c12ah] (gap TOCTOU originale, Task 6), vincolo [c15ii] (fix I-C: ha ristretto cosa viene controllato, non ha chiuso la finestra TOCTOU), vincolo [c17e8] (scoperta degli hard link, SEC-003, che mostra perché i controlli sul solo percorso non bastano).
- Codice: `crates/storage/src/vault.rs`, funzioni `record_path_checked` e `ensure_resolves_inside_root`.
- Piano A1: `docs/superpowers/plans/2026-10-26-a1-schema-storage.md`, sezione "Outstanding technical debt to carry into A2".
- Evidenza A1: `eval/results/A1/evidence.md`, sezione 9.1 ("What this does not do").
- Relazionato a SEC-002 e SEC-003 (stessa causa, stesso meccanismo di chiusura proposto).

### Non fare

- Non implementare questo intervento come parte di A1.
- Non ampliare lo scope di A1 per "risolvere subito" questo debito.
- Non modificare la specifica MVP v6.
- Non segnare questo problema come RESOLVED finché non sono soddisfatti tutti i criteri di chiusura sopra elencati.

## SEC-002 — Aliasing interno alla vault tramite symlink tra record

- **Stato:** DEFERRED — rischio noto, non risolto.
- **Fase di origine:** A1 — Schema e storage (individuato durante la revisione mirata della correzione di SEC-001/I-C).
- **Scadenza architetturale:** valutare la chiusura insieme a SEC-001 e SEC-003 in A2 con un unico meccanismo (verifica dell'handle aperto, non del percorso) — vedi la nota di correzione sotto: la soluzione "rifiuta qualunque symlink" proposta inizialmente per questo debito NON chiude anche SEC-003 (hard link), quindi non è "senza costo aggiuntivo" come scritto in una versione precedente di questa voce.
- **Criterio di chiusura:** il debito può essere marcato RESOLVED soltanto dopo implementazione, test avversari superati e revisione indipendente.

### Contesto

Il controllo introdotto per I-C (`ensure_resolves_inside_root` in `crates/storage/src/vault.rs`) verifica che il percorso `.record` effettivo si risolva **dentro** la radice della vault. Non verifica però se quel percorso sia esso stesso un symlink che punta al file `.record` di un'**altra nota, sempre dentro la vault** — un caso che supera il controllo "risolve dentro la radice" senza costituire una fuga.

Riprodotto empiricamente (2026-10-27): un symlink a `root/note-a.record` che punta a `root/note-b.record` fa sì che `read_note(note-a)` restituisca il contenuto e la versione di `note-b`; `propose_and_commit(note-a, expected = versione di note-b)` ha successo e sostituisce il symlink con un file reale (note-b resta intatta). Non c'è fuga dalla vault né corruzione per scrittura cross-note, ma c'è un'attribuzione errata di contenuto e versione a un id di nota sbagliato — un problema di integrità reale, non solo teorico. Nota aggiuntiva dalla quarta revisione: `note-a` eredita il numero di versione di `note-b` (es. riparte da v4 se `note-b` era a v3), non da zero.

### Correzione importante (quarta revisione, 2026-10-27)

La proposta iniziale di questo debito — "rifiutare qualunque symlink al percorso `.record`, stesso meccanismo di SEC-001, senza costo aggiuntivo" — è stata verificata e si è rivelata incompleta: un **hard link** (non un symlink) allo stesso percorso produce l'identico effetto di aliasing, sia fuori dalla vault (vedi SEC-003) sia tra due note interne, e un controllo che rifiuta solo i symlink non lo rileva, perché la canonicalizzazione non vede gli hard link (la entry è un file regolare autentico). La soluzione corretta, condivisa con SEC-001 e SEC-003, è verificare l'handle aperto (file regolare, contatore di hard link uguale a 1, percorso finale dentro la radice), non aggiungere un ulteriore controllo sul percorso prima dell'apertura.

### Intervento richiesto (A2, insieme a SEC-001 e SEC-003)

- Non limitarsi a rifiutare i symlink al percorso `.record` candidato: quel controllo da solo non rileva gli hard link (SEC-003). Il meccanismo corretto è la verifica dell'handle aperto descritta in SEC-001.
- Verificare che questo non interferisca con test legittimi che oggi assumono l'aliasing interno come "non una fuga" (`record_symlink_resolving_inside_the_vault_root_is_not_an_escape` o nome equivalente in `vault.rs`) — quel test andrà invertito o rimosso quando si chiude questo debito.
- Includere nei test avversari di SEC-001 anche lo scenario di aliasing cross-note, non solo l'escape verso l'esterno, per entrambi i tipi di collegamento (symlink e hard link).

### Riferimenti incrociati

- Continuity: vincolo [c16c6] (questo debito), vincolo [c17e8] (SEC-003, la scoperta che corregge la proposta iniziale di questa voce).
- Codice: `crates/storage/src/vault.rs`, funzione `ensure_resolves_inside_root`.
- Evidenza A1: `eval/results/A1/evidence.md`, sezione 9.1 ("What this does not do").
- Relazionato a SEC-001 e SEC-003 (stessa causa, stesso meccanismo di chiusura proposto: verifica dell'handle, non del percorso).

### Non fare

- Non implementare questo intervento come parte di A1.
- Non ampliare lo scope di A1 per "risolvere subito" questo debito.
- Non segnare questo problema come RESOLVED finché non sono soddisfatti tutti i criteri di chiusura sopra elencati.

## SEC-003 — Aliasing tramite hard link al percorso `.record`

- **Stato:** DEFERRED — rischio noto, non risolto.
- **Fase di origine:** A1 — Schema e storage (individuato durante la quarta revisione completa del branch, 2026-10-27).
- **Scadenza architetturale:** chiudere insieme a SEC-001 e SEC-002 in A2, con lo stesso meccanismo (verifica dell'handle aperto).
- **Criterio di chiusura:** il debito può essere marcato RESOLVED soltanto dopo implementazione, test avversari superati e revisione indipendente.

### Contesto

`ensure_resolves_inside_root` canonicalizza il percorso `.record` candidato per decidere se risolve dentro la radice della vault. La canonicalizzazione non può rilevare un **hard link**: un hard link non è un collegamento simbolico che punta altrove, è una seconda voce di directory che punta direttamente allo stesso inode di un file — per il filesystem è un file regolare autentico, il cui percorso è genuinamente dentro la radice.

Riprodotto empiricamente su Windows (2026-10-27): un hard link a `root/note-a.record` che punta a un file **fuori** dalla vault, sullo stesso volume, fa sì che `read_note(note-a)` restituisca il contenuto del file esterno, e `propose_and_commit` al numero di versione del file esterno abbia successo — lo stesso identico effetto del bug originale di I-C prima della correzione, ma attraverso un hard link invece di un symlink. Il file esterno non viene modificato (il rename sostituisce la voce di directory). Un hard link tra due note interne alla vault produrrebbe lo stesso effetto di aliasing descritto in SEC-002.

### Intervento richiesto (A2, insieme a SEC-001 e SEC-002)

- Non aggiungere un controllo sul numero di hard link al percorso prima dell'apertura (soggetto alla stessa finestra TOCTOU di ogni controllo sul percorso) — verificare invece il contatore di hard link sull'**handle già aperto** (`nlink` su Unix, informazioni equivalenti su Windows), insieme a "file regolare" e "percorso finale dentro la radice", come parte del meccanismo unico descritto in SEC-001.
- Includere test avversari con hard link sia verso l'esterno della vault sia tra due note interne.

### Riferimenti incrociati

- Continuity: vincolo [c17e8] (questo debito).
- Codice: `crates/storage/src/vault.rs`, funzione `ensure_resolves_inside_root`.
- Evidenza A1: `eval/results/A1/evidence.md`, sezione 9.1 ("What this does not do").
- Relazionato a SEC-001 e SEC-002 (stessa causa — un controllo sul percorso non può vedere come l'I/O risolverà quel percorso — stesso meccanismo di chiusura proposto).

### Non fare

- Non implementare questo intervento come parte di A1.
- Non ampliare lo scope di A1 per "risolvere subito" questo debito.
- Non segnare questo problema come RESOLVED finché non sono soddisfatti tutti i criteri di chiusura sopra elencati.
