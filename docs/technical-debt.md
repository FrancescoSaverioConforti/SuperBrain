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
