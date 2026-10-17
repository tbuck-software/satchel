<!-- HISTORICAL: original architecture/planning spec, superseded by the code.
     Kept for context; see docs/archive/ for more historical material. -->

# Satchel — Projektplan & Architektur-Spezifikation

> **Status: historisch.** Ursprüngliche Architektur- und Produktspezifikation;
> nicht mehr massgeblich für den aktuellen Code.

> Ein moderner, tastaturgesteuerter Email-Client mit Kanban-Workflow, Automations-Pipeline und Plugin-System.
> Inspiriert von Google Inbox, Spark Mail, Shortwave — mit der Philosophie von Vim und Supermaven.

---

## 1. Vision & Kernprinzipien

### 1.1 Produkt-Vision

Satchel behandelt E-Mails als **Workflow-Einheiten** statt als passive Nachrichten. Jede E-Mail hat einen Lebenszyklus — von Eingang über Triage bis zur Erledigung. Der Client macht diesen Lebenszyklus sichtbar, navigierbar und automatisierbar.

### 1.2 Design-Prinzipien

| Prinzip | Bedeutung |
|---------|-----------|
| **Keyboard-first** | Jede Aktion per Tastatur erreichbar, Vim-artige Modale Navigation |
| **Local-first** | Alle Daten lokal, kein Cloud-Zwang, offline-fähig |
| **Erweiterbar** | Plugin-Architektur für Actions, Integrations, UI-Erweiterungen |
| **Aufgeräumt** | Minimale, fokussierte UI — nur das Relevante ist sichtbar |
| **AI-ready** | Architektur vorbereitet für spätere AI-Integration, aber kein hartes Requirement |

### 1.3 Abgrenzung zu Vorbildern

- **Google Inbox**: Bundling, Snooze, Done-Konzept → Satchel übernimmt, erweitert um Kanban
- **Spark Mail**: Smart Inbox, Team-Features → Satchel fokussiert auf Solo/Power-User
- **Shortwave**: AI-Zusammenfassungen, Streams → Satchel: Plugin-basiert statt hardcoded
- **Superhuman**: Speed, Keyboard, Split Inbox → Satchel: Open Source, keine Cloud-Abhängigkeit

---

## 2. Tech-Stack & Begründung

### 2.1 Architektur-Übersicht

```
┌─────────────────────────────────────────────────────┐
│                    Tauri Shell                        │
│  ┌───────────────────────────────────────────────┐   │
│  │              Vue 3 Frontend                    │   │
│  │  ┌─────────┐ ┌──────────┐ ┌───────────────┐  │   │
│  │  │ Mail    │ │ Kanban   │ │ Command       │  │   │
│  │  │ Views   │ │ Board    │ │ Palette       │  │   │
│  │  ├─────────┤ ├──────────┤ ├───────────────┤  │   │
│  │  │ Thread  │ │ Workflow │ │ Vim-Mode      │  │   │
│  │  │ Display │ │ Engine   │ │ Keybindings   │  │   │
│  │  └────┬────┘ └────┬─────┘ └───────┬───────┘  │   │
│  │       │           │               │           │   │
│  │  ┌────▼───────────▼───────────────▼───────┐   │   │
│  │  │          Pinia State Store              │   │   │
│  │  └────────────────┬───────────────────────┘   │   │
│  └───────────────────┼───────────────────────────┘   │
│                      │ Tauri IPC (invoke)             │
│  ┌───────────────────▼───────────────────────────┐   │
│  │              Rust Backend                      │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────────┐  │   │
│  │  │ IMAP/    │ │ Gmail/   │ │ Plugin       │  │   │
│  │  │ SMTP     │ │ Outlook  │ │ Runtime      │  │   │
│  │  │ Engine   │ │ API      │ │ (WASM)       │  │   │
│  │  ├──────────┤ ├──────────┤ ├──────────────┤  │   │
│  │  │ Sync     │ │ Workflow │ │ Action       │  │   │
│  │  │ Manager  │ │ State    │ │ Pipeline     │  │   │
│  │  │          │ │ Machine  │ │              │  │   │
│  │  └────┬─────┘ └────┬─────┘ └──────┬───────┘  │   │
│  │       │            │              │           │   │
│  │  ┌────▼────────────▼──────────────▼───────┐   │   │
│  │  │           SQLite (sqlx)                │   │   │
│  │  │    + Full-Text Search (FTS5)           │   │   │
│  │  └────────────────────────────────────────┘   │   │
│  └───────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────┘
```

### 2.2 Stack-Entscheidungen

#### Frontend: Vue 3 + TypeScript

**Warum Vue 3:**
- Composition API für komplexe State-Logik (Mail-Threading, Kanban-Drag)
- Exzellente TypeScript-Integration
- Leichtgewichtig (~33kb gzipped) — wichtig für Tauri
- Vertrautheit aus Textura-Projekt (shared knowledge)

**Kern-Libraries:**
- `pinia` — State Management
- `vue-router` — Navigation zwischen Views (Inbox, Kanban, Settings)
- `@vueuse/core` — Composables (Keyboard, Intersection Observer, Storage)
- `tiptap` — Rich-Text Email-Compose (ProseMirror-basiert)
- `date-fns` — Datum/Zeit-Handling
- `marked` — Markdown-Darstellung im Reader

**CSS-Approach:**
- Tailwind v4 (`@tailwindcss/vite`) über CSS Custom Properties
- CSS Custom Properties für Theming
- `@container` Queries für responsive Panels

#### Backend: Rust

**Warum Rust:**
- Nativer IMAP/SMTP über `async-imap`, `lettre`
- SQLite über `sqlx` (compile-time checked queries)
- WASM-Plugin-Runtime über `wasmtime`
- Memory Safety für langlaufende Background-Sync
- Exzellente Concurrency für Multi-Account-Sync

**Kern-Crates:**
- `async-imap` + `imap-proto` — IMAP-Client
- `lettre` — SMTP/Email-Versand
- `sqlx` — Async SQLite
- `reqwest` — HTTP für Gmail/Outlook APIs
- `oauth2` — OAuth2 für Gmail/Outlook
- `serde` + `serde_json` — Serialisierung
- `wasmtime` — WASM Plugin Runtime
- `tokio` — Async Runtime
- `html2text` — HTML-zu-Text Konvertierung
- `scraper` — HTML Parsing für Email-Rendering
- `mail-parser` — RFC-compliant Email Parsing
- `keyring` — OS Credential Storage
- `notify-rust` — Desktop Notifications

#### Shell: Tauri v2

**Warum Tauri über Electron:**
- ~10x kleinere Binary (8-15MB vs 150MB+)
- Nativer Systemzugriff ohne Node.js
- Rust-Backend direkt integriert (kein IPC-Overhead)
- Multi-Window Support für Compose
- Deep Links & Custom Protocol Handler (`satchel://`)
- Auto-Updater built-in
- Mobile Support (Tauri v2) für spätere iOS/Android-Version

