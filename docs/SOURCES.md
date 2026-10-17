# Satchel Sources — Design der generischen Quellen-Abstraktion

Status: lokale v1 implementiert (2026-07-16)
Baut auf: `docs/PLAN.md` (Architektur), Schema `src-tauri/src/db/migrations/`

## 1. Leitidee

Satchel wird von „E-Mail-Client" zu „Nachrichten-Zentrale": Neben E-Mail-Konten
gibt es **Quellen** (Webhook, RSS, Script, später mehr), deren Nachrichten in
derselben Inbox, demselben Board und demselben Workflow landen. Der zentrale
Trick: **Eine Quelle ist ein Account.** Es gibt keine zweite Parallelwelt —
Sidebar, Unified Inbox, Farben, Filter, Regeln, Snooze, Benachrichtigungen und
Board funktionieren für Quellen exakt so wie für Konten, weil es dieselben
Codepfade sind.

Was sich unterscheidet, wird über genau zwei Mechanismen ausgedrückt:

1. **`kind`** — ein Diskriminator auf Account-Ebene (`email | webhook | rss | script`).
2. **Capabilities** — eine zentrale Matrix, die pro `kind` sagt, welche Aktionen
   existieren (antworten? remote löschen? Ordner?). UI und Backend fragen die
   Matrix, niemals `if kind === '...'` verstreut im Code.

## 2. Ist-Analyse: Was schon generisch ist (und was nicht)

Die Codebasis ist der Vision näher, als das Datenmodell vermuten lässt:

**Schon (fast) generisch:**
- `accounts.provider` ist in der DB ein freier `TEXT`; die Dispatch-Stelle ist
  eine einzige Funktion (`providers/mod.rs::provider_for_account`).
- Die Thread-Identität ist bereits ein generischer Key:
  `th-{provider}-{account}-{thread_key}` in `sync/message_ingest.rs::upsert_provider_message`
  (`remote_thread_id` mit Fallback auf `remote_id`). Webhook-`thread_key`,
  RSS-GUID und Script-Jobname passen ohne neue Engine hinein.
- `threads` ist e-mail-frei: workflow_state, board, snooze, priority, category,
  tags — alles quellenneutral.
- `messages_fts` indiziert subject/body_text/from — funktioniert für jede Quelle.
- Der Event-Bus (NewMail → Notification/Webhook-Delivery) hängt am Message-Upsert,
  nicht an IMAP.

**E-Mail-spezifisch (die eigentliche Arbeit):**
- `messages` hat NOT-NULL-Zwänge (`from_address`, `to_addresses`, `folder_id`)
  und keine Ablage für quellenspezifische Daten.
- `Account.provider` ist im Frontend ein geschlossener Union-Typ
  (`'imap' | 'gmail' | 'outlook'`, `src/types/mail.ts`).
- Sync-Manager wählt Accounts per Hardcode-Liste
  (`WHERE provider IN ('imap','gmail','outlook')`).
- Remote-Mutationen (archive/move/delete/flags) und Outbox gehen davon aus, dass
  jeder Account einen `MailProvider` hat.
- `MessageBubble`/`EmailRenderer` rendern ausschließlich Mail.
- Kategorisierung (`classify_message`) läuft header-basiert beim Ingest.

## 3. Entscheidungen

### E1: Quelle = Account mit `kind`-Spalte
`accounts` bekommt `kind TEXT NOT NULL DEFAULT 'email'`. `provider` bleibt der
**konkrete Konnektor** (imap/gmail/outlook/webhook/rss/script), `kind` die
**Familie**, an der Capabilities und Rendering hängen. Die Zuordnung
provider→kind lebt in genau einer Registry-Funktion (Rust), das Frontend erhält
`kind` im Account-DTO.

Warum eine Spalte statt Ableitung im Code: SQL-Filter (`WHERE kind='email'`)
ersetzen die Hardcode-Provider-Listen und bleiben korrekt, wenn Konnektoren
dazukommen.

`accounts.email` (NOT NULL) wird für Quellen als **Identität** weiterverwendet:
Feed-URL bei RSS, Slug bei Webhook/Script (z. B. `job:backup`). Kein Schema-Bruch.

### E2: Ein Nachrichtenmodell, nullable statt zweites Modell
Keine zweite Message-Tabelle. E-Mail-only-Felder sind bereits nullable oder
werden mit neutralen Werten gefüllt (`to_addresses = '[]'`). Neu:

- `messages.source_payload TEXT` — JSON-Blob für quellenspezifische Daten
  (Webhook-`data`, RSS-Link+Enclosures, Script-Exit-Code/Job-Metadaten).
  Bewusst **ein** Feld statt vieler Spalten: Payload ist Anzeige-Detail, nie
  Query-Kriterium (Queries laufen über title/body/FTS/level→priority).

