<!-- HISTORICAL: execution plan from the early build-out. Kept for context;
     current behaviour is described in README.md and AGENTS.md. -->

# Satchel Implementation Breakdown

> **Status: historisch.** Dieses Dokument beschreibt die Planung der frühen
> Build-out-Phase und ist nicht mehr der Stand des Codes.

This file translates `docs/PLAN.md` into execution-ready work packages.

## 1. End-to-End User Flow

1. Onboarding:
- User opens Satchel and lands in list view.
- Existing account is shown in Settings > Accounts.
- Future step: account wizard writes credentials to keychain and starts initial sync.

2. Triage:
- Inbox list is filtered by workflow states.
- User navigates with `j/k` and opens thread details.
- State transitions happen via keyboard (`t`, `w`, `d`) or action buttons.

3. Deep Work:
- User switches to board view (`b`) for spatial planning.
- Drag/drop updates workflow state and board order.
- WIP limits are visible per column.

4. Resolve:
- Reply from thread view updates activity and can move thread to `waiting`.
- Done items remain visible for audit and can be moved back to `todo`.

5. Search and command ops:
- `/` opens search overlay (`from:`, `in:`, `tag:`, `has:attachment`).
- `:` / `Ctrl+K` opens command palette for quick operations.

## 2. UX and Interaction Rules

- Keyboard-first parity: every core action has a shortcut.
- Two high-frequency workspaces:
  - List view for linear triage.
  - Board view for workflow balancing.
- Focus and modality:
  - `normal`: navigation/commands
  - `insert`: text inputs and search
  - `command`: command palette
- Visual language:
  - Dense but readable cards
  - State-driven color accents
  - Low-latency interactions with minimal animation.

## 3. Technical Task Split

## Package A: Foundation (implemented)
- Vue + Vite + Pinia + Router scaffold
- Tauri + Rust module skeleton
- Base tokens, layout, and global styles
- Core migration SQL

## Package B: Workflow UI (implemented)
- Thread list + detail view
- Workflow state transitions
- Keyboard navigation and command/search overlays
- Board drag/drop with column model

## Package C: Backend Domain Skeleton (implemented)
- IPC commands for core surfaces
- Provider abstraction trait and provider stubs
- Sync queue + watcher + thread assignment helper
- Workflow engine validation and scheduler

## Package D: Next Functional Milestones
- [x] Real IMAP provider + SMTP send for IMAP accounts
- [x] Real database integration for core account/thread/message/workflow commands
- [x] HTML sanitization hardening with a production sanitizer
- [x] Plugin runtime execution with Wasmtime component loading
- [x] Gmail provider sync/send implementation behind OAuth2
- [x] Outlook provider sync/send implementation behind OAuth2
- [x] OAuth browser flow with local callback listener and manual fallback
- [x] Provider follow-up: Gmail/Outlook pagination and backfill parity for multi-page mailbox sync
- [x] Provider follow-up: UI wiring for remote mailbox mutations
- [x] Plugin packaging/build flow for shipping `component.wasm` artifacts

## 4. Acceptance Criteria for Current Iteration

- App starts and shows inbox threads and board columns.
- Keyboard actions update thread state without mouse.
- Search overlay filters thread corpus with operator syntax.
- Rust tree matches target architecture and contains compile-ready stubs.
- Migration includes all core entities from the plan.
