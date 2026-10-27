# Debito tecnico

Registro dei rischi noti e accettati, non ancora risolti, tracciati con un identificativo stabile per poterli ritrovare tra fasi e sessioni.

## SEC-001 — Protezione filesystem contro symlink race e TOCTOU

- **Stato:** DEFERRED — rischio noto, non risolto.
- **Fase di origine:** A1 — Schema e storage.
- **Scadenza architetturale:** rivalutazione obbligatoria durante A2, prima di dichiarare completato lo storage affidabile. Verifica ulteriore prima della Fase D, quando sarà introdotta l'esecuzione di codice non fidato.
- **Criterio di chiusura:** il debito può essere marcato RESOLVED soltanto dopo implementazione, test avversari superati e revisione indipendente. Se la protezione completa viene rinviata ulteriormente, occorre una nuova valutazione esplicita del rischio e del modello di minaccia — non un rinvio silenzioso.

### Contesto

In A1 sono stati introdotti controlli anti-symlink mediante `symlink_metadata` sui percorsi effettivi `.record` (`crates/storage/src/vault.rs`, `record_path_checked`). Questi controlli impediscono alcuni accessi tramite symlink già presenti al momento del controllo, ma non eliminano le vulnerabilità Time-of-Check to Time-of-Use (TOCTOU): il filesystem può cambiare tra il momento della verifica e il momento dell'uso effettivo (lettura, scrittura, rename). Questo limite è stato esplicitamente accettato per A1 perché il modello di minaccia della fase è dichiarato non-avversario e a processo singolo — non perché il rischio sia stato eliminato.

### Intervento richiesto (A2)

- Valutare aperture sicure basate su handle (es. `openat`-style, directory-fd-relative) e verifiche dei componenti del percorso, non solo canonicalizzazione seguita da un uso separato.
- Implementare protezioni specifiche per Windows, considerando anche Linux (i due sistemi hanno primitive e garanzie diverse su symlink e handle).
- Impedire che sostituzioni concorrenti dei file consentano letture o scritture fuori dalla vault.
- Introdurre test avversari con modifiche concorrenti ai percorsi e ai symlink (non solo simlink statici piantati prima del test, come nei test attuali di A1).
- Verificare che recovery, backup e restore rispettino gli stessi confini di sicurezza — questi componenti arrivano in A2/A3 e devono essere progettati con lo stesso vincolo fin dall'inizio, non corretti successivamente.

### Riferimenti incrociati

- Continuity: decisione [d5b9] (questo debito), vincolo [c12ah] (gap TOCTOU originale, Task 6), vincolo [c15ii] (fix I-C: ha ristretto cosa viene controllato, non ha chiuso la finestra TOCTOU).
- Codice: `crates/storage/src/vault.rs`, funzione `record_path_checked`.
- Piano A1: `docs/superpowers/plans/2026-10-26-a1-schema-storage.md` — limitazioni accettate.
- Evidenza A1: `eval/results/A1/evidence.md`.

### Non fare

- Non implementare questo intervento come parte di A1.
- Non ampliare lo scope di A1 per "risolvere subito" questo debito.
- Non modificare la specifica MVP v6.
- Non segnare questo problema come RESOLVED finché non sono soddisfatti tutti i criteri di chiusura sopra elencati.

## SEC-002 — Aliasing interno alla vault tramite symlink tra record

- **Stato:** DEFERRED — rischio noto, non risolto.
- **Fase di origine:** A1 — Schema e storage (individuato durante la revisione mirata della correzione di SEC-001/I-C).
- **Scadenza architetturale:** valutare la chiusura insieme a SEC-001 in A2 — lo stesso meccanismo (rifiuto di qualunque symlink su un percorso `.record`, dato che la vault non ne crea mai) chiude entrambi senza costo aggiuntivo.
- **Criterio di chiusura:** il debito può essere marcato RESOLVED soltanto dopo implementazione, test avversari superati e revisione indipendente.

### Contesto

Il controllo introdotto per I-C (`ensure_resolves_inside_root` in `crates/storage/src/vault.rs`) verifica che il percorso `.record` effettivo si risolva **dentro** la radice della vault. Non verifica però se quel percorso sia esso stesso un symlink che punta al file `.record` di un'**altra nota, sempre dentro la vault** — un caso che supera il controllo "risolve dentro la radice" senza costituire una fuga.

Riprodotto empiricamente (2026-10-27): un symlink a `root/note-a.record` che punta a `root/note-b.record` fa sì che `read_note(note-a)` restituisca il contenuto e la versione di `note-b`; `propose_and_commit(note-a, expected = versione di note-b)` ha successo e sostituisce il symlink con un file reale (note-b resta intatta). Non c'è fuga dalla vault né corruzione per scrittura cross-note, ma c'è un'attribuzione errata di contenuto e versione a un id di nota sbagliato — un problema di integrità reale, non solo teorico.

### Intervento richiesto (A2, insieme a SEC-001)

- Rifiutare qualunque symlink trovato al percorso `.record` candidato, indipendentemente da dove punti (dentro o fuori dalla vault) — la vault non crea mai un symlink a quel percorso, quindi la sua presenza è sempre un segnale di manomissione o errore esterno.
- Verificare che questo non interferisca con test legittimi che oggi assumono l'aliasing interno come "non una fuga" (`record_symlink_resolving_inside_the_vault_root_is_not_an_escape` o nome equivalente in `vault.rs`) — quel test andrà invertito o rimosso quando si chiude questo debito.
- Includere nei test avversari di SEC-001 anche lo scenario di aliasing cross-note, non solo l'escape verso l'esterno.

### Riferimenti incrociati

- Continuity: vincolo [c16c6] (questo debito).
- Codice: `crates/storage/src/vault.rs`, funzione `ensure_resolves_inside_root`.
- Evidenza A1: `eval/results/A1/evidence.md`, sezione 9.1 (prima menzione, non ancora con un identificativo stabile prima di questa voce).
- Relazionato a SEC-001 (stesso meccanismo di chiusura: rifiuto categorico dei symlink su percorsi `.record`).

### Non fare

- Non implementare questo intervento come parte di A1.
- Non ampliare lo scope di A1 per "risolvere subito" questo debito.
- Non segnare questo problema come RESOLVED finché non sono soddisfatti tutti i criteri di chiusura sopra elencati.
