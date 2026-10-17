# E2E-Runbook

Die Abdeckung des Sendewegs liegt auf zwei Ebenen, weil keine einzelne die
ganze Strecke sieht.

## 1. Browser-Suite (`pnpm run test:smoke`)

```bash
pnpm run test:smoke
```

Deckt Inbox, Thread öffnen, Antwort verfassen, **Senden**, Archivieren mit
**Undo**, **Outbox-Eintrag abbrechen**, Suche, Drucken und den Mail-Renderer ab —
letzteren in beiden Engines (`chromium`, `webkit-mail`).

`dev:mock` (`VITE_SATCHEL_MOCK=1`) tauscht **nur den Command-Transport**:
`vite.config.ts` hängt `src/services/backend/mock.ts` anstelle von
`src/services/backend/transport.ts` ein. Alles darüber — Services, Stores,
Composables, Komponenten — ist derselbe Code wie in der App. Es gibt keinen
`mockDataEnabled()`-Zweig mehr, der den Backend-Aufruf überspringt;
`src/architecture/mockBoundary.test.ts` hält das fest.

Was das kauft: `backendSendMessage` so zu ändern, dass es wirft, macht
„sends a reply and shows it in the conversation" rot. Vorher blieb die Suite
grün, weil kein Test gesendet hat und der Mock-Zweig den Aufruf ohnehin umging.

Der Fehlerpfad wird ausdrücklich mitgeprüft: `?backendFailure=<command>` lässt
den Mock-Transport genau diese Commands ablehnen. `core-flows.pw.ts` nutzt das
für den Sendefehler — ein Test, der nur den Erfolgsfall behauptet, kann einen
ausgeführten Aufruf nicht von einem übersprungenen unterscheiden.

Ein Command ohne Handler wirft und schreibt `unimplemented command` auf die
Konsole; ein eigener Smoke-Test lässt den Lauf daran scheitern.

Für eine manuelle Browser-Prüfung:

```bash
pnpm run dev:mock
```

`pnpm run dev` startet ohne Mock-Backend: dort ist gar kein Backend erreichbar,
die Listen bleiben leer. Der Tauri-Entwicklungsbuild zeigt niemals
Frontend-Mocks; ein optionaler Datenbank-Demo-Seed muss separat mit
`SATCHEL_SEED_DEMO_DATA=1` aktiviert werden.

**Was die Browser-Suite nicht kann:** die IPC-Grenze überqueren. Im Browser gibt
es kein Tauri, `backendSupportsNativeCommands()` ist dort immer `false`. Was
Tauri-Events auslösen — der Undo-Fenster-Toast beim Senden (`outbox:undo-window`,
`src/bootstrap.ts`), Tray, Fenster, Benachrichtigungen — ist deshalb nur in der
gebauten App prüfbar. Die Suite deckt den Command ab, den das Undo aufruft
(`cancel_outbox_action`), nicht das Event, das das Fenster öffnet.

## 2. Rust-Integrationstest am Netzwerkrand

```bash
cargo test -p satchel-core --test smtp_submission
```

`crates/satchel-core/tests/smtp_submission.rs` fährt den echten Sendeweg gegen
eine echte SQLite-Datenbank und ersetzt **nur den Netzwerkpartner**: eine
Account-Zeile mit echter Config wird eingefügt, `provider_for_account` wählt den
Provider wie im Betrieb, lettre schreibt auf einen Socket, den der Test
zurückliest. Geprüft werden der zugestellte `From`-Header, der SMTP-Envelope und
dass eine abgelehnte Adresse als Fehler ankommt statt als stille Zustellung.

Das ist die Fehlerklasse aus `21092a3`: `sender_mailbox` auf den alten Stand
zurückdrehen macht `delivers_display_names_lettre_cannot_parse` rot.

## Warum kein `tauri-driver`

Bewusste Entscheidung, nicht Vergesslichkeit:

- `tauri-driver` unterstützt **macOS nicht** (nur Linux/WebKitWebDriver und
  Windows/Edge Driver). Die Hauptentwicklungsplattform und die ausgelieferte
  Engine (WKWebView) fielen also gerade heraus.
- Ein Lauf bräuchte einen vollständigen Linux-Build der App im CI, inklusive
  GTK/WebKit-Abhängigkeiten — teuer, und er prüfte eine Engine, die niemand
  ausliefert.
- Der Rest der Strecke ist anders abgedeckt: `backend.ipc-coverage.test.ts`
  hält Command-Namen auf beiden Seiten zusammen, die Browser-Suite fährt alles
  bis `invoke` inklusive Payload-Aufbau, der Rust-Test alles ab dem Command bis
  auf den Socket. Der Mock prüft außerdem Command-Argumente und verschachtelte
  Feldnamen gegen das aus Rust generierte Manifest. Ein automatisches
  Browser-Fixture lässt jeden Vertragsverstoß fehlschlagen. Die generierten
  DTO- und Input-Typen werden in CI und `pnpm run check` auf Aktualität geprüft
  (siehe [IPC contracts](IPC.md)). Offen bleiben Laufzeitwerte und eigene
  Deserializer an der Serde-Grenze.

Für die verbleibende Laufzeitprüfung ist der nächste Schritt nicht
`tauri-driver`, sondern ein Test über `tauri::test::mock_builder`, der die
`#[tauri::command]`-Funktionen mit echten Payloads aufruft. Das braucht
`tauri = { features = ["test"] }` in den `[dev-dependencies]` von `src-tauri`.

## Provider-Tests

Provider-End-to-End-Tests werden bewusst nicht automatisiert gegen echte Konten
ausgeführt. Die Live-Checkliste steht unter `.private/todo/P0-001-gmail-oauth-e2e.md`.
