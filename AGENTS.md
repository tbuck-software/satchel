# Satchel — Arbeitsregeln

Tastaturgesteuerter, local-first Mail-Client. Vue 3 + TypeScript im Frontend,
Rust + Tauri 2 im Backend, SQLite mit FTS5 als Speicher.

## Vor jedem Commit

```bash
make check     # vitest + cargo test + playwright smoke
```

CI (`.github/workflows/ci.yml`) erzwingt zusätzlich:

```bash
npx vue-tsc -b
cargo fmt --all --check
cargo clippy -p satchel -p satchel-core --all-targets -- -D warnings
```

`clippy` läuft über **beide** Rust-Crates (`--workspace` geht nicht: die
Plugin-Crates sind `wasm32-wasip2`-Komponenten und linken für den Host nicht).
Zwei Lints sind bewusst crate-weit erlaubt (`type_complexity`,
`too_many_arguments`, begründet in `src-tauri/src/lib.rs` und
`crates/satchel-core/src/lib.rs`) — alles andere ist ein Fehler, kein Hinweis.

Die Toolchain ist in `rust-toolchain.toml` gepinnt und gilt für CI wie für
lokale Rechner. Vorher löste CI `stable` zur Laufzeit auf: `clippy -D warnings`
war lokal grün und in CI rot, wegen Lints, die es in der älteren Version noch
nicht gab. Eine Anhebung ist ein eigener Commit — Kanal hochsetzen, `make check`,
und aufräumen, was das neuere `clippy` gelernt hat.

Die Workspace-Profile in `Cargo.toml` geben `sqlx-macros` Line-Tables: Ohne
sie erzeugt rustc 1.97.1 auf macOS 27 ein von dyld abgelehntes Dylib.
Lokale Checks und CI verwenden dieselben Profile, ohne zusätzliche Cargo-Config.

## Commits

- **Niemals** KI-Assistenten erwähnen: kein `Co-Authored-By`, keine Signatur,
  keine Referenz im Text.
- Conventional Commits (`feat(mail):`, `fix(auth):`, …).

## Sprache

- **UI-Strings nur über `t()`; Klartext im Template ist ein Bug**
  (`pnpm i18n:check`). Das gilt auch für Platzhalter, `title`, `aria-label`,
  Fehler- und Toast-Meldungen.
- **Englisch ist Default, Deutsch zweite Sprache; beide Dateien werden im
  selben Commit gepflegt.**
- Code, Kommentare und Doc-Comments auf Englisch.
- Kataloge werden generiert: bearbeitet wird `src/i18n/parts/*.json`
  (`{ "en": …, "de": … }`), `pnpm i18n:build` schreibt daraus `en.json`,
  `de.json` und die Subsets für Viewer und Tray. `en.json`/`de.json` nie von
  Hand anfassen.
- Rust liefert **Message-Keys plus Parameter**, nie fertige Sätze. Was das
  Betriebssystem zeichnet (Tray, Dock, Notification-Aktionen, OAuth-Rückseite),
  bekommt seine Strings über `set_ui_strings` vom Frontend.
- Der Viewer und der Tray haben eigene, generierte Kataloge. Wer dort einen
  **dynamischen** Key baut (`t(\`x.${id}\`)`), fällt aus der Analyse und
  bekommt zur Laufzeit den Key zu sehen — statische Literale verwenden oder in
  `SUBSETS` in `scripts/i18n-merge.mjs` eintragen.

## Frontend

- Neue UI **immer** aus `src/components/ui/` zusammensetzen (`AppButton`,
  `IconButton`, `NavItem`, `PaneHeader`, `SegmentedControl`, `EmptyState`,
  `ToastStack`, …). Keine handgerollten Buttons oder Zeilen.
- Farben, Radien und Abstände nur über die Tokens aus `src/styles/tokens.css`
  (siehe `STYLEGUIDE.md`). **Keine Hex-Werte in Komponenten** — sie brechen im
  Light-Theme.