Mapping der universellen Felder:

| universell        | messages-Spalte       | Bemerkung |
|-------------------|-----------------------|-----------|
| title             | subject               | |
| author            | from_address/from_name| Webhook: Quelle-Slug; RSS: Autor/Feed; Script: `job:<name>` |
| body (md/plain)   | body_text             | Markdown erlaubt, Renderer entscheidet |
| body_html         | body_html             | RSS-Artikel (sanitized wie Mail) |
| timestamp         | date                  | |
| external_id       | remote_id             | Dedup-Anker (RSS-GUID, Webhook-Idempotency) |
| thread_key        | → Thread-ID-Konvention| bestehende `th-…`-Konvention, unverändert |
| url               | source_payload.url    | „Im Browser öffnen"-Aktion |
| level             | → threads.priority    | error→hoch, warn→normal, info→niedrig |
| payload           | source_payload        | |

### E3: Synthetischer Ordner statt nullable folder_id
`messages.folder_id` bleibt NOT NULL. Jede Nicht-E-Mail-Quelle bekommt bei
Anlage genau einen Ordner (`folder_type='inbox'`). Das hält sämtliche
Folder-Joins, Zähler und die Mailbox-Tree-Logik unverändert; der Tree blendet
Ordner von Quellen mit `capabilities.hasFolders=false` schlicht aus.

### E4: Capability-Matrix als einzige Verzweigungsstelle

Rust: `sources/capabilities.rs` mit `SourceCapabilities` + `capabilities_for(kind)`;
Command `get_source_capabilities` liefert die Map einmalig ans Frontend
(`services/capabilities.ts`, im Accounts-Store gecacht). Komponenten fragen
`caps(account).canReply` statt Provider-Strings.

| Capability            | email | webhook | rss | script |
|-----------------------|:-----:|:-------:|:---:|:------:|
| canReply / canForward / canCompose | ✓ | – | – | – |
| hasFolders (Tree, Move-UI)         | ✓ | – | – | – |
| remoteFlags (read/star syncen)     | ✓ | – | – | – |
| remoteArchive / remoteDelete       | ✓ | – | – | – |
| hasAttachments                     | ✓ | – | später (Enclosures) | – |
| hasRecipients (to/cc anzeigen)     | ✓ | – | – | – |
| canUnsubscribe                     | ✓ | – | – | – |
| defaultNotify                      | ✓ | ✓ | **–** (opt-in) | level≥warn |
| Workflow/Board/Snooze/Tags/Regeln/Suche | ✓ | ✓ | ✓ | ✓ |

Lesart der letzten Zeile: Alles Lokale gilt **immer** und taucht in der Matrix
gar nicht erst auf — nur Remote-/Compose-Fähigkeiten sind capability-gesteuert.
Archivieren/Löschen/Gelesen existieren für jede Quelle, sind bei
`remote* = false` einfach rein lokale DB-Operationen.

### E5: Zwei Ingest-Wege, ein Upsert

**Kern:** Der Message-Upsert liegt in `sync/message_ingest.rs::upsert_provider_message`
und wird von beiden Welten benutzt. Das ist das
wichtigste Refactoring des Vorhabens — E-Mail-Sync und Quellen teilen sich
danach denselben getesteten Pfad (Dedup, Thread-Upsert, FTS, Event-Bus,
Notification-Entscheid).

```rust
pub struct IngestItem {
    pub external_id: String,          // Dedup (→ remote_id)
    pub thread_key: Option<String>,   // None ⇒ eigener Thread pro Item
    pub title: String,
    pub author: Option<String>,
    pub body_text: Option<String>,    // Markdown/Plain
    pub body_html: Option<String>,    // wird wie Mail sanitized
    pub timestamp: Option<String>,    // RFC3339, Default: jetzt
    pub level: Option<Level>,         // info|warn|error
    pub url: Option<String>,
    pub payload: Option<serde_json::Value>,
}
```

- **Push** (webhook, script): `ingest_source_inner` ist der Einstieg, idempotent
  über `external_id`. Der HTTP-Transport dafür wurde mit ADR 0005 entfernt und
  kommt mit der Hub-API (ADR 0008) neu; bis dahin gibt es keinen Push-Weg.
- **Pull** (rss, später mehr): schlanker Trait, bewusst **nicht** `MailProvider`
  (der schleppt send/flags/drafts/idle mit):

```rust
#[async_trait]
pub trait PullSource: Send + Sync {
    async fn poll(&self, checkpoint: SyncCheckpoint)
        -> Result<(Vec<IngestItem>, SyncCheckpoint), String>;
    fn default_interval(&self) -> Duration;   // RSS: 30 min
}
```

  Ein Poll-Worker nach dem Muster der bestehenden Worker (`sync/maintenance.rs`)
  iteriert `WHERE kind != 'email' AND is_active=1`, Checkpoint in
  `accounts.sync_state` (wie beim Mail-Sync).

