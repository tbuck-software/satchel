#!/usr/bin/env node
import { execFileSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { chromium } from '@playwright/test';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const defaultDb = resolve(
  process.env.HOME ?? '',
  'Library/Application Support/de.tbuck.satchel.daily/satchel/satchel.db',
);

function argValue(name, fallback = null) {
  const index = process.argv.indexOf(name);
  if (index < 0) return fallback;
  return process.argv[index + 1] ?? fallback;
}

const dbPath = resolve(argValue('--db', defaultDb));
const outDir = resolve(repoRoot, argValue('--out', 'output/email-render-fidelity'));
const allowRemoteImages = process.argv.includes('--allow-remote-images');
const cacheInlineImages = process.argv.includes('--cache-inline-images');
const strictMissing = process.argv.includes('--strict-missing');
const acceptBaseline = process.argv.includes('--accept-baseline') || process.argv.includes('--update-baseline');
const vitePort = Number(argValue('--port', '5187'));
const viteUrl = `http://127.0.0.1:${vitePort}/render-fidelity.html`;

const cases = [
  {
    key: 'google-business-profile',
    label: 'Google Business Profile',
    where: "LOWER(m.from_address) LIKE '%businessprofile-noreply@google.com%' OR m.subject LIKE 'Leistungsbericht%'",
  },
  {
    key: 'dhl',
    label: 'DHL',
    where: "LOWER(m.from_address) LIKE '%@dhl.de%' OR m.subject LIKE '%Zustell%' OR m.subject LIKE '%Sendung%'",
  },
  {
    key: 'agency',
    label: 'agency',
    where: "LOWER(m.from_address) LIKE '%agency.example%' OR LOWER(m.subject) LIKE '%agency%'",
  },
  {
    key: 'nested-quoted-history',
    label: 'Nested Outlook/Word quoted history',
    where: `LOWER(m.from_address) LIKE '%agency.example%'
      AND LOWER(m.subject) LIKE '%meeting ig%'
      AND m.body_html LIKE '%messageReplySection%'
      AND m.body_html LIKE '%border-left%'
      AND m.body_html LIKE '%blockquote%'`,
    normalizeQuotes: true,
  },
  {
    key: 'proton-quoted-history',
    label: 'Proton quoted HTML history',
    where: `LOWER(m.from_address) LIKE '%@proton.me%'
      AND m.body_html LIKE '%protonmail_quote%'
      AND m.body_html LIKE '%href=%'`,
    normalizeQuotes: true,
  },
  {
    key: 'github',
    label: 'GitHub',
    where: "LOWER(m.from_address) LIKE '%github%' OR LOWER(m.subject) LIKE '%github%' OR m.subject LIKE '[example-owner/%'",
  },
  {
    key: 'gitlab',
    label: 'GitLab',
    where: "LOWER(m.from_address) LIKE '%gitlab%' OR LOWER(m.subject) LIKE '%gitlab%'",
  },
  {
    key: 'linkedin',
    label: 'LinkedIn',
    where: "LOWER(m.from_address) LIKE '%linkedin%' OR LOWER(m.subject) LIKE '%linkedin%'",
  },
  {
    key: 'calendar-invite',
    label: 'Calendar invite',
    where: `EXISTS (
      SELECT 1 FROM attachments a
      WHERE a.message_id = m.id
        AND (LOWER(a.content_type) LIKE '%text/calendar%' OR LOWER(a.filename) LIKE '%.ics')
    )`,
  },
  {
    key: 'hotel',
    label: 'info@hotel.example',
    where: "LOWER(m.from_address) LIKE '%info@hotel.example%' OR LOWER(m.to_addresses) LIKE '%info@hotel.example%'",
  },
  {
    key: 'google-general',
    label: 'Google general',
    where: "LOWER(m.from_address) LIKE '%google%' AND LOWER(m.from_address) NOT LIKE '%businessprofile-noreply@google.com%'",
  },
];

function sqlString(value) {
  return `'${String(value).replace(/'/g, "''")}'`;
}

function query(sql) {
  const raw = execFileSync('sqlite3', ['-json', dbPath, sql], {
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  }).trim();
  return raw ? JSON.parse(raw) : [];
}

function execute(sql) {
  execFileSync('sqlite3', [dbPath, sql], { encoding: 'utf8' });
}

async function waitForHttp(url, timeoutMs = 30_000) {
  const startedAt = Date.now();
  let lastError = null;
  while (Date.now() - startedAt < timeoutMs) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
      lastError = new Error(`HTTP ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`timed out waiting for ${url}: ${lastError?.message || lastError || 'no response'}`);
}

async function startViteServer() {
  const command = process.platform === 'win32' ? 'npx.cmd' : 'npx';
  const child = spawn(
    command,
    ['vite', '--host', '127.0.0.1', '--port', String(vitePort), '--strictPort'],
    {
      cwd: repoRoot,
      env: { ...process.env, SATCHEL_RENDER_FIDELITY: '1' },
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );
  let logs = '';
  child.stdout.on('data', (chunk) => {
    logs += chunk.toString();
  });
  child.stderr.on('data', (chunk) => {
    logs += chunk.toString();
  });
  try {
    await waitForHttp(viteUrl);
  } catch (error) {
    child.kill('SIGTERM');
    throw new Error(`${error instanceof Error ? error.message : String(error)}\n${logs}`);
  }
  return child;
}

async function stopProcess(child) {
  if (!child || child.killed) {
    return;
  }
  child.kill('SIGTERM');
  await new Promise((resolve) => {
    const timeout = setTimeout(resolve, 2_000);
    child.once('exit', () => {
      clearTimeout(timeout);
      resolve();
    });
  });
}

function escapeHtml(value) {
  return String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function safeFilename(value) {
  const sanitized = String(value || 'message')
    .replace(/[^A-Za-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return sanitized || 'message';
}

function cleanCid(value) {
  const stripped = String(value ?? '').trim().replace(/^cid:/i, '').replace(/^<|>$/g, '').trim();
  try {
    return decodeURIComponent(stripped);
  } catch {
    return stripped;
  }
}

function cidCandidates(attachment) {
  const values = new Set();
  for (const raw of [attachment.content_id, attachment.filename]) {
    const cleaned = cleanCid(raw);
    if (!cleaned) continue;
    values.add(cleaned);
    values.add(`<${cleaned}>`);
    values.add(encodeURIComponent(cleaned));
  }
  return Array.from(values).filter(Boolean);
}

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function base64UrlToBuffer(value) {
  const normalized = String(value || '').replace(/-/g, '+').replace(/_/g, '/');
  return Buffer.from(normalized, 'base64');
}

async function fetchAndCacheInlineImage(attachment, message) {
  if (!cacheInlineImages || attachment.cached_path) {
    return attachment;
  }
  if (message.provider !== 'gmail') {
    return attachment;
  }
  const config = JSON.parse(message.config || '{}');
  const token = config.oauth?.access_token;
  const messageRemoteId = attachment.remote_uid || message.message_remote_id;
  if (!token || !messageRemoteId || !attachment.remote_part_id) {
    return attachment;
  }

  const response = await fetch(
    `https://gmail.googleapis.com/gmail/v1/users/me/messages/${encodeURIComponent(messageRemoteId)}/attachments/${encodeURIComponent(attachment.remote_part_id)}`,
    { headers: { Authorization: `Bearer ${token}` } },
  );
  if (!response.ok) {
    throw new Error(`failed to fetch inline Gmail image for ${message.subject}: HTTP ${response.status}`);
  }
  const payload = /** @type {{ data: string }} */ (await response.json());
  const bytes = base64UrlToBuffer(payload.data);
  const cacheDir = resolve(dirname(dbPath), 'attachments');
  mkdirSync(cacheDir, { recursive: true });
  const cacheKey = createHash('sha256').update(attachment.id).digest('hex').slice(0, 24);
  const cachePath = resolve(cacheDir, `${cacheKey}-${safeFilename(attachment.filename)}`);
  writeFileSync(cachePath, bytes);
  execute(`
    UPDATE attachments
    SET cached_path = ${sqlString(cachePath)}, size = ${bytes.length}
    WHERE id = ${sqlString(attachment.id)}
  `);
  return { ...attachment, cached_path: cachePath, size: bytes.length };
}

async function prepareAttachmentsForRender(message, attachments) {
  const prepared = [];
  for (const attachment of attachments) {
    prepared.push(await fetchAndCacheInlineImage(attachment, message));
  }
  return prepared;
}

function dataUrlForAttachment(attachment) {
  if (!attachment.cached_path || !existsSync(attachment.cached_path)) {
    return null;
  }
  const bytes = readFileSync(attachment.cached_path);
  return `data:${attachment.content_type};base64,${bytes.toString('base64')}`;
}

function rendererAttachment(attachment) {
  const dataUrl = dataUrlForAttachment(attachment);
  return {
    id: attachment.id,
    filename: attachment.filename || 'attachment',
    contentType: attachment.content_type || 'application/octet-stream',
    size: attachment.size ?? 0,
    isInline: Boolean(attachment.is_inline),
    contentId: attachment.content_id ?? null,
    cachedPath: attachment.cached_path ?? null,
    dataUrl,
  };
}

function inlineImageCacheStats(attachments) {
  let cachedInlineImages = 0;
  let uncachedInlineImages = 0;
  for (const attachment of attachments) {
    if (!String(attachment.content_type || '').toLowerCase().startsWith('image/')) {
      continue;
    }
    if (!attachment.cached_path || !existsSync(attachment.cached_path)) {
      uncachedInlineImages += 1;
      continue;
    }
    cachedInlineImages += 1;
  }
  return { cachedInlineImages, uncachedInlineImages };
}

function inlineCachedCidImages(html, attachments) {
  let output = html;
  for (const attachment of attachments) {
    const dataUrl = dataUrlForAttachment(attachment);
    if (!dataUrl) {
      continue;
    }
    for (const candidate of cidCandidates(attachment)) {
      output = output.replace(new RegExp(`cid:${escapeRegex(candidate)}`, 'gi'), dataUrl);
    }
  }
  return output;
}

function buildRawReferenceDocument(payload) {
  const remoteSources = allowRemoteImages ? ' http: https:' : '';
  const body = payload.bodyHtml
    ? inlineCachedCidImages(payload.bodyHtml, payload.attachments)
    : `<pre>${escapeHtml(payload.bodyText)}</pre>`;
  return `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data: cid: blob:${remoteSources}; style-src 'unsafe-inline'; font-src data:${remoteSources}; media-src data: blob:${remoteSources}">
  <base target="_blank">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escapeHtml(payload.subject)}</title>
  <style>
    :where(*, *::before, *::after) { box-sizing: border-box; }
    html, body { margin: 0; padding: 0; width: 100%; overflow-x: hidden; }
    img, picture, video, canvas, svg { max-width: 100% !important; height: auto !important; }
    table, div, p, blockquote, pre { max-width: 100% !important; }
    pre { white-space: pre-wrap; overflow-wrap: break-word; }
  </style>
</head>
<body>${body}</body>
</html>`;
}

function selectMessage(testCase) {
  const rows = query(`
    SELECT m.id, m.thread_id, m.subject, m.from_address, COALESCE(m.from_name, '') AS from_name,
           m.date, COALESCE(m.body_text, '') AS body_text, COALESCE(m.body_html, '') AS body_html,
           m.remote_id AS message_remote_id, a.provider, a.config
    FROM messages m
    JOIN accounts a ON a.id = m.account_id
    WHERE (${testCase.where})
      AND (TRIM(COALESCE(m.body_html, '')) != '' OR TRIM(COALESCE(m.body_text, '')) != '')
    ORDER BY datetime(m.date) DESC, m.id DESC
    LIMIT 1
  `);
  return rows[0] ?? null;
}

function attachmentsFor(messageId) {
  return query(`
    SELECT id, filename, content_type, content_id, cached_path, is_inline
           , remote_uid, remote_part_id
    FROM attachments
    WHERE message_id = ${sqlString(messageId)}
      AND is_inline = 1
    ORDER BY filename
  `);
}

async function payloadForMessage(testCase, message, attachments) {
  const preparedAttachments = await prepareAttachmentsForRender(message, attachments);
  const stats = inlineImageCacheStats(preparedAttachments);
  const meta = [
    `${stats.cachedInlineImages}/${attachments.length} cached inline attachment(s)`,
    `${stats.uncachedInlineImages} uncached inline attachment(s)`,
    cacheInlineImages ? 'inline cache enabled' : 'inline cache disabled',
    'remote images handled by EmailRenderer image policy',
  ];
  return {
    payload: {
      id: message.id,
      label: testCase.label,
      subject: message.subject,
      fromAddress: message.from_address,
      fromName: message.from_name,
      date: message.date,
      bodyHtml: message.body_html.trim() ? message.body_html : undefined,
      bodyText: message.body_text,
      normalizeQuotes: Boolean(testCase.normalizeQuotes),
      attachments: preparedAttachments.map(rendererAttachment),
      meta,
    },
    cachedInlineImages: stats.cachedInlineImages,
    uncachedInlineImages: stats.uncachedInlineImages,
  };
}

async function inspectCase(browser, item) {
  const page = await browser.newPage({ viewport: { width: 1160, height: 1200 }, deviceScaleFactor: 1 });
  if (allowRemoteImages && item.payload.fromAddress) {
    await page.addInitScript((senderEmail) => {
      localStorage.setItem('satchel:images-allowed-senders', JSON.stringify([senderEmail.toLowerCase()]));
    }, item.payload.fromAddress);
  }
  await page.goto(viteUrl);
  await page.waitForFunction(() => typeof window.__setSatchelRenderFidelityPayload === 'function');
  await page.evaluate(async (payload) => {
    await window.__setSatchelRenderFidelityPayload(payload);
  }, item.payload);
  await page.waitForFunction(() => document.documentElement.dataset.renderReady === 'true');
  await page.waitForTimeout(250);

  if (item.payload.normalizeQuotes) {
    const quoteSummary = page.locator('summary.quote-summary').first();
    if (await quoteSummary.count() === 0) {
      await page.close();
      return { issues: ['quoted-history normalization produced no expandable history'], metrics: {} };
    }
    await quoteSummary.click();
    await page.waitForTimeout(250);
  }

  const inspection = await page.evaluate(() => {
    function parseRgb(input) {
      const match = input.match(/rgba?\(([^)]+)\)/i);
      if (!match) return null;
      const parts = match[1].split(/[\s,/]+/).filter(Boolean).map(Number);
      if (parts.length < 3 || parts.slice(0, 3).some(Number.isNaN)) return null;
      return { r: parts[0], g: parts[1], b: parts[2], a: Number.isFinite(parts[3]) ? parts[3] : 1 };
    }
    function luminance(color) {
      const values = [color.r, color.g, color.b].map((channel) => {
        const value = channel / 255;
        return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * values[0] + 0.7152 * values[1] + 0.0722 * values[2];
    }
    function contrast(first, second) {
      const light = Math.max(luminance(first), luminance(second));
      const dark = Math.min(luminance(first), luminance(second));
      return (light + 0.05) / (dark + 0.05);
    }
    function backgroundFor(element) {
      let current = element;
      while (current) {
        const bg = parseRgb(getComputedStyle(current).backgroundColor);
        if (bg && bg.a > 0.02) return bg;
        current = current.parentElement;
      }
      return { r: 255, g: 255, b: 255, a: 1 };
    }
    function isVisible(element) {
      const style = getComputedStyle(element);
      if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) return false;
      if (Number.parseFloat(style.fontSize) <= 1 || Number.parseFloat(style.lineHeight) <= 1) return false;
      const rect = element.getBoundingClientRect();
      return rect.width > 1 && rect.height > 1;
    }
    const emailDocuments = Array.from(document.querySelectorAll('iframe.satchel-email-document'))
      .map((frame) => /** @type {HTMLIFrameElement} */ (frame).contentDocument)
      .filter(Boolean);
    const text = [
      document.body.innerText || '',
      ...emailDocuments.map(
        (doc) => /** @type {HTMLElement | null} */ (doc.querySelector('.satchel-email-body'))?.innerText || '',
      ),
    ].join('\n');
    const images = [
      ...Array.from(document.images),
      ...emailDocuments.flatMap((doc) => Array.from(doc.querySelectorAll('img'))),
    ];
    const unresolvedCidImages = images
      .filter((image) => /^cid:/i.test(image.getAttribute('src') || ''))
      .map((image) => image.getAttribute('src'));
    const brokenDataImages = images
      .filter((image) => (image.currentSrc || image.src || '').startsWith('data:') && image.complete && image.naturalWidth === 0)
      .map((image) => image.getAttribute('alt') || image.getAttribute('src')?.slice(0, 80) || 'data image');
    const missingInlineImagePlaceholders = images
      .filter((image) => (image.getAttribute('src') || '').includes('Inline%20image%20not%20available'))
      .length;
    const lowContrastText = [];
    // Keep contrast checks on the app chrome. Many marketing mails contain
    // intentionally hidden responsive variants that report layout boxes in
    // headless Chromium and otherwise create false positives here.
    for (const root of [document.body]) {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      while (walker.nextNode() && lowContrastText.length < 12) {
        const node = walker.currentNode;
        const sample = node.nodeValue?.replace(/\s+/g, ' ').trim() || '';
        if (sample.length < 2) continue;
        const element = node.parentElement;
        if (!element || !isVisible(element) || element.closest('[aria-hidden="true"]')) continue;
        const color = parseRgb(getComputedStyle(element).color);
        const bg = backgroundFor(element);
        if (!color || color.a < 0.5) continue;
        const ratio = contrast(color, bg);
        if (ratio < 2.2) {
          lowContrastText.push({ sample: sample.slice(0, 80), ratio: Number(ratio.toFixed(2)) });
        }
      }
    }
    return {
      text,
      textLength: text.trim().length,
      scrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
      unresolvedCidImages,
      brokenDataImages,
      missingInlineImagePlaceholders,
      lowContrastText,
      imageCount: images.length,
      emailDocumentCount: emailDocuments.length,
      emailHorizontalOverflow: emailDocuments.some((doc) => (
        doc.documentElement.scrollWidth > doc.documentElement.clientWidth + 4
      )),
    };
  });
  const actualDir = resolve(outDir, 'actual');
  mkdirSync(actualDir, { recursive: true });
  const screenshotPath = resolve(actualDir, `${item.key}.png`);
  await page.screenshot({ path: screenshotPath, fullPage: true });
  await page.close();

  const issues = [];
  if (item.payload.bodyHtml && inspection.emailDocumentCount === 0) {
    issues.push('HTML body did not produce an isolated mail document');
  }
  const text = inspection.text || '';
  const badMarkers = [
    { name: 'mojibake C3 marker', value: '\u00c3' },
    { name: 'mojibake C2 marker', value: '\u00c2' },
    { name: 'mojibake e2 marker', value: '\u00e2\u20ac' },
    { name: 'replacement character', value: '\ufffd' },
  ];
  for (const marker of badMarkers) {
    if (text.includes(marker.value)) {
      issues.push(`visible ${marker.name}`);
    }
  }
  if (/&(amp|lt|gt|quot|apos|nbsp|#[0-9]+|#x[0-9a-f]+);/i.test(text)) {
    issues.push('visible html entity');
  }
  if (/(?:=[0-9A-F]{2}){2,}|=\r?\n/i.test(text)) {
    issues.push('visible quoted-printable residue');
  }
  if (/Loading full message/i.test(text)) {
    issues.push('visible loading placeholder');
  }
  if (inspection.textLength < 20) {
    issues.push('rendered text is nearly blank');
  }
  if (inspection.scrollWidth > inspection.clientWidth + 4) {
    issues.push(`horizontal overflow ${inspection.scrollWidth}px > ${inspection.clientWidth}px`);
  }
  if (inspection.emailHorizontalOverflow) {
    issues.push('horizontal overflow inside isolated mail document');
  }
  if (inspection.unresolvedCidImages.length > 0) {
    issues.push(`${inspection.unresolvedCidImages.length} unresolved cid image(s)`);
  }
  if (inspection.brokenDataImages.length > 0) {
    issues.push(`${inspection.brokenDataImages.length} broken embedded data image(s)`);
  }
  if (inspection.missingInlineImagePlaceholders > 0) {
    issues.push(`${inspection.missingInlineImagePlaceholders} missing inline image placeholder(s)`);
  }
  if (inspection.lowContrastText.length > 0) {
    issues.push(`${inspection.lowContrastText.length} low-contrast visible text sample(s)`);
  }

  return {
    issues,
    metrics: {
      textLength: inspection.textLength,
      imageCount: inspection.imageCount,
      scrollWidth: inspection.scrollWidth,
      clientWidth: inspection.clientWidth,
      lowContrastText: inspection.lowContrastText,
      screenshotPath,
    },
  };
}

async function screenshotRawReference(browser, item) {
  const page = await browser.newPage({ viewport: { width: 1160, height: 1200 }, deviceScaleFactor: 1 });
  await page.setContent(buildRawReferenceDocument(item.payload), { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(250);
  const referenceDir = resolve(outDir, 'reference');
  mkdirSync(referenceDir, { recursive: true });
  const referencePath = resolve(referenceDir, `${item.key}.png`);
  await page.screenshot({ path: referencePath, fullPage: true });
  await page.close();
  return referencePath;
}

function relativeUrl(fromDir, targetPath) {
  return relative(fromDir, targetPath).split('/').map(encodeURIComponent).join('/');
}

function ensureBaseline(result) {
  const baselineDir = resolve(outDir, 'baseline');
  mkdirSync(baselineDir, { recursive: true });
  const baselinePath = resolve(baselineDir, `${result.key}.png`);
  let accepted = false;
  if (acceptBaseline) {
    copyFileSync(result.screenshot, baselinePath);
    accepted = true;
  }
  return {
    baselinePath: existsSync(baselinePath) ? baselinePath : null,
    baselineAccepted: accepted,
  };
}

function writeIndex(report, indexPath) {
  const rows = report.results.map((result) => {
    const status = result.issues.length === 0 ? 'PASS' : 'FAIL';
    const issueList = result.issues.length
      ? `<ul>${result.issues.map((issue) => `<li>${escapeHtml(issue)}</li>`).join('')}</ul>`
      : '<span class="muted">No issues detected</span>';
    const baselineFigure = result.baseline
      ? `<figure>
            <figcaption>Approved baseline${result.baseline_accepted ? ' (accepted this run)' : ''}</figcaption>
            <a href="${relativeUrl(dirname(indexPath), result.baseline)}"><img src="${relativeUrl(dirname(indexPath), result.baseline)}" alt="${escapeHtml(result.label)} approved baseline"></a>
          </figure>`
      : `<figure class="empty">
            <figcaption>No approved baseline</figcaption>
            <div>Review raw reference vs. current app render first. Accept intentionally with <code>pnpm run render:fidelity -- --accept-baseline</code>.</div>
          </figure>`;
    return `
      <article class="case ${status.toLowerCase()}">
        <header>
          <div>
            <span class="status">${status}</span>
            <h2>${escapeHtml(result.label)}</h2>
            <p>${escapeHtml(result.subject)}</p>
          </div>
          <a href="${relativeUrl(dirname(indexPath), result.payload)}">payload</a>
        </header>
        <div class="shots">
          <figure>
            <figcaption>Raw browser reference</figcaption>
            <a href="${relativeUrl(dirname(indexPath), result.reference)}"><img src="${relativeUrl(dirname(indexPath), result.reference)}" alt="${escapeHtml(result.label)} raw reference"></a>
          </figure>
          <figure>
            <figcaption>Current app render</figcaption>
            <a href="${relativeUrl(dirname(indexPath), result.screenshot)}"><img src="${relativeUrl(dirname(indexPath), result.screenshot)}" alt="${escapeHtml(result.label)} current"></a>
          </figure>
          ${baselineFigure}
        </div>
        <section class="issues">${issueList}</section>
      </article>`;
  }).join('\n');
  const missing = report.missing_cases.length
    ? `<section class="missing"><h2>Missing cases</h2><ul>${report.missing_cases.map((item) => `<li>${escapeHtml(item.label)}</li>`).join('')}</ul></section>`
    : '';

  writeFileSync(indexPath, `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Satchel Render Fidelity</title>
  <style>
    body { margin: 0; padding: 28px; background: #0b0f14; color: #e7edf5; font: 14px -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
    h1 { margin: 0 0 8px; font-size: 28px; }
    h2 { margin: 0; font-size: 18px; }
    p { margin: 4px 0 0; color: #9aa7b8; }
    a { color: #58a6ff; }
    .summary { margin-bottom: 24px; color: #9aa7b8; display: grid; gap: 4px; }
    .case { border: 1px solid #263241; border-radius: 8px; margin: 0 0 22px; padding: 16px; background: #111821; }
    .case.fail { border-color: #f85149; }
    .case header { display: flex; align-items: start; justify-content: space-between; gap: 16px; margin-bottom: 14px; }
    .status { display: inline-block; margin-bottom: 6px; border-radius: 999px; padding: 3px 8px; background: #16351f; color: #56d364; font-weight: 700; font-size: 12px; }
    .fail .status { background: #3a1618; color: #ff7b72; }
    .shots { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; }
    figure { margin: 0; min-width: 0; }
    figcaption { margin-bottom: 8px; color: #c9d1d9; font-weight: 650; }
    img { display: block; width: 100%; max-height: 760px; object-fit: contain; object-position: top; background: #05080d; border: 1px solid #263241; border-radius: 6px; }
    figure.empty div { min-height: 180px; display: grid; place-content: center; padding: 18px; color: #9aa7b8; text-align: center; border: 1px dashed #3b4756; border-radius: 6px; background: #0b1118; }
    .issues { margin-top: 12px; color: #c9d1d9; }
    .issues ul { margin: 0; padding-left: 18px; color: #ffb3ad; }
    .muted { color: #7d8590; }
    .missing { margin-top: 26px; padding: 16px; border: 1px solid #7c5b16; background: #1f1a0d; border-radius: 8px; }
    @media (max-width: 900px) { .shots { grid-template-columns: 1fr; } }
  </style>
</head>
<body>
  <h1>Satchel Render Fidelity</h1>
  <section class="summary">
    <span>Generated: ${escapeHtml(report.generated_at)}</span>
    <span>Renderer: ${escapeHtml(report.renderer)}</span>
    <span>Database: ${escapeHtml(report.db)}</span>
    <span>Open this page after running <code>make render-fidelity</code>. Raw reference is the HTML/MIME-near browser render; current is the latest app render. Approved baselines are only created with <code>--accept-baseline</code>.</span>
  </section>
  ${rows}
  ${missing}
</body>
</html>`);
}

async function main() {
  if (!existsSync(dbPath)) {
    throw new Error(`database not found: ${dbPath}`);
  }
  mkdirSync(outDir, { recursive: true });

  const selected = [];
  const missing = [];
  const payloadDir = resolve(outDir, 'payloads');
  mkdirSync(payloadDir, { recursive: true });
  for (const testCase of cases) {
    const message = selectMessage(testCase);
    if (!message) {
      missing.push(testCase);
      continue;
    }
    const attachments = attachmentsFor(message.id);
    const rendered = await payloadForMessage(testCase, message, attachments);
    const hash = createHash('sha256').update(message.id).digest('hex').slice(0, 10);
    const key = `${testCase.key}-${hash}`;
    const payloadPath = resolve(payloadDir, `${key}.json`);
    writeFileSync(payloadPath, `${JSON.stringify(rendered.payload, null, 2)}\n`);
    selected.push({
      ...testCase,
      key,
      message,
      payload: rendered.payload,
      payloadPath,
      attachmentCount: attachments.length,
      cachedInlineImages: rendered.cachedInlineImages,
      uncachedInlineImages: rendered.uncachedInlineImages,
    });
  }

  let viteServer = null;
  const browser = await chromium.launch({ headless: true });
  const results = [];
  try {
    viteServer = await startViteServer();
    for (const item of selected) {
      const inspected = await inspectCase(browser, item);
      const referencePath = await screenshotRawReference(browser, item);
      const result = {
        key: item.key,
        label: item.label,
        message_id: item.message.id,
        subject: item.message.subject,
        from: item.message.from_address,
        date: item.message.date,
        app_url: viteUrl,
        payload: item.payloadPath,
        reference: referencePath,
        screenshot: inspected.metrics.screenshotPath,
        attachments: item.attachmentCount,
        cached_inline_images: item.cachedInlineImages,
        uncached_inline_images: item.uncachedInlineImages,
        issues: inspected.issues,
        metrics: inspected.metrics,
      };
      const baseline = ensureBaseline(result);
      results.push({
        ...result,
        baseline: baseline.baselinePath,
        baseline_accepted: baseline.baselineAccepted,
      });
    }
  } finally {
    await browser.close();
    await stopProcess(viteServer);
  }

  const report = {
    generated_at: new Date().toISOString(),
    db: dbPath,
    renderer: 'Vue EmailRenderer via Vite render-fidelity.html',
    vite_url: viteUrl,
    allow_remote_images_requested: allowRemoteImages,
    cache_inline_images: cacheInlineImages,
    accept_baseline: acceptBaseline,
    strict_missing: strictMissing,
    missing_cases: missing.map((item) => ({ key: item.key, label: item.label })),
    results,
  };
  const reportPath = resolve(outDir, 'report.json');
  const indexPath = resolve(outDir, 'index.html');
  writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  writeIndex(report, indexPath);

  for (const result of results) {
    const status = result.issues.length === 0 ? 'PASS' : 'FAIL';
    console.log(`${status} ${result.label}: ${result.subject}`);
    if (result.issues.length > 0) {
      for (const issue of result.issues) {
        console.log(`  - ${issue}`);
      }
    }
    console.log(`  payload: ${result.payload}`);
    console.log(`  reference: ${result.reference}`);
    console.log(`  screenshot: ${result.screenshot}`);
    console.log(`  baseline: ${result.baseline ? `${result.baseline}${result.baseline_accepted ? ' (accepted)' : ''}` : 'none'}`);
  }
  if (missing.length > 0) {
    console.log(`WARN missing cases: ${missing.map((item) => item.label).join(', ')}`);
  }
  console.log(`report: ${reportPath}`);
  console.log(`index: ${pathToFileURL(indexPath).href}`);

  const failing = results.filter((result) => result.issues.length > 0);
  if (failing.length > 0 || (strictMissing && missing.length > 0)) {
    process.exitCode = 1;
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack || error.message : String(error));
  process.exit(1);
});