- Backend-Zugriff ausschließlich über `src/services/backend.ts`. Kein `invoke`
  in Komponenten.
- **Kein Zweig, der im Mock etwas anderes tut als im Betrieb.** Mock-Daten
  kommen ausschließlich aus `src/services/backend/mock.ts`, das
  `vite.config.ts` bei `VITE_SATCHEL_MOCK=1` anstelle von
  `src/services/backend/transport.ts` einhängt. Stores fragen
  `backendCommandsAvailable()` („kann ich Commands schicken?"), nicht
  `backendSupportsNativeCommands()` („laufe ich in einer Tauri-Webview?") —
  letzteres bleibt Fenstern, Events, Tray und Dateisystem vorbehalten.
  `src/architecture/mockBoundary.test.ts` erzwingt beides.
- Thread-Aktionen (Archivieren, Löschen, Sternen, Zustandswechsel) laufen über
  `useThreadActions()` — dort hängen optimistisches Update und Undo-Toast.
  Nicht am Store vorbei direkt das Backend rufen.

Die drei Regeln oben prüft `src/architecture/designSystem.test.ts` maschinell.
Der heutige Bestand steht dort namentlich in den `KNOWN_*`-Listen, mit Zahl je
Datei und einem Budget. Ein neuer Verstoß kostet **zwei** bewusste Handgriffe:
Eintrag anlegen und Budget hochsetzen. Eine Zahl, die zu hoch geworden ist, ist
ebenfalls rot — die Liste soll schrumpfen. Abstände sind bewusst **nicht**
geprüft; was der Check abdeckt, steht in `STYLEGUIDE.md` unter „What is
enforced".

### Virtualisierte Thread-Liste

`ThreadGroupList.vue` misst Zeilenhöhen über **Konstanten**
(`THREAD_HEIGHT`, `DATE_HEADER_HEIGHT`, `BUCKET_HEIGHT`, …). Die Konstanten
liegen als Custom Properties (`rowHeights`) am Scroller, die Wrapper benutzen
`h-[var(--thread-row-h)]` — eine Zeilenhöhe wird also nur an der Konstante
geändert. Eine Pixelhöhe im Template bringt die zweite Kopie zurück und lässt
das Scrolling driften; `ThreadGroupList.heights.test.ts` lehnt sie ab.
Container-Queries dürfen nur das Innere der Zeile ändern, nie die Wrapper-Höhe.

Kein `deep: true` auf `props.items` — der Store erzeugt bei jeder Änderung ein
neues Array.

### Kommandos und ACL

Jedes `#[tauri::command]` gehört an **zwei** Stellen: in `generate_handler!`
(`src-tauri/src/lib.rs`) und in `COMMANDS` (`src-tauri/build.rs`). Aus der
zweiten Liste erzeugt Tauri das App-ACL-Manifest und je eine
`allow-<kommando>`-Berechtigung; fehlt der Eintrag, gibt es keine Berechtigung
zu erteilen und das Kommando ist aus **keinem** Fenster aufrufbar.
`tests/app_acl.rs` vergleicht die beiden Listen.

Fenster, die das ganze Client-Bundle laden (`main`, `compose-*`, mobil),
bekommen das generierte Set `allow-app-commands`. Die schmalen Fenster
(`message-file-*`, `tray-popover`) zählen ihre Kommandos einzeln auf — das ist
dort die Sicherheitsgrenze, nicht bloß eine Beschreibung.

### Nachrichten-Viewer (`.eml` / `.msg`)

Doppelklick auf eine Mail-Datei öffnet ein eigenes Fenster, nicht den Client.
Drei Teile hängen zusammen und brechen einzeln lautlos:

- `viewer.html` ist ein **zweiter Vite-Entry** (`build.rollupOptions.input`).
  Fällt er aus der Liste, baut der Build ihn nicht mehr und das Fenster zeigt
  404 — erst im gebauten `.app`, nie im Dev-Server.
- Die Fenster heißen `message-file-<hash>`. Die Capability
  `capabilities/message-viewer.json` greift über genau dieses Präfix; ein
  anderer Name lässt das Fenster ohne Rechte laufen (kein Speichern-Dialog).
  Sie listet die sieben Kommandos auf, die der Viewer braucht — alles andere
  ist aus diesem Fenster nicht erreichbar.
- Der Dateipfad reist base64url-kodiert als `?p=` — Mail-Dateien heißen mit
  Leerzeichen, Umlauten und `#`.

Der Viewer lädt bewusst keine Stores, keinen Router, keinen Sync. Maßgeblich ist
nicht sein eigenes Entry-Chunk (7 KiB gzip), sondern was das Fenster vor der
ersten Zeile Mail parsen muss: 118 KiB gzip gegenüber 238 KiB der
Hauptanwendung. `scripts/check-entry-bundles.mjs` erzwingt beide Grenzen — wer
Store-Zugriffe oder einen Barrel-Import (`@/services/backend` statt des
Submoduls) einbaut, zieht den Bootstrap wieder hinein und die Prüfung wird rot.

`open_message_file` schreibt **nie** in die Datenbank — das ist der Unterschied
zu `import_mbox`, das ein Konto braucht.

## Backend

Zwei Crates: `crates/satchel-core` (`satchel_core`) hält Domäne, DB, Sync,
Provider, MIME und Krypto — **ohne** Tauri; `src-tauri` hält Commands, Fenster,
Plattform-Plugins und die AppHandle-gebundenen Sync-Pfade (`mailbox_sync`, `idle`,
`maintenance`, `outbox`). Core kennt die App nicht: was Core braucht, wandert
nach Core, statt zurückzugreifen. `lib.rs` re-exportiert Core, damit
`crate::db::…` weiter aufgeht. Migrationen und `sqlx::migrate!` liegen in Core;
im App-Crate lautet der Pfad `"../crates/satchel-core/src/db/migrations"`.

- Migrationen sind **additiv**. Nummern werden nie neu vergeben; entfernte
  Migrationen werden über `set_ignore_missing(true)` abgefangen
  (`crates/satchel-core/src/db/mod.rs`), verlorene Indizes über eine Nachzügler-Migration
  wiederhergestellt (Beispiel: `0030`).
- Remote-Mutationen **vor** lokalen Moves einreihen. IMAP-UIDs gelten pro
  Ordner; wer erst lokal verschiebt, löst die falschen Nachrichten auf.
- Jeder neue `#[tauri::command]` braucht einen Aufrufer in
  `src/services/backend.ts` — `backend.ipc-coverage.test.ts` schlägt sonst fehl.
  Bewusste Ausnahmen kommen mit Begründung in dessen `KNOWN_UNWIRED`.
- Eine Migration **ändert Schema** oder **setzt eine Markierung** — sie schreibt
  keine Daten über ganze Tabellen um. sqlx fährt jede Migration in einer
  Transaktion vor dem ersten Frame; ein Rewrite blockiert dort den Start und
  beginnt nach einem Abbruch von vorn. Stattdessen:
  `INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES
  ('data_repair.<name>', '', …)` in der Migration, und ein `DataRepair` mit
  demselben Namen in `crates/satchel-core/src/db/data_repairs.rs`, der in Blöcken
  arbeitet und seinen Cursor mit jedem Block speichert. Die sechs bereits
  ausgelieferten Daten-Migrationen bleiben, wie sie sind.

### Mobile (Android/iOS)

Ein Bundle, drei Plattformen. Was der Shell kann, wird **erfragt**, nicht geraten:
`runtime_capabilities` liefert es, `BackendRuntimeCapabilities` ist die Form.

Plattformweise getrennt sind: TLS-Vertrauensanker (Desktop `webpki-roots`,
Mobile die OS-Trust-Stores über `rustls-platform-verifier`), der Keyring
(Desktop `keyring`, Mobile `keyring-core` plus Android-Keystore bzw.
iOS-Keychain) und die Plugins (Tray, Autostart, Single-Instance und die lokale
HTTP-API bleiben `#[cfg(desktop)]`).

`utils::crypto::load_or_create_master_key` fällt auf Mobile **nicht** auf eine
Schlüsseldatei zurück. Auf dem Desktop ist ein unerreichbarer Schlüsselbund
ärgerlich, auf dem Telefon wäre app-privater Speicher der falsche Ort für den
Schlüssel, der alle Mail-Passwörter schützt.

Der TLS-Aufbau für IMAP **und** POP3 steht einmal in
`providers::imap::mail_tls_client_config`. Der Crypto-Provider wird dort
ausdrücklich benannt (`rustls::crypto::ring`), nicht dem prozessweiten Default
überlassen — sonst entscheidet die Abhängigkeit, die sich zuerst registriert,
über unsere Cipher-Suites, und der Handshake bricht, sobald eine unbeteiligte
Crate dazukommt.

OAuth-PKCE-Verifier liegen verschlüsselt in `oauth_flow_sessions` und werden mit
`DELETE … RETURNING` verbraucht. Auf Mobile passiert die Autorisierung in einer
fremden App und der Prozess kann dazwischen sterben; ein Verifier nur im
Speicher nähme den Flow mit.

Der Ausgang des Flows liegt daneben in `oauth_flow_outcomes`, geschrieben in
derselben Transaktion wie das `DELETE … RETURNING`. Ohne diese Zeile hinterlässt
ein Absturz mitten im Code-Tausch keine Spur, und `poll_oauth_flow` — das nach
einem Kaltstart nur noch die Datenbank hat — könnte einen abgeschlossenen Flow
nicht von einem unbekannten unterscheiden.

### Mail-Benachrichtigungen

`tauri-plugin-notification` verwirft auf dem Desktop die `NotificationResponse` —
über das Plugin ist ein Klick **grundsätzlich** nicht beobachtbar. Mail-Meldungen
laufen deshalb auf macOS über `UNUserNotificationCenter`
(`src-tauri/src/utils/mac_notify.rs`): die Thread-ID reist im `userInfo` mit, der
Delegate bekommt sie beim Klick zurück und emittiert `mail:focus-thread`.

Das Framework verlangt ein echtes `.app`-Bundle. `cargo tauri dev` startet die
nackte Binary, dort greift der Fallback aufs Plugin — **Klick und Schnellaktionen
sind im Dev-Server also wirkungslos und nur in der gebauten App prüfbar.**
`is_bundled()` prüft den Bundle-*Pfad*, nicht den Identifier: `mac-notification-sys`
swizzelt `-[NSBundle bundleIdentifier]` und fälscht ihn für unbundled Binaries.

Die Schnellaktionen (Erledigt/Gelesen/Löschen) hängen an der Kategorie
`satchel.mail`, nicht an der einzelnen Meldung. Zwei davon rufen die Kommandos
`set_threads_read` bzw. `trash_threads` direkt; Erledigt ruft
`archive_threads_inner`, das **kein** IPC-Kommando ist — das Frontend archiviert
über `transition_threads`. Danach emittieren sie
`thread:updated` — ohne das Event mutiert die Aktion den Zustand hinter dem
Rücken der Stores.

## Sicherheit

- **Die CSP im Mail-Renderer ist die tragende Schicht** gegen Tracking-Pixel,
  nicht der Regex in `useImagePolicy`. Sie lebt in `src/utils/emailCsp.ts` und
  wird von `IsolatedEmailDocument.vue` als erstes `<meta>` in den `srcdoc`-Head
  geschrieben. `src/utils/emailCsp.test.ts` erzwingt das — sie ist schon einmal
  bei einem Renderer-Umbau lautlos verschwunden.
- Der Mail-iframe bleibt `sandbox` **ohne** `allow-scripts`. Der Preis dafür:
  **WebKit führt in einem so gesandboxten Frame keine Event-Listener aus** —
  Observer feuern, Events nicht. Klick und Hover auf Mail-Links laufen deshalb
  über transparente Hotspots im Hauptdokument (`IsolatedEmailDocument.vue`),
  nicht über Listener im Mail-Dokument. Wer die Hotspots als Umweg wegräumt,
  macht Links in der gebauten App wieder tot — in Chromium bleibt alles grün,
  weil dort die Listener feuern. Genau so lag der Fehler vom 17.07. bis 30.07.
  unentdeckt.
- Die Rendering-Tests laufen deshalb in **beiden** Engines
  (`tests/playwright.smoke.config.ts`, Projekte `chromium` und `webkit-mail`).
  Tauri liefert auf macOS WKWebView aus; ein grüner Chromium-Lauf beweist für
  den Mail-Renderer nichts.
- `tauri.conf.json` hat eine gesetzte `csp`/`devCsp`. Nicht auf `null` zurück.
  `pnpm run test:csp` prüft die Policy gegen den echten Build — sie hat schon
  einmal die Google-Fonts blockiert, ohne dass es jemandem auffiel.
- **Keine externen Ressourcen im Frontend.** Schriften liegen lokal unter
  `public/fonts` (`pnpm run fonts:vendor`). Ein Mail-Client, der beim Start
  Google kontaktiert, widerspricht dem Local-first-Versprechen und bricht
  offline.
- Mail-HTML wird serverseitig mit `ammonia` sanitisiert
  (`crates/satchel-core/src/email/sanitizer.rs`), bevor es gespeichert wird.

## Layout-Zustand

Sidebar- und Listenbreite liegen in `localStorage` (Sidebar 220–420 px,
Thread-Liste 320–760 px). Grenzen im UI-Store, nicht in Komponenten ändern.

## Datenbank-Betrieb

- WAL wird stündlich per `wal_checkpoint(TRUNCATE)` im Maintenance-Worker
  gekürzt. Ohne das wächst es unbegrenzt, weil die langlebigen Leser im Pool
  den Auto-Checkpoint blockieren — in der Praxis auf 2 GB neben einer 2-GB-DB.
  `VACUUM` kürzt das WAL **nicht**.
- Backups nie als rohe Kopie von `db`+`wal`+`shm`. `VACUUM INTO` bzw.
  `sqlite3 .backup` liefert eine konsistente Einzeldatei — und der Master-Key
  aus dem profilspezifischen OS-Schlüsselbund **muss** mit, sonst sind
  verschlüsselte Zugangsdaten verloren. Das gilt auch ohne Klartextdatei.
  `db::MASTER_KEY_FILES` benennt Backup-Dateien und alte Fallback-Pfade;
  portable Exporte schreiben `pgp-master-key-v1`. Der Schlüssel geht mit einer
  Nutzer-Passphrase umschlossen mit (Argon2id, `wrapped:v1:`); wer ihn blank
  exportieren will, muss pro Sicherung ausdrücklich zustimmen.
- Sicherung und Wiederherstellung sind App-Einstellungen (Backup & Export →
  `commands/backup.rs`: `create_backup`, `list_backups`, `restore_backup`,
  `credentials_status`). Restore ersetzt die laufende DB **nicht** sofort:
  die Datei landet als `satchel.db.restore-pending` neben der DB und wird beim
  nächsten Start vor dem Öffnen des Pools eingesetzt
  (`db::apply_pending_restore`); die verdrängte DB bleibt als
  `satchel.db.replaced-<Zeit>` liegen.

## Architekturentscheidungen

`docs/adr/` hält die getroffenen Entscheidungen (Zielbild, Crate-Split,
i18n, Hub-Modell, Plugin-Modell, …). Wer eine davon kippen will, schreibt
ein neues ADR, statt das alte zu ändern.

## Backlog

Der lebende Backlog liegt **lokal** unter `.private/todo/` (P0…P3) und ist
bewusst nicht eingecheckt — nicht im Repo suchen.
`docs/archive/` enthält historische Pläne — **nicht** als Statusquelle
verwenden.
