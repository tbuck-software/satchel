import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { devNull } from 'node:os';

/** @param {string} snapshot @param {string} statement */
function sqlite(snapshot, statement) {
  return execFileSync('sqlite3', ['-batch', '-bail', '-init', devNull, '-noheader', '-list', snapshot], {
    input: statement,
    encoding: 'utf8',
  }).trim();
}

/** @param {string} path */
function readSql(path) {
  return readFileSync(new URL(`../crates/satchel-core/src/${path}.sql`, import.meta.url), 'utf8');
}

/** @param {string} snapshot */
export function holdImportedSends(snapshot) {
  const queueColumns = sqlite(snapshot, "SELECT name FROM pragma_table_info('sync_queue');").split('\n');
  let holdSends = readSql('backup/hold_sends');
  if (!queueColumns.includes('submission_started_at')) {
    // Legacy snapshots cannot tell whether a claimed send reached the provider.
    holdSends = holdSends.replaceAll("submission_started_at IS NOT NULL", "status = 'sending'");
  }
  const hasDraftSchedules = sqlite(
    snapshot,
    "SELECT 1 FROM pragma_table_info('messages') WHERE name = 'scheduled_for';",
  );
  const tables = sqlite(
    snapshot,
    "SELECT name FROM sqlite_schema WHERE type = 'table' AND name IN ('mail_merge_jobs', 'ingest_effects', 'notification_outbox');",
  ).split('\n');
  sqlite(snapshot, [
    'PRAGMA foreign_keys = ON;',
    'PRAGMA synchronous = EXTRA;',
    'BEGIN IMMEDIATE;',
    readSql('db/migrations/0198_restore_queue_failures'),
    readSql('backup/preserve_queue_failures'),
    holdSends,
    readSql('backup/hold_sent_copies'),
    hasDraftSchedules ? readSql('backup/clear_draft_schedules') : '',
    tables.includes('mail_merge_jobs') ? readSql('backup/pause_mail_merge') : '',
    tables.includes('ingest_effects') ? readSql('backup/skip_ingest_rules') : '',
    tables.includes('notification_outbox') ? readSql('backup/clear_notification_intents') : '',
    'COMMIT;',
  ].join('\n'));
}

/** Record the import time just before publishing the copy. @param {string} snapshot */
export function advanceImportedCutoff(snapshot) {
  const hasCutoff = sqlite(
    snapshot,
    "SELECT 1 FROM pragma_table_info('accounts') WHERE name = 'incoming_effects_cutoff';",
  );
  sqlite(snapshot, [
    'PRAGMA synchronous = EXTRA;',
    'BEGIN IMMEDIATE;',
    readSql('backup/record_incoming_cutoff').replaceAll('?1', "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"),
    hasCutoff ? readSql('backup/apply_incoming_cutoff') : '',
    "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES ('daily_import.awaiting_first_open', '1', datetime('now'));",
    'COMMIT;',
  ].join('\n'));
}