**Warum nicht reines Web/PWA:**
- IMAP braucht TCP Sockets (kein Browser-Support)
- Credential Storage im OS Keychain
- System Tray / Background Sync
- Native Notifications
- Filesystem Access für Attachments

### 2.3 Datenbank: SQLite mit FTS5

SQLite ist ideal für Local-first:
- Single-File Database, kein Server-Prozess
- FTS5 für Volltext-Suche über alle Mails
- WAL-Mode für concurrent reads während Sync
- Backup = File kopieren

---

## 3. Datenmodell

### 3.1 Core Schema

```sql
-- Accounts
CREATE TABLE accounts (
    id              TEXT PRIMARY KEY,  -- UUID
    display_name    TEXT NOT NULL,
    email           TEXT NOT NULL,
    provider        TEXT NOT NULL,     -- 'imap', 'gmail', 'outlook'
    config          TEXT NOT NULL,     -- JSON: server, port, security
    credentials_ref TEXT NOT NULL,     -- Referenz auf OS Keychain
    sync_state      TEXT,             -- JSON: letzte UIDs, tokens etc.
    color           TEXT,             -- Accent-Farbe für Account
    is_active       INTEGER DEFAULT 1,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- Folders (IMAP Mailboxes / Gmail Labels)
CREATE TABLE folders (
    id              TEXT PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(id),
    name            TEXT NOT NULL,
    full_path       TEXT NOT NULL,     -- z.B. "INBOX/Projects"
    folder_type     TEXT NOT NULL,     -- 'inbox','sent','drafts','trash','archive','custom'
    remote_id       TEXT,             -- IMAP name / Gmail label ID
    uidvalidity     INTEGER,
    message_count   INTEGER DEFAULT 0,
    unread_count    INTEGER DEFAULT 0,
    sync_enabled    INTEGER DEFAULT 1,
    sort_order      INTEGER DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- Messages (Emails)
CREATE TABLE messages (
    id              TEXT PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(id),
    folder_id       TEXT NOT NULL REFERENCES folders(id),
    thread_id       TEXT,             -- Konversations-Gruppierung
    message_id_header TEXT,           -- RFC Message-ID
    in_reply_to     TEXT,             -- RFC In-Reply-To
    references_header TEXT,           -- RFC References (JSON array)
    
    -- Envelope
    from_address    TEXT NOT NULL,
    from_name       TEXT,
    to_addresses    TEXT NOT NULL,    -- JSON array
    cc_addresses    TEXT,             -- JSON array
    bcc_addresses   TEXT,             -- JSON array
    reply_to        TEXT,             -- JSON array
    subject         TEXT,
    date            TEXT NOT NULL,    -- RFC 2822 date
    
    -- Content
    body_text       TEXT,             -- Plain text version
    body_html       TEXT,             -- HTML version
    snippet         TEXT,             -- Erste ~150 Zeichen
    
    -- Flags & State
    is_read         INTEGER DEFAULT 0,
    is_starred      INTEGER DEFAULT 0,
    is_draft        INTEGER DEFAULT 0,
    is_sent         INTEGER DEFAULT 0,
    has_attachments INTEGER DEFAULT 0,
    
    -- Sync
    remote_uid      INTEGER,          -- IMAP UID
    remote_id       TEXT,             -- Gmail message ID
    raw_size        INTEGER,
    raw_headers     TEXT,             -- Vollständige Headers
    
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- Volltext-Suche
CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject, body_text, from_name, from_address,
    content='messages',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

-- Threads (Konversationen)
CREATE TABLE threads (
    id              TEXT PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(id),
    subject         TEXT,             -- Normalisierter Subject
    last_message_date TEXT NOT NULL,
    message_count   INTEGER DEFAULT 1,
    unread_count    INTEGER DEFAULT 0,
    has_attachments INTEGER DEFAULT 0,
    participants    TEXT,             -- JSON array of addresses
    snippet         TEXT,             -- Snippet der letzten Nachricht
    
    -- Workflow State
    workflow_state  TEXT DEFAULT 'inbox',  -- 'inbox','todo','waiting','done','snoozed'
    workflow_changed_at TEXT,
    snooze_until    TEXT,             -- Datetime für Snooze
    due_date        TEXT,             -- Optionales Fälligkeitsdatum
    priority        INTEGER DEFAULT 0, -- 0=none, 1=low, 2=medium, 3=high
    
    -- Kanban
    board_column    TEXT,             -- Aktuelle Kanban-Spalte
    board_position  REAL,            -- Position innerhalb der Spalte (fractional indexing)
    
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- Attachments
CREATE TABLE attachments (
    id              TEXT PRIMARY KEY,
    message_id      TEXT NOT NULL REFERENCES messages(id),
    filename        TEXT,
    content_type    TEXT NOT NULL,
    size            INTEGER,
    content_id      TEXT,             -- For inline images
    storage_path    TEXT,             -- Lokaler Pfad wenn heruntergeladen
    is_inline       INTEGER DEFAULT 0,
    created_at      TEXT NOT NULL
);

-- Labels / Tags (zusätzlich zu Folders)
CREATE TABLE tags (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    color           TEXT,
    icon            TEXT,             -- Lucide icon name
    is_system       INTEGER DEFAULT 0,
    sort_order      INTEGER DEFAULT 0
);

CREATE TABLE thread_tags (
    thread_id       TEXT NOT NULL REFERENCES threads(id),
    tag_id          TEXT NOT NULL REFERENCES tags(id),
    PRIMARY KEY (thread_id, tag_id)
);

-- Workflow Rules / Automations
CREATE TABLE rules (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    description     TEXT,
    is_active       INTEGER DEFAULT 1,
    priority        INTEGER DEFAULT 0,
    
    -- Trigger
    trigger_type    TEXT NOT NULL,    -- 'on_receive','on_send','on_state_change','on_schedule'
    trigger_config  TEXT NOT NULL,    -- JSON: conditions
    
    -- Actions
    actions         TEXT NOT NULL,    -- JSON array of actions
    
    -- Stats
    last_triggered  TEXT,
    trigger_count   INTEGER DEFAULT 0,
    
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- Plugin State
CREATE TABLE plugin_state (
    plugin_id       TEXT NOT NULL,
    key             TEXT NOT NULL,
    value           TEXT,
    PRIMARY KEY (plugin_id, key)
);

-- Contacts (gecached)
CREATE TABLE contacts (
    id              TEXT PRIMARY KEY,
    email           TEXT NOT NULL UNIQUE,
    display_name    TEXT,
    avatar_url      TEXT,
    frequency       INTEGER DEFAULT 0,  -- Wie oft kommuniziert
    last_contacted  TEXT,
    notes           TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- Indices
CREATE INDEX idx_messages_thread ON messages(thread_id);
CREATE INDEX idx_messages_folder ON messages(folder_id);
CREATE INDEX idx_messages_date ON messages(date DESC);
CREATE INDEX idx_messages_account ON messages(account_id);
CREATE INDEX idx_threads_workflow ON threads(workflow_state);
CREATE INDEX idx_threads_board ON threads(board_column, board_position);
CREATE INDEX idx_threads_snooze ON threads(snooze_until) WHERE snooze_until IS NOT NULL;
CREATE INDEX idx_threads_date ON threads(last_message_date DESC);
CREATE INDEX idx_folders_account ON folders(account_id);
CREATE INDEX idx_attachments_message ON attachments(message_id);
```

