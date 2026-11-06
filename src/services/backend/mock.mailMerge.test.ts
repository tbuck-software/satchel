// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendMergeCsv, BackendMergeJob, BackendMergeJobDetail, BackendMergePreview } from './mailMerge';

/**
 * The mock answers the mail merge commands with the rules of
 * `satchel_core::mail_merge`: detected delimiter and encoding, one row per
 * recipient, placeholders with fallbacks, and skipped rows instead of a
 * stopped job.
 */
describe('mock mail merge', () => {
  beforeEach(() => {
    vi.resetModules();
    window.history.replaceState({}, '', '/?mockExtraThreads=0');
  });

  const encode = (text: string) => btoa(String.fromCharCode(...new TextEncoder().encode(text)));

  it('reads a semicolon list and plans each row', async () => {
    const { sendCommand } = await import('./mock');
    const csv = await sendCommand<BackendMergeCsv>('parse_mail_merge_csv', { input: {
      content_base64: encode('﻿Vorname;E-Mail\nAnna;anna@example.test\nBen;kaputt\n;cem@example.test\n'),
    } });
    expect(csv.delimiter).toBe(';');
    expect(csv.encoding).toBe('utf8_bom');
    expect(csv.email_field).toBe('E-Mail');

    const preview = await sendCommand<BackendMergePreview>('preview_mail_merge', { input: {
      account_id: null,
      template: { subject: 'Hallo {{Vorname}}', body: 'Liebe {{Vorname}}', body_html: null },
      fields: csv.fields,
      email_field: 'E-Mail',
      rows: csv.rows,
      index: 0,
    } });
    expect(preview.plan.ready).toBe(1);
    expect(preview.plan.rows.map((row) => row.issue?.kind ?? null)).toEqual([null, 'invalid_email', 'missing_values']);
    expect(preview.rendered?.subject).toBe('Hallo Anna');
    expect(preview.max_recipients).toBe(500);
  });

  it('starts a job with the ready rows and steers it', async () => {
    const { sendCommand } = await import('./mock');
    const rows = [
      { line: 2, values: { Email: 'a@example.test', Name: 'A' } },
      { line: 3, values: { Email: 'b@example.test', Name: '' } },
      { line: 4, values: { Email: 'c@example.test', Name: 'C' } },
    ];
    const job = await sendCommand<BackendMergeJob>('create_mail_merge_job', { input: {
      account_id: 'acc-1',
      message: { to: [], subject: 'Hi {{Name}}', body: 'x', body_html: null, draft_id: null },
      source_label: 'list.csv',
      delay_seconds: 60,
      fields: ['Email', 'Name'],
      email_field: 'Email',
      rows,
      request_id: 'req-1',
    } });
    expect(job.total).toBe(2);
    // A start sent again after a lost answer finds the job it made.
    const again = await sendCommand<BackendMergeJob>('create_mail_merge_job', { input: {
      account_id: 'acc-1',
      message: { to: [], subject: 'Hi {{Name}}', body: 'x', body_html: null, draft_id: null },
      source_label: 'list.csv', delay_seconds: 60, fields: ['Email', 'Name'], email_field: 'Email', rows,
      request_id: 'req-1',
    } });
    expect(again.id).toBe(job.id);
    expect(job.sent).toBe(1);

    const paused = await sendCommand<BackendMergeJob>('control_mail_merge_job', { id: job.id, action: 'pause' });
    expect(paused.status).toBe('paused');
    // Retry failed copies is not Resume: a paused job stays paused.
    const retried = await sendCommand<BackendMergeJob>('control_mail_merge_job', { id: job.id, action: 'retry_failed' });
    expect(retried.status).toBe('paused');
    const cancelled = await sendCommand<BackendMergeJob>('control_mail_merge_job', { id: job.id, action: 'cancel' });
    expect(cancelled.cancelled).toBe(1);
    const detail = await sendCommand<BackendMergeJobDetail>('get_mail_merge_job', { id: job.id });
    expect(detail.recipients.map((row) => row.status)).toEqual(['sent', 'cancelled']);
    await sendCommand('control_mail_merge_job', { id: job.id, action: 'delete' });
    expect(await sendCommand<BackendMergeJob[]>('list_mail_merge_jobs')).toEqual([]);
  });
});
