#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

function argValue(name, fallback = null) {
  const index = process.argv.indexOf(name);
  if (index < 0) return fallback;
  return process.argv[index + 1] ?? fallback;
}

const defaultDb = resolve(
  process.env.HOME ?? '',
  'Library/Application Support/de.tbuck.satchel.daily/satchel/satchel.db',
);
const dbPath = argValue('--db', defaultDb);
const messageId = argValue('--message-id');
const shouldCache = process.argv.includes('--cache');
const outputPath = resolve(
  repoRoot,
  argValue('--out', `output/email-render-review/message-${Date.now()}.html`),
);

if (!messageId) {
  console.error('Usage: node scripts/render-email-message.mjs --message-id <id> [--db path] [--out path]');
  process.exit(1);
}

function sqlString(value) {
  return `'${String(value).replace(/'/g, "''")}'`;
}

function query(sql) {
  const raw = execFileSync('sqlite3', ['-json', dbPath, sql], { encoding: 'utf8' }).trim();
  return raw ? JSON.parse(raw) : [];
}

function execute(sql) {
  execFileSync('sqlite3', [dbPath, sql], { encoding: 'utf8' });
}

function escapeHtml(value) {
  return String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function cleanCid(value) {
  return String(value ?? '').trim().replace(/^cid:/i, '').replace(/^<|>$/g, '').trim();
}

function percentEncodeCid(value) {
  return Buffer.from(value)
    .toString('utf8')
    .split('')
    .map((ch) => {
      const code = ch.charCodeAt(0);
      if (/^[A-Za-z0-9._~@-]$/.test(ch)) return ch;
      return `%${code.toString(16).toUpperCase().padStart(2, '0')}`;
    })
    .join('');
}

function candidatesForAttachment(attachment) {
  const values = [];
  for (const raw of [attachment.content_id, attachment.filename]) {
    const cleaned = cleanCid(raw);
    if (!cleaned) continue;
    for (const value of [cleaned, percentEncodeCid(cleaned)]) {
      if (value && !values.includes(value)) values.push(value);
    }
  }
  return values;
}

function replaceAllCaseInsensitive(input, needle, replacement) {
  return input.replace(new RegExp(needle.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi'), replacement);
}

function safeFilename(value) {
  const sanitized = String(value || 'inline-image').replace(/[^A-Za-z0-9._-]/g, '_');
  return sanitized.replace(/^_+|_+$/g, '') || 'inline-image';
}

async function attachmentBytes(attachment, account) {
  if (attachment.cached_path && existsSync(attachment.cached_path)) {
    return readFileSync(attachment.cached_path);
  }
  if (account.provider !== 'gmail') {
    return null;
  }
  const config = JSON.parse(account.config || '{}');
  const token = config.oauth?.access_token;
  const messageRemoteId = attachment.remote_uid || attachment.message_remote_id;
  if (!token || !messageRemoteId || !attachment.remote_part_id) {
    return null;
  }
  const response = await fetch(
    `https://gmail.googleapis.com/gmail/v1/users/me/messages/${messageRemoteId}/attachments/${attachment.remote_part_id}`,
    { headers: { Authorization: `Bearer ${token}` } },
  );
  if (!response.ok) {
    throw new Error(`Gmail attachment fetch failed ${response.status}: ${await response.text()}`);
  }
  const payload = /** @type {{ data?: string }} */ (await response.json());
  const data = String(payload.data || '').replace(/-/g, '+').replace(/_/g, '/');
  const bytes = Buffer.from(data, 'base64');
  if (shouldCache) {
    const cacheDir = join(dirname(dbPath), 'attachments');
    mkdirSync(cacheDir, { recursive: true });
    const cacheKey = createHash('sha256').update(attachment.id).digest('hex').slice(0, 24);
    const cachePath = join(cacheDir, `${cacheKey}-${safeFilename(attachment.filename)}`);
    writeFileSync(cachePath, bytes);
    execute(`
      UPDATE attachments
      SET cached_path = ${sqlString(cachePath)}, size = ${bytes.length}
      WHERE id = ${sqlString(attachment.id)}
    `);
    attachment.cached_path = cachePath;
  }
  return bytes;
}

const [message] = query(`
  SELECT m.id, m.subject, m.from_address, m.from_name, m.date, m.body_text, m.body_html,
         m.remote_id AS message_remote_id, acc.provider, acc.config
  FROM messages m
  JOIN accounts acc ON acc.id = m.account_id
  WHERE m.id = ${sqlString(messageId)}
  LIMIT 1
`);

if (!message) {
  console.error(`Message not found: ${messageId}`);
  process.exit(1);
}

const attachments = query(`
  SELECT a.id, a.filename, a.content_type, a.content_id, a.cached_path,
         COALESCE(a.remote_uid, ${sqlString(message.message_remote_id)}) AS remote_uid,
         ${sqlString(message.message_remote_id)} AS message_remote_id,
         a.remote_part_id
  FROM attachments a
  WHERE a.message_id = ${sqlString(messageId)}
    AND a.is_inline = 1
    AND LOWER(a.content_type) LIKE 'image/%'
  ORDER BY a.filename
`);

let body = message.body_html || `<pre>${escapeHtml(message.body_text || '')}</pre>`;
let embedded = 0;
for (const attachment of attachments) {
  const candidates = candidatesForAttachment(attachment);
  if (!candidates.some((candidate) => body.toLowerCase().includes(`cid:${candidate}`.toLowerCase()))) {
    continue;
  }
  const bytes = await attachmentBytes(attachment, message);
  if (!bytes) {
    continue;
  }
  const dataUrl = `data:${attachment.content_type};base64,${bytes.toString('base64')}`;
  for (const candidate of candidates) {
    body = replaceAllCaseInsensitive(body, `cid:${candidate}`, dataUrl);
  }
  embedded += 1;
}

const html = `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escapeHtml(message.subject)}</title>
  <style>
    body { margin: 0; padding: 32px; background: #0b0f14; color: #e7edf5; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
    header { max-width: 980px; margin: 0 auto 24px; }
    h1 { margin: 0 0 8px; font-size: 28px; line-height: 1.15; }
    .meta { color: #9aa7b8; }
    .mail { max-width: 980px; margin: 0 auto; background: white; color: black; overflow: hidden; }
    .mail :where(*, *::before, *::after) { box-sizing: border-box; }
    .mail img, .mail picture, .mail video, .mail canvas, .mail svg { max-width: 100% !important; height: auto !important; }
    .mail table, .mail tbody, .mail thead, .mail tfoot, .mail tr, .mail td, .mail th, .mail div, .mail p, .mail blockquote, .mail pre { max-width: 100% !important; min-width: 0 !important; }
    .mail table { max-width: 100% !important; table-layout: auto !important; border-spacing: 0; }
    .mail [style*="width" i] { max-width: 100% !important; }
    .mail [style*="min-width" i] { min-width: 0 !important; }
    .mail a[data-btn] { align-items: center !important; display: flex !important; height: auto !important; justify-content: center !important; line-height: 1.35 !important; min-height: 1.35em !important; text-align: center !important; }
    .mail td:has(a[data-btn]), .mail th:has(a[data-btn]) { height: auto !important; }
  </style>
</head>
<body>
  <header>
    <h1>${escapeHtml(message.subject)}</h1>
    <div class="meta">${escapeHtml(message.from_name || message.from_address)} &lt;${escapeHtml(message.from_address)}&gt; · ${escapeHtml(message.date)}</div>
    <div class="meta">${embedded}/${attachments.length} inline images embedded</div>
  </header>
  <main class="mail">${body}</main>
</body>
</html>`;

mkdirSync(dirname(outputPath), { recursive: true });
writeFileSync(outputPath, html);
console.log(pathToFileURL(outputPath).href);
console.log(`embedded_inline_images=${embedded}/${attachments.length}`);
console.log(`cached_inline_images=${shouldCache ? 'yes' : 'no'}`);
console.log(`output=${outputPath}`);