### 3.2 Thread-Building Algorithmus

Threads werden durch eine Kombination aufgebaut:

1. **Message-ID / In-Reply-To / References** — primäre Zuordnung (RFC 5256 ORDEREDSUBJECT)
2. **Subject-Normalisierung** — Fallback: `Re:`, `Fwd:`, `AW:`, `WG:` strippen, case-insensitive match
3. **Gmail-Thread-ID** — direkte Übernahme bei Gmail-Accounts
4. **Zeitfenster** — Nachrichten mit gleichem normalisierten Subject innerhalb von 30 Tagen

```rust
// Pseudo-Algorithmus
fn assign_thread(message: &Message, existing_threads: &[Thread]) -> ThreadId {
    // 1. Direkte Referenz
    if let Some(in_reply_to) = &message.in_reply_to {
        if let Some(thread) = find_thread_by_message_id(in_reply_to) {
            return thread.id;
        }
    }
    
    // 2. References-Chain
    for ref_id in message.references.iter().rev() {
        if let Some(thread) = find_thread_by_message_id(ref_id) {
            return thread.id;
        }
    }
    
    // 3. Subject-Match (mit Zeitfenster)
    let normalized = normalize_subject(&message.subject);
    if let Some(thread) = find_thread_by_subject(&normalized, message.date, 30.days()) {
        return thread.id;
    }
    
    // 4. Neuer Thread
    create_new_thread(message)
}
```

---

## 4. Sync-Engine

### 4.1 Multi-Provider Abstraction

```rust
#[async_trait]
pub trait MailProvider: Send + Sync {
    /// Initialen Sync aller Folder/Labels durchführen
    async fn sync_folders(&self) -> Result<Vec<RemoteFolder>>;
    
    /// Neue/geänderte Nachrichten seit letztem Sync abrufen
    async fn sync_messages(&self, folder: &str, since: SyncCheckpoint) 
        -> Result<SyncBatch>;
    
    /// Auf Echtzeit-Änderungen warten (IMAP IDLE / Push)
    async fn watch(&self, folders: &[String], tx: Sender<MailEvent>) 
        -> Result<()>;
    
    /// Nachricht senden
    async fn send(&self, message: OutgoingMessage) -> Result<()>;
    
    /// Flags setzen (read, starred, etc.)
    async fn set_flags(&self, uids: &[MessageRef], flags: FlagUpdate) 
        -> Result<()>;
    
    /// Nachrichten verschieben
    async fn move_messages(&self, uids: &[MessageRef], target: &str) 
        -> Result<()>;
    
    /// Nachrichten löschen
    async fn delete_messages(&self, uids: &[MessageRef]) -> Result<()>;
}

pub enum MailEvent {
    NewMessage(MessageEnvelope),
    FlagsChanged { uid: u32, flags: Vec<Flag> },
    MessageExpunged(u32),
    FolderChanged(String),
}
```

### 4.2 IMAP Sync-Strategie

```
Phase 1: Initial Sync
├── LIST/LSUB → alle Mailboxen holen
├── Für jede Mailbox:
│   ├── SELECT → UIDVALIDITY + EXISTS prüfen
│   ├── FETCH 1:* (ENVELOPE FLAGS BODYSTRUCTURE) → Alle Envelopes
│   └── Body on-demand laden (erst bei Thread-Öffnung)
└── Thread-Building über alle Accounts

Phase 2: Incremental Sync
├── UIDVALIDITY prüfen (bei Änderung → Full Resync der Box)
├── Neue UIDs seit letztem bekannten UID fetchen
├── CONDSTORE (RFC 7162): FETCH CHANGEDSINCE für Flag-Updates
└── Thread-Zuordnung für neue Nachrichten

Phase 3: Real-time
├── IMAP IDLE auf INBOX + beobachtete Folder
├── Bei EXISTS-Änderung → Incremental Fetch
├── Reconnect-Logic mit exponential backoff
└── Multi-Folder: rotating IDLE (IMAP erlaubt nur 1 IDLE pro Connection)
```

### 4.3 Gmail API Sync

```
Phase 1: Initial Sync
├── users.labels.list → Alle Labels
├── users.threads.list → Thread-IDs (paginiert)
├── Batch: users.threads.get (format=metadata) → Envelopes
└── Content on-demand: users.messages.get (format=full)

Phase 2: Incremental
├── users.history.list(startHistoryId) → Deltas
│   ├── messagesAdded
│   ├── messagesDeleted
│   ├── labelsAdded
│   └── labelsRemoved
└── Thread-Updates anwenden

Phase 3: Real-time
├── users.watch → Push Notification an Webhook
│   (benötigt Pub/Sub oder lokalen Polling-Fallback)
└── Fallback: Polling alle 30s mit history.list
```

### 4.4 Outlook/Microsoft Graph Sync

```
Phase 1: Initial Sync
├── GET /me/mailFolders → Alle Folder
├── GET /me/messages?$orderby=receivedDateTime → Nachrichten (paginiert, $top=50)
└── Content: $select=body,attachments bei Thread-Öffnung

Phase 2: Incremental
├── Delta Query: GET /me/messages/delta
│   → gibt geänderte Nachrichten + @odata.deltaLink
└── Delta-Token speichern für nächsten Sync

Phase 3: Real-time
├── Subscriptions: POST /subscriptions (Webhook)
│   (benötigt erreichbaren Endpoint oder Polling-Fallback)
└── Fallback: Polling alle 30s mit delta query
```

### 4.5 Offline & Conflict Resolution

**Strategie: Server wins, local queue**

Offline-Aktionen werden in einer lokalen Queue gespeichert:

```sql
CREATE TABLE sync_queue (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL,
    action_type TEXT NOT NULL,  -- 'move','flag','send','delete'
    payload     TEXT NOT NULL,  -- JSON
    status      TEXT DEFAULT 'pending',  -- 'pending','syncing','failed','done'
    retry_count INTEGER DEFAULT 0,
    created_at  TEXT NOT NULL
);
```

Bei Reconnect: Queue abarbeiten in FIFO-Reihenfolge. Bei Konflikten (z.B. Nachricht zwischenzeitlich gelöscht): Action verwerfen + User benachrichtigen.

---

## 5. Workflow-Engine — Das Herzstück

### 5.1 Thread Lifecycle (State Machine)

```
                    ┌─────────┐
           ┌────────│  inbox  │────────┐
           │        └────┬────┘        │
           │             │             │
      [snooze]      [triage]      [archive]
           │             │             │
           ▼             ▼             ▼
    ┌──────────┐   ┌─────────┐   ┌─────────┐
    │ snoozed  │   │  todo   │   │  done   │
    └────┬─────┘   └────┬────┘   └─────────┘
         │              │             ▲
    [wake up]     [reply/act]         │
         │              │        [complete]
         ▼              ▼             │
    ┌─────────┐   ┌──────────┐       │
    │  inbox  │   │ waiting  │───────┘
    └─────────┘   └──────────┘
                   (Antwort erhalten
                    → zurück zu todo
                    oder auto-done)
```

