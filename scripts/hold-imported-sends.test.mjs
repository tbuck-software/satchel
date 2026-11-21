import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { devNull, tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { advanceImportedCutoff, holdImportedSends } from './hold-imported-sends.mjs';

const importScript = readFileSync(new URL('./import-daily-data.mjs', import.meta.url), 'utf8');

/** @param {(path: string, sql: (input: string) => string) => void} test */
function withSnapshot(test) {
  const folder = mkdtempSync(join(tmpdir(), 'satchel-import-test-'));
  const path = join(folder, 'satchel.db');
  /** @param {string} input */
  const sql = (input) => execFileSync('sqlite3', ['-batch', '-bail', '-init', devNull, path], { input, encoding: 'utf8' }).trim();
  try {
    test(path, sql);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
}

describe('Stable to Daily send holding', () => {
  it('holds only the copied queue before switching either the Daily key or database', () => {
    const snapshot = importScript.indexOf('sqliteLiteral(tempDb)');
    const hold = importScript.indexOf('holdImportedSends(tempDb)');
    expect(hold).toBeGreaterThan(snapshot);
    expect(hold).toBeLessThan(importScript.indexOf('copyProfileMasterKey(stableRoot, dailyRoot)'));
    const activation = importScript.indexOf('advanceImportedCutoff(tempDb)');
    expect(activation).toBeGreaterThan(importScript.indexOf('copyProfileMasterKey(stableRoot, dailyRoot)'));
    expect(activation).toBeLessThan(importScript.indexOf('renameSync(tempDb, dailyDb)'));
  });

  it('holds direct sends, forwards and merges without deleting draft data', () => withSnapshot((path, sql) => {
    sql(`
      CREATE TABLE sync_queue (id PRIMARY KEY, action_type, payload, status, last_error, submission_started_at);
      INSERT INTO sync_queue VALUES
        ('1', 'send_message', 'queued draft', 'scheduled', NULL, NULL),
        ('2', 'send_message', 'copied draft', 'sending', NULL, 'started'),
        ('3', 'rule_forward', 'forward source', 'pending', NULL, NULL),
        ('4', 'send_message', 'older draft', 'failed', 'network offline', NULL),
        ('5', 'sync_draft', 'upload draft', 'pending', NULL, NULL),
        ('6', 'remote_mutation', '{"kind":"append_sent","raw_base64":"bWFpbA=="}', 'sending', NULL, NULL),
        ('7', 'remote_mutation', '{"kind":"flags","refs":[{"remote_id":"1","source_folder":"INBOX","kind":"append_sent"}],"read":true}', 'pending', NULL, NULL),
        ('8', 'remote_mutation', 'malformed append_sent payload', 'failed', 'old error', NULL);
      CREATE TABLE messages (is_draft, scheduled_for, body_text);
      INSERT INTO messages VALUES (1, '2099-01-01', 'draft text');
      CREATE TABLE mail_merge_jobs (status, pause_reason);
      INSERT INTO mail_merge_jobs VALUES ('running', NULL), ('completed', NULL);
      CREATE TABLE ingest_effects (run_rules);
      INSERT INTO ingest_effects VALUES (1);
      CREATE TABLE notification_outbox (message_id, state);
      INSERT INTO notification_outbox VALUES
        ('unread', 'pending'), ('claimed', 'dispatching'),
        ('shown', 'delivered'), ('muted', 'dismissed');
    `);

    holdImportedSends(path);

    expect(sql('SELECT status, payload FROM sync_queue ORDER BY id;')).toBe([
      'failed|queued draft',
      'delivery_unknown|copied draft',
      'failed|forward source',
      'failed|older draft',
      'pending|upload draft',
      'failed|{"kind":"append_sent","raw_base64":"bWFpbA=="}',
      'pending|{"kind":"flags","refs":[{"remote_id":"1","source_folder":"INBOX","kind":"append_sent"}],"read":true}',
      'failed|malformed append_sent payload',
    ].join('\n'));
    expect(sql("SELECT last_error FROM sync_queue WHERE id = '4';")).toBe('backend.errors.outgoingSendHeldAfterRestore');
    expect(sql("SELECT last_error FROM sync_queue WHERE id = '6';")).toBe('backend.errors.sentCopyHeldAfterRestore');
    expect(sql("SELECT last_error FROM sync_queue WHERE id = '8';")).toBe('old error');
    expect(sql('SELECT queue_id, error FROM restored_queue_failures;')).toBe('4|network offline');
    expect(sql('SELECT scheduled_for, body_text FROM messages;')).toBe('|draft text');
    expect(sql('SELECT status, pause_reason FROM mail_merge_jobs;')).toBe('paused|restart\ncompleted|');
    expect(sql('SELECT run_rules FROM ingest_effects;')).toBe('0');
    expect(sql('SELECT message_id FROM notification_outbox ORDER BY message_id;')).toBe('muted\nshown');
  }));

  it('advances the copied incoming cutoff without replacing a later one or changing sync history', () => withSnapshot((path, sql) => {
    sql(`
      CREATE TABLE sync_queue (id PRIMARY KEY, action_type, payload, status, last_error);
      CREATE TABLE app_settings (key PRIMARY KEY, value, updated_at);
      CREATE TABLE accounts (id, created_at, last_sync_at, incoming_effects_cutoff);
      INSERT INTO accounts VALUES
        ('old', '2025-01-01', '2026-01-01', '2026-01-02T00:00:00Z'),
        ('future', '2025-01-01', '2026-01-01', '2099-01-01T01:00:00+01:00');
    `);
    advanceImportedCutoff(path);
    expect(sql("SELECT julianday(incoming_effects_cutoff) > julianday('2026-01-02') FROM accounts WHERE id = 'old';")).toBe('1');
    expect(sql("SELECT incoming_effects_cutoff FROM accounts WHERE id = 'future';")).toBe('2099-01-01T01:00:00+01:00');
    expect(sql('SELECT DISTINCT last_sync_at FROM accounts;')).toBe('2026-01-01');
    expect(sql("SELECT value FROM app_settings WHERE key = 'daily_import.awaiting_first_open';")).toBe('1');
  }));

  it('accepts older snapshots without merge tables or submission markers', () => withSnapshot((path, sql) => {
    sql(`
      CREATE TABLE sync_queue (id PRIMARY KEY, action_type, payload, status, last_error);
      INSERT INTO sync_queue VALUES
        ('1', 'send_message', 'old draft', 'sending', NULL),
        ('2', 'remote_mutation', '{"kind":"append_sent","raw_base64":"bWFpbA=="}', 'pending', NULL);
      CREATE TABLE app_settings (key PRIMARY KEY, value, updated_at);
      CREATE TABLE accounts (id, created_at, last_sync_at);
      INSERT INTO accounts VALUES
        ('old', '2025-01-01', '2026-01-01'),
        ('future', '2025-01-01', '2099-01-01T01:00:00+01:00');
    `);
    holdImportedSends(path);
    expect(sql('SELECT status, payload FROM sync_queue ORDER BY status;')).toBe(
      'delivery_unknown|old draft\nfailed|{"kind":"append_sent","raw_base64":"bWFpbA=="}',
    );
    advanceImportedCutoff(path);
    const cutoff = sql("SELECT value FROM app_settings WHERE key = 'restore.incoming_effects_cutoff';");
    expect(cutoff).toMatch(/T.*Z$/);
    sql('ALTER TABLE accounts ADD COLUMN incoming_effects_cutoff TEXT;');
    sql(readFileSync(new URL('../crates/satchel-core/src/backup/apply_incoming_cutoff.sql', import.meta.url), 'utf8'));
    expect(sql("SELECT incoming_effects_cutoff FROM accounts WHERE id = 'old';")).toBe(cutoff);
    expect(sql("SELECT incoming_effects_cutoff FROM accounts WHERE id = 'future';")).toBe('2099-01-01T01:00:00+01:00');
    expect(sql("SELECT value FROM app_settings WHERE key = 'restore.incoming_effects_cutoff';")).toBe('');
  }));
});
