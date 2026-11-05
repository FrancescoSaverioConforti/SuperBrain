# Riproduzioni locali dell'audit

Baseline produttiva esaminata: `21f0e56`. L'indice storico viene esportato da
`338fbf1`; il fix temporale corrente è `3c6b1bb`. Nessun reset/checkout di file
produttivi o modifica alla specifica viene eseguito dagli script.

Sono fixture **sintetici e isolati**, non test contro dati reali né endpoint
remoti. I file `.stdout.txt` sono output effettivamente ottenuti nell'audit,
normalizzati a UTF-8/LF. Non contengono chiavi o dati personali. Non usare i
risultati per affermare che tutti i percorsi siano stati testati.

## Preflight e requisiti

- Windows con Rust e DPAPI accessibile nello stesso contesto utente del fixture.
  Nell'ambiente Codex la sandbox non consentiva alcune operazioni DPAPI: le
  esecuzioni positive sono state svolte con l'escalation consentita dal task.
- Verificare worktree, processi e lock prima di avviare. Entrambi i runner
  rifiutano un processo cargo/rustc/b1_scalability già attivo. La verifica processi
  è un preflight, non un coordinatore che impedisce ogni avvio successivo esterno.
- Il benchmark B1 resta interrotto; non avviarne un altro per eseguire i probe.
- Dipendenze del workspace già disponibili in cache. Il runner temporale usa
  Cargo **offline**, lock isolato copiato da quello del repository e target cache
  esistente; compila il piccolo helper/indice storico, non una suite di carico.
  Il runner deletion usa rlibs cached: se le feature producono tipi incompatibili
  deve fallire, non contare il build failure come prova di sicurezza.
- Ogni esecuzione conserva un nuovo fixture sotto `.superpowers/security-*`.
  Nessuna cancellazione ricorsiva, bonifica o modifica a vault esistenti. Lo
  staging che attraversa la root è limitato a un fratello **dello stesso fixture
  appena creato**, non a un percorso scelto da input esterno.

## Comandi eseguiti

Dal worktree, una riproduzione alla volta:

```powershell
./docs/security/reproductions/run-temporal-and-store-probe.ps1 -Workspace (Get-Location).Path
./docs/security/reproductions/run-deletion-read-gate.ps1 -Workspace (Get-Location).Path
```

Il primo runner raccoglie stdout/stderr separati sotto
`.superpowers/security-audit-build/` e controlla il codice di uscita. Un build
fallito o un assert fallito interrompe il runner: non riutilizzare un vecchio
eseguibile per dichiarare superata una prova nuova.

## Cosa dimostrano gli output

| Artifact | Prova e limiti |
| --- | --- |
| temporal-and-store-probe.rs/.stdout.txt | Per ognuno dei tre campi: parser/gate ordinari accettano il marker; veri commit Sensitive, file e blob git lo contengono; query con sorgente index storico lo espone, corrente no. Prova anche staging/get raw fuori root del PII store entro il fixture e ciphertext substitution sotto stessa master key, non escalation fra utenti. |
| Stesso probe, parte backup | Un record sintetico senza journal, rifiutato dal secret gate ordinario, viene raccolto dal backup raw e promosso dal restore autenticato; soltanto su nuovo source/destination. L'host possiede legittimamente chiavi e invoca backup/restore: non è una falsificazione AEAD. |
| Stesso probe, parte Vault | Dopo una cancellazione registrata ma con file ancora presenti, Vault rifiuta; dopo spostamento del log nel fixture, Vault e search corrente servono l'id. Il riferimento DPAPI esiste ancora. Non assume che una cancellazione fisica completata lasci sempre ciphertext vivo. |
| Stesso probe, schema | `4294967297` viene troncato a `1` nel campo schema_version. Non dimostra da solo un bypass di una migrazione specifica o esecuzione di codice. |
| deletion-read-gate.rs/.stdout.txt | Reader PII prima rifiuta l'id cancellato; poi serve plaintext con log mancante. Conferma separatamente ref assente e catena alterata rilevabile da verify_chain ma non controllata dal reader. Usa vera DPAPI, non mock del riferimento. |

Un tentativo di traversal tramite `commit_staged` era stato rifiutato dalla
guardia before-image; non viene dichiarato riuscito. I casi positivi riguardano
le funzioni indicate, con codice e prerequisiti nel documento principale.

La suite completa del prodotto e le mutation campaign precedenti non sono state
rieseguite per questi probe. Nuovi controlli mirati dell'audit: domain32,
policy14, search31 e quattro caratterizzazioni SEC-001/002/003, più Cargo Deny.
Il pass delle caratterizzazioni SEC conferma anche vulnerabilità note: non è
un attestato che quei rischi siano chiusi.