### 5.2 Automatische State-Transitions

Die "Waiting → Antwort → Todo"-Logik funktioniert so:

```rust
pub struct WorkflowEngine {
    rules: Vec<TransitionRule>,
}

pub struct TransitionRule {
    from_state: WorkflowState,
    trigger: Trigger,
    to_state: WorkflowState,
    conditions: Vec<Condition>,
    actions: Vec<PostAction>,
}

pub enum Trigger {
    /// Neue Nachricht im Thread
    NewMessageInThread,
    /// Snooze-Timer abgelaufen
    SnoozeExpired,
    /// Manueller State-Change
    UserAction,
    /// Zeitbasiert (z.B. "nach 3 Tagen ohne Antwort")
    TimeBased { duration: Duration },
    /// Externer Event (Plugin)
    PluginEvent(String),
}

// Konkrete Default-Regeln:
fn default_rules() -> Vec<TransitionRule> {
    vec![
        // Wenn ich auf "Waiting" setze und eine Antwort kommt → Todo
        TransitionRule {
            from_state: Waiting,
            trigger: NewMessageInThread,
            to_state: Todo,
            conditions: vec![
                // Nur wenn die Antwort NICHT von mir kommt
                Condition::SenderIsNot(AccountOwner),
            ],
            actions: vec![
                PostAction::Notify("Antwort erhalten"),
                PostAction::SetPriority(Priority::Medium),
            ],
        },
        
        // Snooze abgelaufen → zurück in Inbox
        TransitionRule {
            from_state: Snoozed,
            trigger: SnoozeExpired,
            to_state: Inbox,
            conditions: vec![],
            actions: vec![
                PostAction::Notify("Snoozed thread zurück"),
                PostAction::MoveToTop,
            ],
        },
        
        // Nachricht gesendet → Auto-Waiting
        TransitionRule {
            from_state: Todo,
            trigger: NewMessageInThread,
            to_state: Waiting,
            conditions: vec![
                Condition::SenderIs(AccountOwner),
            ],
            actions: vec![],
        },
        
        // 7 Tage in "Waiting" ohne Antwort → Reminder
        TransitionRule {
            from_state: Waiting,
            trigger: TimeBased { duration: Duration::days(7) },
            to_state: Todo, // zurück in Todo mit Reminder-Tag
            conditions: vec![],
            actions: vec![
                PostAction::AddTag("follow-up"),
                PostAction::Notify("Keine Antwort seit 7 Tagen"),
            ],
        },
    ]
}
```

### 5.3 Wie erfährst du von Änderungen?

Der zentrale Event-Bus verbindet Sync-Engine mit Workflow-Engine:

```rust
pub enum AppEvent {
    // Von Sync-Engine
    MessageReceived { thread_id: ThreadId, message: MessageEnvelope },
    MessageSent { thread_id: ThreadId, message: MessageEnvelope },
    FlagsChanged { message_id: MessageId, flags: Vec<Flag> },
    
    // Von Workflow-Engine
    WorkflowStateChanged { thread_id: ThreadId, from: State, to: State },
    SnoozeTriggered { thread_id: ThreadId },
    RuleTriggered { rule_id: RuleId, thread_id: ThreadId },
    
    // Von Plugin-System
    PluginAction { plugin_id: String, action: serde_json::Value },
}

// Event-Bus Implementierung
pub struct EventBus {
    subscribers: HashMap<EventType, Vec<Sender<AppEvent>>>,
}

impl EventBus {
    pub async fn publish(&self, event: AppEvent) {
        // 1. Workflow-Engine evaluiert Rules
        // 2. UI wird über Tauri Events benachrichtigt
        // 3. Plugins erhalten Events
        // 4. Sync-Queue wird ggf. befüllt
    }
}
```

**Konkreter Flow: "Waiting → Antwort erhalten"**

```
1. IMAP IDLE meldet neue Nachricht in INBOX
2. Sync-Engine: fetch envelope → parse → in DB speichern
3. Thread-Builder: Nachricht dem existierenden Thread zuordnen
4. Event: MessageReceived { thread_id: "xyz", message: ... }
5. Workflow-Engine: Thread "xyz" ist in State "waiting"
   → Rule matched: NewMessageInThread + SenderIsNot(Owner)
   → Transition: waiting → todo
6. PostActions: Notification senden, Priority setzen
7. Event: WorkflowStateChanged { thread_id: "xyz", from: waiting, to: todo }
8. Frontend via Tauri Event: Thread verschiebt sich im Kanban Board
9. Desktop Notification: "Antwort von Max auf 'Budget Q3'"
```

---

## 6. Kanban-Board

### 6.1 Spalten-Konfiguration

Default-Spalten bilden den Workflow ab, sind aber benutzerdefinierbar:

```typescript
interface BoardColumn {
  id: string
  name: string
  workflowState: WorkflowState  // Mapping auf State Machine
  icon: string                   // Lucide icon
  color: string                  // Accent color
  limit?: number                 // WIP Limit
  autoCollapse?: boolean         // Automatisch einklappen wenn leer
  sortBy: 'date' | 'priority' | 'manual'
}

const defaultColumns: BoardColumn[] = [
  { id: 'inbox',   name: 'Inbox',    workflowState: 'inbox',   icon: 'inbox',       color: '#3b82f6', sortBy: 'date' },
  { id: 'todo',    name: 'Todo',     workflowState: 'todo',    icon: 'circle-dot',  color: '#f59e0b', limit: 10, sortBy: 'priority' },
  { id: 'waiting', name: 'Waiting',  workflowState: 'waiting', icon: 'clock',       color: '#8b5cf6', sortBy: 'date' },
  { id: 'done',    name: 'Done',     workflowState: 'done',    icon: 'check-circle', color: '#22c55e', autoCollapse: true, sortBy: 'date' },
]
```

### 6.2 Kanban-Karten

Jede Karte repräsentiert einen Thread:

```typescript
interface KanbanCard {
  threadId: string
  subject: string
  participants: Participant[]
  snippet: string
  lastDate: Date
  unreadCount: number
  hasAttachments: boolean
  priority: Priority
  tags: Tag[]
  dueDate?: Date
  
  // Visual
  accountColor: string    // Farbiger Sidebar-Streifen
  isOverdue: boolean
  waitingSince?: Date     // Für "Waiting"-Spalte: Tage-Counter
}
```

### 6.3 Drag & Drop Logik

```typescript
function onCardDrop(threadId: string, targetColumn: string, targetIndex: number) {
  // 1. Optimistic UI Update
  moveCardInStore(threadId, targetColumn, targetIndex)
  
  // 2. Workflow State Transition
  const newState = columnToState(targetColumn)
  invoke('transition_workflow', { threadId, newState })
  
  // 3. Position berechnen (Fractional Indexing)
  const position = calculatePosition(targetColumn, targetIndex)
  invoke('update_board_position', { threadId, column: targetColumn, position })
  
  // 4. Ggf. IMAP-Actions auslösen
  if (newState === 'done') {
    invoke('archive_thread', { threadId })  // → IMAP MOVE to Archive
  }
}
```