### E6: Rendering pro kind, ein Einstiegspunkt
`MessageBubble.vue` delegiert anhand `kind` an genau einer Stelle:
- `email` → bestehender `EmailRenderer` (unverändert).
- `rss`/`webhook`/`script` → neuer `ItemRenderer`: Titel, Meta-Zeile (Quelle, Level,
  Zeit), Body als Markdown/Plain, `payload` als aufklappbare Key-Value/JSON-Ansicht.

Threadlisten-Items brauchen fast nichts: Level-Badge statt Attachment-Icon,
Quelle-Farbe kommt vom Account wie bisher.

### E7: Schutz der E-Mail-Pfade (Guards)
- Sync-Manager: `WHERE provider IN (…)` → `WHERE kind = 'email'`.
- IDLE-/Mail-Sync starten weiterhin nur für die bekannten Mail-Provider.
- Remote-Mutationen/Outbox (`sync/remote_mutations.rs`, `outbox.rs`): Guard am
  Eingang — Aktionen für Accounts ohne entsprechende Remote-Capability werden
  **lokal angewendet und als erledigt markiert** (kein Fehler, kein Queue-Müll).
- `classify_message` (Kategorien) läuft nur für `kind='email'`; Quellen setzen
  ihre Kategorie über Quelle-Defaults/Regeln.
- Vacation, Signaturen, Aliases, S/MIME, PGP, Unsubscribe: unverändert, UI per
  Capability ausgeblendet.

## 4. Schema-Diff (Migration `0034_sources_and_api_tokens.sql`; die Token-Tabellen daraus sind seit 0058 wieder weg)

```sql
ALTER TABLE accounts ADD COLUMN kind TEXT NOT NULL DEFAULT 'email';
ALTER TABLE messages ADD COLUMN source_payload TEXT;
CREATE INDEX IF NOT EXISTS idx_accounts_kind ON accounts(kind, is_active);
```

Das ist absichtlich alles. Bestehende Zeilen sind per DEFAULT korrekt
(`email`), kein Backfill, kein Umbau bestehender Spalten, Rollback trivial.
Frontend-DTO (`services/backend.ts`) und `Account`-Typ bekommen `kind`;
der Union-Typ `provider` wird zu `string` mit Registry-Lookup.

## 5. Umsetzungsstand

Die vier Phasen wurden als lokale v1 umgesetzt; E-Mail-Verhalten blieb regressionsgetestet:

- **Phase 0 — Fundament (erledigt):** Migration 0034, `kind` durch
  Account-DTOs ziehen, Registry provider→kind, Guards aus E7. Risiko
  konzentriert sich hier; danach Smoke-Test E-Mail (P2-007 hilft).
- **Phase 1 — Capability-Gating (erledigt):** Matrix in Rust + `get_source_capabilities`,
  quellenabhängige UI-Stellen absichern
  (Composer-Aktionen, Move-UI, Unsubscribe-Banner, VacationSettings).
  Für reine E-Mail-Nutzung ein No-Op — genau so muss es sich verhalten.
- **Phase 2 — Ingest-Kern (erledigt):** `IngestItem`, synthetischer Ordner
  bei Quellen-Anlage, `ItemRenderer`.
- **Phase 3 — Erste Quellen (erledigt):** RSS als erster `PullSource` (P3-012).
  Webhook-UI und CLI wurden mit ADR 0005 wieder entfernt; die Settings-UI legt
  nur noch RSS-Quellen an.

Phase 0+1 sind die Fable-5-würdigen Teile (Querschnitt, Regressionsrisiko);
Phase 2 ist gut abgrenzbar; Phase 3 ist Muster-Implementierung für günstigere
Modelle mit den Issues P3-010/011/012 als Spezifikation.

## 6. Nicht-Ziele

- **Kein Chat/Streaming** — Quellen sind nachrichtenbasiert, keine Live-Kanäle.
- **Keine Zwei-Wege-Pflicht** — Quellen dürfen read-only sein; „Antworten auf
  einen Webhook" ist explizit kein Ziel (falls je gewünscht: eigenes Design).
- **Kein Plugin-Ingest in v1** — die Wasmtime-Runtime bekommt erst Zugriff auf
  den Ingest, wenn der Kern steht.
- **E-Mail bleibt First-Class** — jede Abwägung fällt zugunsten der Mail-Pfade.

## 7. Offene Punkte (nicht blockierend)

- **Retention:** Script-/Webhook-Quellen können schnell wachsen. Vorschlag:
  per-Quelle „Done älter als N Tage löschen", angehängt an den bestehenden
  Maintenance-Worker. Entscheidung bei Phase 3.