---

## 7. Keyboard Navigation & Vim-Mode

### 7.1 Mode-System

```typescript
type Mode = 'normal' | 'visual' | 'command' | 'compose' | 'search'

// Normal Mode (Standard)
const normalBindings = {
  // Navigation
  'j': 'next-thread',
  'k': 'prev-thread',
  'l': 'open-thread',        // oder Enter
  'h': 'back-to-list',       // oder Escape
  'g g': 'go-to-top',
  'G': 'go-to-bottom',
  'Ctrl+d': 'half-page-down',
  'Ctrl+u': 'half-page-up',
  
  // Quick Actions
  'e': 'archive',             // Done/Archive
  '#': 'trash',
  's': 'star-toggle',
  'r': 'reply',
  'R': 'reply-all',
  'f': 'forward',
  'c': 'compose',
  'u': 'mark-unread',
  
  // Workflow
  't': 'move-to-todo',
  'w': 'move-to-waiting',
  'z': 'snooze-menu',         // Öffnet Snooze-Picker
  'p 1/2/3': 'set-priority',
  
  // Kanban
  'b': 'toggle-board-view',
  '1-9': 'move-to-column-n',
  
  // Views
  'g i': 'go-inbox',
  'g t': 'go-todo',
  'g w': 'go-waiting',
  'g d': 'go-done',
  'g b': 'go-board',
  'g s': 'go-sent',
  
  // Search & Command
  '/': 'search',
  ':': 'command-palette',
  '?': 'show-shortcuts',
  
  // Selection
  'x': 'toggle-select',
  'v': 'enter-visual-mode',   // Bereich auswählen
}

// Command Palette (: oder Ctrl+K)
// Fuzzy-matched Commands wie in VSCode/Supermaven:
// :move @project-x      → Tag zuweisen
// :snooze tomorrow 9am  → Snooze mit NLP
// :rule from:boss → todo priority:high
// :send-to github       → Plugin-Action
```

### 7.2 Command Palette

Die Command Palette ist das zentrale Steuerungselement:

```typescript
interface Command {
  id: string
  name: string
  description?: string
  shortcut?: string
  category: 'navigation' | 'action' | 'workflow' | 'plugin' | 'settings'
  execute: (args?: string) => void | Promise<void>
  isAvailable?: () => boolean   // Context-dependent
  icon?: string
}

// Fuzzy Search über alle Commands + Plugin-Commands
// NLP für zeitbasierte Inputs:
// "snooze until monday" → parst zu nächsten Montag 9:00
// "remind me in 2 hours" → snooze + 2h
```

---

## 8. Plugin-System

### 8.1 Plugin-Architektur

Plugins laufen in einer WASM-Sandbox (wasmtime) für Sicherheit und Portabilität:

```rust
// Plugin Interface (wit-bindgen)
interface plugin {
    // Plugin-Metadaten
    record metadata {
        id: string,
        name: string,
        version: string,
        description: string,
        permissions: list<permission>,
    }
    
    enum permission {
        read-messages,
        modify-messages,
        send-messages,
        network-access,
        storage,
        notifications,
    }
    
    // Plugin Lifecycle
    initialize: func() -> result<_, string>
    
    // Event Handler — Plugin wählt welche Events es empfängt
    on-event: func(event: mail-event) -> list<action>
    
    // Command Handler — registriert Commands in der Palette
    get-commands: func() -> list<command-def>
    execute-command: func(command-id: string, args: string) -> result<_, string>
    
    // UI Extension Point — liefert HTML/Vue-Komponente
    render-panel: func(context: panel-context) -> option<string>
}
```

### 8.2 Beispiel-Plugin: GitHub Integration

```rust
// github-integration/src/lib.rs
use satchel_plugin_sdk::*;

struct GitHubPlugin {
    api_token: String,
}

impl Plugin for GitHubPlugin {
    fn metadata() -> Metadata {
        Metadata {
            id: "github-integration",
            name: "GitHub Integration",
            permissions: vec![
                Permission::ReadMessages,
                Permission::NetworkAccess,
                Permission::Storage,
            ],
        }
    }
    
    fn on_event(&mut self, event: MailEvent) -> Vec<Action> {
        match event {
            MailEvent::MessageReceived(msg) => {
                // Prüfe ob Mail ein GitHub Notification ist
                if msg.from.contains("notifications@github.com") {
                    vec![
                        Action::AddTag("github"),
                        Action::SetWorkflowState(WorkflowState::Todo),
                    ]
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }
    
    fn get_commands() -> Vec<CommandDef> {
        vec![
            CommandDef {
                id: "create-issue",
                name: "Create GitHub Issue from Email",
                description: "Erstellt ein GitHub Issue basierend auf dem Inhalt der Mail",
            },
            CommandDef {
                id: "link-pr",
                name: "Link Pull Request",
                description: "Verknüpft die Mail mit einem PR",
            },
        ]
    }
    
    async fn execute_command(&mut self, cmd: &str, context: &CommandContext) -> Result<()> {
        match cmd {
            "create-issue" => {
                let thread = context.current_thread()?;
                let issue = GitHubIssue {
                    title: thread.subject.clone(),
                    body: format!(
                        "Created from email by {}\n\n---\n\n{}",
                        thread.participants.first().map(|p| &p.name).unwrap_or(&"Unknown"),
                        thread.latest_message().body_text
                    ),
                    labels: vec!["from-email".into()],
                };
                
                let repo = context.prompt_user("Repository (owner/repo):")?;
                self.github_api.create_issue(&repo, &issue).await?;
                
                Ok(())
            }
            _ => Err(anyhow!("Unknown command")),
        }
    }
}
```

### 8.3 Weitere Plugin-Ideen

| Plugin | Trigger | Action |
|--------|---------|--------|
| **Calendar Sync** | Mail enthält .ics Attachment | Event in Kalender anzeigen/akzeptieren |
| **Auto-Unsubscribe** | Newsletter erkannt | Unsubscribe-Link extrahieren, One-Click Abmeldung |
| **Smart Reply** (AI) | Beliebige Mail | Draft-Vorschläge generieren |
| **Expense Tracker** | Mail enthält Rechnung/PDF | Betrag extrahieren, kategorisieren |
| **CRM Link** | Mail von bekanntem Kontakt | Link zu CRM-Eintrag anzeigen |
| **Code Review** | GitHub/GitLab PR-Notification | Diff anzeigen, Review-Kommentar per Mail |
| **Translation** | Fremdsprachige Mail | Automatische Übersetzung |

---

## 9. UI/UX Design

### 9.1 Layout-Konzept

```
┌──────────────────────────────────────────────────────────────┐
│ ◉ Satchel          [Unified Search (/)]           ⚙ 👤    │
├────────┬─────────────────────────────────────────────────────┤
│        │                                                     │
│  📥 ⁷  │  ┌─ Inbox ──────────────────────────────────────┐  │
│  ✅ ³  │  │                                               │  │
│  ⏳ ²  │  │  ▸ Team Standup (3)            vor 5 min      │  │
│  ✓ ¹²  │  │    Max, Lisa, Tom             "Hier ist..."   │  │
│        │  │                                               │  │
│  ──────│  │  ▸ Bug Report: Login fails (2)  vor 1 Std    │  │
│  Tags  │  │    sarah@client.io              🔴 P1          │  │
│  📁 Pro│  │                                               │  │
│  📁 Fin│  │  ▸ Invoice March               gestern        │  │
│  🏷 git│  │    accounting@vendor.io         📎 1            │  │
│        │  │                                               │  │
│  ──────│  │  ── Earlier ───────────────────────────        │  │
│  Accts │  │                                               │  │
│  🔵 W  │  │  ▸ Quarterly Review            Mo, 14:30     │  │
│  🟢 P  │  │    management@company.io                      │  │
│        │  │                                               │  │
│        │  └───────────────────────────────────────────────┘  │
│        │                                                     │
│ [?] ▁▁▁│  ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁ j/k ↕ l → ▁▁▁▁▁ │
└────────┴─────────────────────────────────────────────────────┘
```

### 9.2 Thread-Ansicht

```
┌──────────────────────────────────────────────────────────────┐
│ ← Back (h)    Bug Report: Login fails       🔴 P1  ⏳ 🏷   │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌── sarah@client.io ──────────── gestern, 14:30 ──────┐   │
│  │                                                       │   │
│  │  Hi Team,                                             │   │
│  │                                                       │   │
│  │  we're experiencing login failures on the staging     │   │
│  │  environment since this morning's deployment.         │   │
│  │                                                       │   │
│  │  Steps to reproduce:                                  │   │
│  │  1. Go to staging.app.io/login                        │   │
│  │  2. Enter valid credentials                           │   │
│  │  3. Click "Sign In"                                   │   │
│  │  4. → 500 Internal Server Error                       │   │
│  │                                                       │   │
│  │  📎 error-log.txt (12 KB)                             │   │
│  │                                                       │   │
│  └───────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌── you ─────────────────────── gestern, 15:10 ──────┐    │
│  │  Looking into it. Seems related to the OAuth         │    │
│  │  config change. Will update in 30 min.               │    │
│  └──────────────────────────────────────────────────────┘    │
│                                                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  r Reply  R Reply All  f Forward    :cmd  🤖 Plugin │    │
│  └─────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────┘
```

### 9.3 Kanban-Ansicht

```
┌──────────────────────────────────────────────────────────────┐
│ ← List  🔲 Board View (b)                     Filter ▾ Sort │
├────────────────┬────────────────┬────────────────┬───────────┤
│  📥 Inbox (7)  │  ✅ Todo (3)   │  ⏳ Waiting (2) │  ✓ Done  │
├────────────────┼────────────────┼────────────────┼───────────┤
│ ┌────────────┐ │ ┌────────────┐ │ ┌────────────┐ │           │
│ │Team Standup│ │ │🔴 Bug Rep. │ │ │Invoice Mar │ │           │
│ │Max +2  5m  │ │ │sarah@ 1d   │ │ │vendor  2d  │ │           │
│ │3 msgs      │ │ │📎 P1       │ │ │📎 waiting  │ │           │
│ └────────────┘ │ └────────────┘ │ │since 2d    │ │           │
│ ┌────────────┐ │ ┌────────────┐ │ └────────────┘ │           │
│ │Newsletter  │ │ │Onboarding  │ │ ┌────────────┐ │           │
│ │digest  12h │ │ │New hire 3d │ │ │PR Review   │ │           │
│ └────────────┘ │ │due: Fr     │ │ │github  5d  │ │           │
│ ┌────────────┐ │ └────────────┘ │ └────────────┘ │           │
│ │Conf invite │ │ ┌────────────┐ │                │           │
│ │events@  1d │ │ │Tax docs    │ │                │           │
│ └────────────┘ │ │accountant  │ │                │           │
│        ⋮       │ └────────────┘ │                │           │
└────────────────┴────────────────┴────────────────┴───────────┘
```

### 9.4 Design-Tokens

```css
:root {
  /* Base */
  --bg-primary: #0f0f0f;
  --bg-secondary: #1a1a1a;
  --bg-tertiary: #252525;
  --bg-hover: #2a2a2a;
  --bg-selected: #1e3a5f;
  
  /* Text */
  --text-primary: #e5e5e5;
  --text-secondary: #999;
  --text-tertiary: #666;
  --text-inverse: #0f0f0f;
  
  /* Accents */
  --accent-primary: #3b82f6;
  --accent-purple: #8b5cf6;
  --accent-amber: #f59e0b;
  --accent-green: #22c55e;
  --accent-red: #ef4444;
  
  /* Workflow States */
  --state-inbox: var(--accent-primary);
  --state-todo: var(--accent-amber);
  --state-waiting: var(--accent-purple);
  --state-done: var(--accent-green);
  --state-snoozed: var(--text-tertiary);
  
  /* Priority */
  --priority-high: var(--accent-red);
  --priority-medium: var(--accent-amber);
  --priority-low: var(--accent-primary);
  
  /* Typography */
  --font-sans: 'IBM Plex Sans', system-ui, sans-serif;
  --font-mono: 'JetBrains Mono', 'Fira Code', monospace;
  
  /* Spacing */
  --space-xs: 4px;
  --space-sm: 8px;
  --space-md: 16px;
  --space-lg: 24px;
  --space-xl: 32px;
  
  /* Borders */
  --border-subtle: 1px solid #2a2a2a;
  --border-focus: 2px solid var(--accent-primary);
  --radius-sm: 6px;
  --radius-md: 8px;
  --radius-lg: 12px;
  
  /* Shadows */
  --shadow-card: 0 2px 8px rgba(0,0,0,0.3);
  --shadow-dropdown: 0 8px 24px rgba(0,0,0,0.5);
  
  /* Transitions */
  --transition-fast: 100ms ease;
  --transition-normal: 200ms ease;
}

/* Light Theme Override */
[data-theme="light"] {
  --bg-primary: #ffffff;
  --bg-secondary: #f5f5f5;
  --bg-tertiary: #ebebeb;
  --bg-hover: #e5e5e5;
  --bg-selected: #dbeafe;
  --text-primary: #1a1a1a;
  --text-secondary: #666;
  --text-tertiary: #999;
  --border-subtle: 1px solid #e5e5e5;
}
```

### 9.5 Email-Rendering Pipeline

HTML-Emails sicher und sauber darstellen:

```
Eingehende HTML-Mail
    │
    ▼
┌────────────────────┐
│ 1. Sanitize        │  ammonia (Rust): XSS-Schutz, script/iframe entfernen
│    (Rust-Side)      │  Erlaubte Tags: p, div, span, a, img, table, br, hr,
│                     │  h1-h6, ul, ol, li, blockquote, pre, code
└────────┬────────────┘
         │
    ▼
┌────────────────────┐
│ 2. Transform       │  - Externe Bilder: Proxy oder Platzhalter + "Load images"
│    (Rust-Side)      │  - Links: target="_blank" + rel="noopener"
│                     │  - CSS: Inline-only, kein <style> Block
└────────┬────────────┘
         │
    ▼
┌────────────────────┐
│ 3. Render          │  - Shadow DOM im Frontend für Style-Isolation
│    (Vue-Side)       │  - Responsive: max-width: 100%, overflow-x: auto
│                     │  - Dark Mode: CSS filter invert für helle Mails
└────────────────────┘
```

---

## 10. Projektstruktur

```
satchel/
├── src-tauri/                    # Rust Backend
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs              # Tauri setup & startup
│   │   ├── lib.rs               # Module exports
│   │   ├── commands/            # Tauri IPC Commands
│   │   │   ├── mod.rs
│   │   │   ├── accounts.rs      # Account CRUD
│   │   │   ├── messages.rs      # Message fetch/search/send
│   │   │   ├── threads.rs       # Thread operations
│   │   │   ├── workflow.rs      # State transitions
│   │   │   ├── rules.rs         # Automation rules
│   │   │   └── plugins.rs       # Plugin management
│   │   │
│   │   ├── providers/           # Mail Provider Implementations
│   │   │   ├── mod.rs
│   │   │   ├── traits.rs        # MailProvider trait
│   │   │   ├── imap.rs          # IMAP/SMTP implementation
│   │   │   ├── gmail.rs         # Gmail API implementation
│   │   │   └── outlook.rs       # Microsoft Graph implementation
│   │   │
│   │   ├── sync/                # Sync Engine
│   │   │   ├── mod.rs
│   │   │   ├── mailbox_sync.rs  # Sync orchestration
│   │   │   ├── queue.rs         # Offline action queue
│   │   │   ├── threading.rs     # Thread-building algorithm
│   │   │   └── watcher.rs       # IDLE / Push notification handler
│   │   │
│   │   ├── workflow/            # Workflow Engine
│   │   │   ├── mod.rs
│   │   │   ├── engine.rs        # Rule evaluation & state transitions
│   │   │   ├── states.rs        # State definitions & transitions
│   │   │   └── scheduler.rs     # Snooze & time-based triggers
│   │   │
│   │   ├── plugins/             # Plugin Runtime
│   │   │   ├── mod.rs
│   │   │   ├── runtime.rs       # WASM runtime (wasmtime)
│   │   │   ├── api.rs           # Host functions exposed to plugins
│   │   │   └── registry.rs      # Plugin discovery & loading
│   │   │
│   │   ├── db/                  # Database Layer
│   │   │   ├── mod.rs
│   │   │   ├── migrations/      # SQL migrations
│   │   │   ├── queries.rs       # Prepared queries
│   │   │   └── search.rs        # FTS5 search interface
│   │   │
│   │   ├── email/               # Email Processing
│   │   │   ├── mod.rs
│   │   │   ├── parser.rs        # RFC-compliant parsing
│   │   │   ├── composer.rs      # Email composition
│   │   │   ├── sanitizer.rs     # HTML sanitization
│   │   │   └── renderer.rs      # HTML transformation pipeline
│   │   │
│   │   ├── events/              # Event System
│   │   │   ├── mod.rs
│   │   │   └── bus.rs           # Async event bus
│   │   │
│   │   └── utils/
│   │       ├── mod.rs
│   │       ├── crypto.rs        # Keychain integration
│   │       └── config.rs        # App configuration
│   │
│   ├── tauri.conf.json
│   └── capabilities/            # Tauri v2 permissions
│
├── src/                          # Vue 3 Frontend
│   ├── App.vue
│   ├── main.ts
│   │
│   ├── components/
│   │   ├── layout/
│   │   │   ├── Sidebar.vue
│   │   │   ├── MainPanel.vue
│   │   │   ├── DetailPanel.vue
│   │   │   └── StatusBar.vue
│   │   │
│   │   ├── mail/
│   │   │   ├── ThreadList.vue
│   │   │   ├── ThreadListItem.vue
│   │   │   ├── ThreadView.vue
│   │   │   ├── MessageBubble.vue
│   │   │   ├── EmailRenderer.vue  # Sandboxed HTML render
│   │   │   ├── ComposeEditor.vue  # Tiptap-basiert
│   │   │   └── AttachmentChip.vue
│   │   │
│   │   ├── kanban/
│   │   │   ├── Board.vue
│   │   │   ├── Column.vue
│   │   │   ├── Card.vue
│   │   │   └── ColumnConfig.vue
│   │   │
│   │   ├── navigation/
│   │   │   ├── CommandPalette.vue  # :cmd / Ctrl+K
│   │   │   ├── SearchOverlay.vue   # /search
│   │   │   ├── ShortcutHelper.vue  # ? overlay
│   │   │   └── SnoozePicker.vue
│   │   │
│   │   ├── common/
│   │   │   ├── Avatar.vue
│   │   │   ├── Badge.vue
│   │   │   ├── TagChip.vue
│   │   │   ├── PriorityIndicator.vue
│   │   │   ├── TimeAgo.vue
│   │   │   └── Tooltip.vue
│   │   │
│   │   └── settings/
│   │       ├── AccountSetup.vue
│   │       ├── KeybindingEditor.vue
│   │       ├── RuleBuilder.vue
│   │       ├── PluginManager.vue
│   │       └── ThemeSelector.vue
│   │
│   ├── composables/
│   │   ├── useKeyboard.ts         # Vim-mode & keybinding engine
│   │   ├── useThreadNavigation.ts # j/k/l/h navigation
│   │   ├── useWorkflow.ts         # State transition helpers
│   │   ├── useSearch.ts           # Unified search
│   │   ├── useDragDrop.ts         # Kanban DnD
│   │   └── useNotifications.ts
│   │
│   ├── stores/
│   │   ├── accounts.ts
│   │   ├── threads.ts
│   │   ├── messages.ts
│   │   ├── workflow.ts
│   │   ├── kanban.ts
│   │   ├── ui.ts                  # UI state (active view, panels, mode)
│   │   └── settings.ts
│   │
│   ├── views/
│   │   ├── InboxView.vue
│   │   ├── BoardView.vue
│   │   ├── ThreadDetailView.vue
│   │   ├── ComposeView.vue
│   │   ├── SearchView.vue
│   │   └── SettingsView.vue
│   │
│   ├── styles/
│   │   ├── tokens.css             # Design tokens
│   │   ├── base.css               # Reset & global styles
│   │   └── transitions.css        # Shared animations
│   │
│   └── types/
│       ├── mail.ts
│       ├── workflow.ts
│       ├── kanban.ts
│       └── plugin.ts
│
├── plugins/                       # Built-in Plugins (WASM)
│   ├── github-integration/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── calendar-sync/
│   └── auto-unsubscribe/
│
├── plugin-sdk/                    # Plugin SDK (veröffentlicht auf crates.io)
│   ├── Cargo.toml
│   ├── src/lib.rs
│   └── wit/                       # WebAssembly Interface Types
│       └── plugin.wit
│
├── package.json
├── vite.config.ts
├── tsconfig.json
├── uno.config.ts
└── README.md
```

---

## 11. Entwicklungs-Roadmap

### Phase 0 — Foundation (2-3 Wochen)

- [ ] Tauri v2 + Vue 3 + TypeScript Projekt-Scaffolding
- [ ] SQLite Schema + Migrations Setup
- [ ] Design Tokens + Base Layout (Sidebar, Main, Detail)
- [ ] Grundlegende Tauri IPC-Kommunikation
- [ ] CI/CD Pipeline (GitHub Actions: build, test, lint für Rust + TS)

### Phase 1 — Core Mail (4-6 Wochen)

- [ ] IMAP Provider Implementation
  - [ ] Account Setup (Server, Credentials, Keychain)
  - [ ] Initial Sync (Folder List, Envelopes)
  - [ ] Incremental Sync
  - [ ] IDLE für Real-time
- [ ] Thread-Building Algorithmus
- [ ] ThreadList-Komponente (virtualisiert für Performance)
- [ ] ThreadView mit Message-Bubbles
- [ ] Email HTML Renderer (Sanitize + Shadow DOM)
- [ ] Basis-Aktionen: Reply, Forward, Archive, Delete, Star
- [ ] SMTP: Compose + Send
- [ ] Offline Queue (Aktionen bei Reconnect syncen)

### Phase 2 — Workflow (3-4 Wochen)

- [ ] Workflow State Machine
- [ ] Sidebar-Navigation (Inbox, Todo, Waiting, Done)
- [ ] State Transitions (manuell: Keyboard + Drag)
- [ ] Automatische Transitions (Waiting → Antwort → Todo)
- [ ] Snooze-System (Timer + State Restore)
- [ ] Priority-System
- [ ] Tag-System
- [ ] Notification-Integration

### Phase 3 — Keyboard & UX (2-3 Wochen)

- [ ] Vim-Mode Keybinding Engine
- [ ] Command Palette mit Fuzzy Search
- [ ] Shortcut Overlay (?)
- [ ] Keyboard-navigierbare Liste (j/k/l/h)
- [ ] Quick-Actions Bar
- [ ] Kontextuelle Shortcuts (verschiedene Modi)
- [ ] Customizable Keybindings

### Phase 4 — Kanban Board (2-3 Wochen)

- [ ] Board-Ansicht mit Spalten
- [ ] Drag & Drop zwischen Spalten
- [ ] Fractional Indexing für Reihenfolge
- [ ] WIP Limits
- [ ] Spalten-Konfiguration
- [ ] Bidirektionaler Sync: Board ↔ Workflow State
- [ ] Due Dates & Overdue-Anzeige

### Phase 5 — Gmail & Outlook (3-4 Wochen)

- [ ] Gmail API Provider
  - [ ] OAuth2 Flow
  - [ ] Labels → Folders Mapping
  - [ ] History API für Incremental Sync
  - [ ] Native Thread-IDs nutzen
- [ ] Microsoft Graph Provider
  - [ ] OAuth2 Flow
  - [ ] Delta Queries
  - [ ] Category Mapping
- [ ] Multi-Account Unified Inbox

### Phase 6 — Plugin-System (3-4 Wochen)

- [ ] WASM Plugin Runtime (wasmtime)
- [ ] Plugin SDK + WIT Definitions
- [ ] Plugin Lifecycle Management
- [ ] Host API: Messages, Storage, Network, Notifications
- [ ] Plugin UI Extension Points
- [ ] Command Registration in Palette
- [ ] Erste Built-in Plugins:
  - [ ] GitHub Integration
  - [ ] Auto-Unsubscribe

### Phase 7 — Search & Polish (2-3 Wochen)

- [ ] FTS5 Volltext-Suche
- [ ] Suchsyntax: `from:`, `to:`, `has:attachment`, `is:unread`, `tag:`, `in:`
- [ ] Saved Searches / Smart Folders
- [ ] Contact-Cache & Autocompletion
- [ ] Attachment-Preview
- [ ] Light/Dark Theme Toggle
- [ ] Smooth Transitions & Micro-Animations
- [ ] Accessibility Audit (Screen Reader, Focus Management)

### Phase 8 — AI-Readiness (future)

- [ ] AI-Provider Abstraction (Local LLM / Claude API / OpenAI)
- [ ] Smart Triage: automatische Kategorisierung
- [ ] Thread-Zusammenfassungen
- [ ] Draft-Vorschläge
- [ ] NLP in Command Palette ("snooze until next Monday")

---

## 12. Technische Risiken & Mitigationen

| Risiko | Schwere | Mitigation |
|--------|---------|------------|
| IMAP Komplexität (Server-Unterschiede) | Hoch | Extensive Tests gegen Dovecot, Exchange, Gmail IMAP. Defensive Parsing. |
| HTML Email Rendering | Mittel | Ammonia für Sanitization, Shadow DOM für Isolation, Fallback auf Text. |
| WASM Plugin Performance | Niedrig | Lazy Loading, Plugin-Timeout, Resource Limits in wasmtime. |
| OAuth Token Management | Mittel | Refresh-Token-Flow robust implementieren, Graceful Re-Auth. |
| Thread-Building Accuracy | Mittel | Hybrid-Algorithmus (Message-ID + Subject), User Override möglich. |
| SQLite Performance bei 100k+ Mails | Niedrig | Indices auf alle Query-Paths, WAL Mode, Pagination, Lazy Body Loading. |
| Tauri v2 Mobile noch beta | Mittel | Desktop-first entwickeln, Mobile als Phase 9. |

---

## 13. Testing-Strategie

### Rust Backend

- **Unit Tests**: Jedes Modul hat `#[cfg(test)]` Tests
- **Integration Tests**: IMAP/SMTP gegen lokalen Greenmail/Dovecot Docker Container
- **Property Tests**: `proptest` für Email-Parser und Thread-Builder
- **Benchmark**: `criterion` für Sync-Performance und Search-Latenz

### Vue Frontend

- **Component Tests**: `vitest` + `@vue/test-utils`
- **E2E Tests**: `playwright` für komplette Flows (Login, Compose, Send)
- **Visual Regression**: `chromatic` oder `Percy` für UI-Konsistenz

### Integration

- **Docker Compose**: Greenmail (IMAP/SMTP), Mock OAuth Server
- **Fixture Emails**: Umfangreiche Test-Mailbox mit Edge Cases (HTML-only, Attachments, Threading, Encodings)

---

## 14. Arbeitstitel & Branding

**Satchel** — Email im Flow. Workflow-orientiert, tastaturgetrieben, erweiterbar.

Alternative Namen: MailFlow, Triage, Conductor, Switchboard, Dispatch

---

*Dieses Dokument ist die Living Specification für Satchel. Es wird mit dem Projekt weiterentwickelt.*
