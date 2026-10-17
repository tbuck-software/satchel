import type { Attachment } from '../src/types/mail';
import { expect, test, type Page } from './mock-contract.fixture';

const stalledPixelUrl = `http://127.0.0.1:${process.env.SATCHEL_SMOKE_PORT ?? '4178'}/stalled-tracking-pixel.png`;

interface MailPayload {
  id: string;
  label: string;
  subject: string;
  fromAddress: string;
  bodyHtml: string;
  attachments: Attachment[];
  meta: string[];
}

function mailPayload(bodyHtml: string, id = 'responsive-regression'): MailPayload {
  return {
    id,
    label: 'Rendering regression',
    subject: 'Responsive rendering regression',
    fromAddress: 'sender@example.test',
    bodyHtml,
    attachments: [],
    meta: [],
  };
}

async function setMail(page: Page, bodyHtml: string, id?: string) {
  await page.waitForFunction(() => typeof window.__setSatchelRenderFidelityPayload === 'function');
  await page.evaluate(async (payload: MailPayload) => {
    await window.__setSatchelRenderFidelityPayload!(payload);
  }, mailPayload(bodyHtml, id));
}

/**
 * Screen position of a mail link, so a test can drive the pointer the way a
 * reader does. Addressing the anchor through the frame would fail Playwright's
 * hit-target check: link interaction is handled by targets that sit above the
 * frame, because WebKit runs no listeners inside a scriptless sandboxed frame.
 */
async function mailLinkPoint(page: Page, label: string) {
  const frame = page.locator('iframe.satchel-email-document');
  const frameBox = await frame.boundingBox();
  const rect = await frame.evaluate((element, name) => {
    const doc = (element as HTMLIFrameElement).contentDocument;
    const anchor = Array.from(doc?.querySelectorAll('a') ?? [])
      .find((candidate) => (candidate.textContent || '').trim() === name);
    const box = anchor?.getBoundingClientRect();
    return box ? { left: box.left, top: box.top, width: box.width, height: box.height } : null;
  }, label);
  if (!frameBox || !rect) throw new Error(`mail link "${label}" not found`);
  return { x: frameBox.x + rect.left + rect.width / 2, y: frameBox.y + rect.top + rect.height / 2 };
}

async function renderMail(page: Page, bodyHtml: string) {
  await page.goto('/render-fidelity.html');
  await setMail(page, bodyHtml);
  const frame = page.locator('iframe.satchel-email-document');
  await expect(frame).toBeVisible();
  return frame.contentFrame();
}

test('blends sender canvas backgrounds into the dark reader while retaining coloured panels', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('satchel.theme', 'dark'));
  const mail = await renderMail(page, `
    <style>html, body { background: #174c70 !important; } .canvas { background: #f4f4f4; }</style>
    <body style="background: white !important">
      <table class="canvas" width="100%"><tr><td bgcolor="#ffffff">
        <p>Canvas content</p><div class="banner" style="background:#174c70;color:white">Coloured banner</div>
      </td></tr></table>
    </body>
  `);
  const canvas = await page.locator('.html-document-shell').evaluate((element) => getComputedStyle(element).backgroundColor);
  for (const selector of ['html', 'body', '.satchel-email-body', '.canvas', 'td']) {
    expect(await mail.locator(selector).evaluate((element) => getComputedStyle(element).backgroundColor)).toBe(canvas);
  }
  expect(await mail.locator('.banner').evaluate((element) => getComputedStyle(element).backgroundColor)).toBe('rgb(23, 76, 112)');
});

test('evaluates sender media queries against the message pane width', async ({ page }) => {
  await page.setViewportSize({ width: 520, height: 900 });
  const mail = await renderMail(page, `
    <style>
      .responsive-layout { width: 600px; }
      .compact-only { display: none; }
      @media screen and (max-width: 479px) {
        .responsive-layout { width: 100%; }
        .compact-only { display: block; }
      }
    </style>
    <div class="responsive-layout">
      <span class="compact-only">Compact layout</span>
      Responsive content
    </div>
  `);

  await expect(mail.getByText('Compact layout')).toBeVisible();
  const metrics = await mail.locator('html').evaluate((element) => ({
    mediaMatches: element.ownerDocument.defaultView?.matchMedia('(max-width: 479px)').matches,
    scrollWidth: element.scrollWidth,
    clientWidth: element.clientWidth,
  }));
  expect(metrics.mediaMatches).toBe(true);
  expect(metrics.clientWidth).toBeLessThan(479);
  expect(metrics.scrollWidth).toBe(metrics.clientWidth);

  await page.setViewportSize({ width: 800, height: 900 });
  await expect(mail.getByText('Compact layout')).toBeHidden();
  expect(await mail.locator('html').evaluate((element) => (
    element.ownerDocument.defaultView?.matchMedia('(max-width: 479px)').matches
  ))).toBe(false);
});

test('preserves table header words when a sender declares a narrow status column', async ({ page }) => {
  await page.setViewportSize({ width: 520, height: 900 });
  const mail = await renderMail(page, `
    <table width="100%" style="width:100%;border-collapse:collapse">
      <thead><tr>
        <th style="width:24px;padding:16px">Status</th>
        <th style="width:100%;padding:16px">Job</th>
        <th style="white-space:nowrap;padding:16px">Annotations</th>
      </tr></thead>
      <tbody><tr><td>×</td><td>CI / quality</td><td>3</td></tr></tbody>
    </table>
  `);
  const status = mail.getByRole('columnheader', { name: 'Status' });

  await expect(status).toBeVisible();
  const metrics = await status.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return {
      width: rect.width,
      height: rect.height,
      overflowWrap: getComputedStyle(element).overflowWrap,
    };
  });
  expect(metrics.overflowWrap).toBe('normal');
  expect(metrics.width).toBeGreaterThan(50);
  expect(metrics.height).toBeLessThan(80);
});

test('keeps fixed legacy tables and blocked images inside the message viewport', async ({ page }) => {
  await page.setViewportSize({ width: 520, height: 900 });
  const mail = await renderMail(page, `
    <table width="570" style="width:570px"><tbody><tr><td>Legacy content</td></tr></tbody></table>
    <img src="https://tracker.example.test/pixel.png" alt="Tracking pixel">
  `);

  const tableWidth = await mail.getByRole('table').evaluate((table) => table.getBoundingClientRect().width);
  const viewportWidth = await mail.locator('html').evaluate((html) => html.clientWidth);
  expect(tableWidth).toBeLessThanOrEqual(viewportWidth);
  expect(await mail.locator('html').evaluate((html) => html.scrollWidth)).toBe(viewportWidth);
  await expect(mail.getByRole('img', { name: 'Tracking pixel' })).not.toHaveAttribute('src');
  await expect(mail.getByRole('img', { name: 'Tracking pixel' })).toHaveAttribute(
    'data-blocked-src',
    'https://tracker.example.test/pixel.png',
  );
  await expect(page.getByText(/1 external image blocked/)).toBeVisible();
});

test('keeps mail scripts disabled inside the isolated document', async ({ page }) => {
  const mail = await renderMail(page, `
    <script>parent.__satchelMailScriptRan = true</script>
    <p>Safe content</p>
  `);

  await expect(mail.getByText('Safe content')).toBeVisible();
  expect(await page.evaluate(() => '__satchelMailScriptRan' in window)).toBe(false);
});

test('blocks form submission and declarative escape from the mail sandbox', async ({ page }) => {
  const mail = await renderMail(page, `
    <form action="/sandbox-escape" method="get" target="_top">
      <button type="submit">Submit form</button>
    </form>
  `);
  const frame = page.locator('iframe.satchel-email-document');

  expect(await frame.getAttribute('sandbox')).toBe('allow-same-origin');
  const pageUrl = page.url();
  await mail.getByRole('button', { name: 'Submit form' }).click();
  await page.waitForTimeout(100);
  expect(page.url()).toBe(pageUrl);
  expect(await mail.locator('html').evaluate((html) => html.ownerDocument.location.href)).toBe('about:srcdoc');
  await expect(mail.getByRole('button', { name: 'Submit form' })).toBeVisible();
});

test('resizes for async content growth and shrinks after replacing the mail document', async ({ page }) => {
  const mail = await renderMail(page, '<div id="grow">Initial content</div>');
  const frame = page.locator('iframe.satchel-email-document');
  const initialHeight = await frame.evaluate((element) => element.getBoundingClientRect().height);

  await mail.locator('#grow').evaluate((element) => {
    (element as HTMLElement).style.height = '600px';
  });
  await expect.poll(
    () => frame.evaluate((element) => element.getBoundingClientRect().height),
  ).toBeGreaterThan(initialHeight + 400);
  const grownHeight = await frame.evaluate((element) => element.getBoundingClientRect().height);

  await setMail(page, '<p>Replacement content</p>', 'replacement-mail');
  await expect(mail.getByText('Replacement content')).toBeVisible();
  await expect(page.locator('iframe.satchel-email-document')).toHaveCount(1);
  await expect.poll(
    () => frame.evaluate((element) => element.getBoundingClientRect().height),
  ).toBeLessThan(grownHeight - 400);
});

test('sizes the mail document while a remote image never finishes loading', async ({ page }) => {
  // The frame's `load` event waits for every subresource. A newsletter whose
  // CDN stalls therefore never fires it, and sizing the frame from that event
  // left the mail clipped to the 150px default of an unsized iframe.
  await page.route('**/stalled-tracking-pixel.png', () => {
    // Intentionally neither fulfilled nor aborted: the request stays pending.
  });
  await page.addInitScript(() => {
    localStorage.setItem('satchel:images-allowed-senders', JSON.stringify(['sender@example.test']));
  });

  const paragraphs = Array.from({ length: 40 }, (_, index) => `<p>Zeile ${index + 1}</p>`).join('');
  const mail = await renderMail(page, `
    <img src="${stalledPixelUrl}" alt="Zählpixel">
    ${paragraphs}
  `);
  const frame = page.locator('iframe.satchel-email-document');

  await expect(mail.getByText('Zeile 40')).toBeAttached();
  await expect.poll(
    () => frame.evaluate((element) => element.getBoundingClientRect().height),
    { message: 'frame stays at the unsized default while a subresource hangs' },
  ).toBeGreaterThan(400);
  // The whole mail has to fit: the shell hides whatever the frame cuts off.
  const fits = await frame.evaluate((element) => {
    const doc = (element as HTMLIFrameElement).contentDocument;
    return (doc?.body?.scrollHeight ?? 0) <= element.getBoundingClientRect().height;
  });
  expect(fits).toBe(true);
});

test('keeps links clickable and hoverable while a remote image never finishes loading', async ({ page }) => {
  // Same root cause as the sizing regression: the click and hover handlers used
  // to be attached on the frame's `load` event, so a mail with a stalled
  // subresource had no handlers at all — clicking a button did nothing and the
  // destination popover never appeared.
  await page.route('**/stalled-tracking-pixel.png', () => {
    // Intentionally neither fulfilled nor aborted: the request stays pending.
  });
  await page.addInitScript(() => {
    localStorage.setItem('satchel:images-allowed-senders', JSON.stringify(['sender@example.test']));
  });

  const mail = await renderMail(page, `
    <img src="${stalledPixelUrl}" alt="Zählpixel">
    <p>Auswählbarer Text neben dem Link</p>
    <a href="https://localhost/angebot">Zum Ratgeber</a>
  `);
  await expect(mail.getByRole('link', { name: 'Zum Ratgeber' })).toBeAttached();

  const point = await mailLinkPoint(page, 'Zum Ratgeber');
  await page.mouse.move(point.x, point.y);
  await expect(page.getByRole('complementary', { name: 'Check the destination' })).toBeVisible();
  await page.mouse.click(point.x, point.y);
  await expect(page.getByRole('alertdialog', { name: 'Check the link before opening' })).toBeVisible();
});

test('routes mail links through the existing external-link protection', async ({ page }) => {
  const mail = await renderMail(page, '<p>Text im Mailbody</p><a href="https://localhost/report">Review report</a>');
  await expect(mail.getByRole('link', { name: 'Review report' })).toBeAttached();

  const point = await mailLinkPoint(page, 'Review report');
  await page.mouse.move(point.x, point.y);
  await expect(page.getByRole('complementary', { name: 'Check the destination' })).toBeVisible();
  await page.mouse.click(point.x, point.y);
  await expect(page.getByRole('alertdialog', { name: 'Check the link before opening' })).toBeVisible();
  await expect(page.getByText('The destination points at this computer.')).toBeVisible();

  // The link targets must not turn the mail body into a dead surface.
  const selected = await page.locator('iframe.satchel-email-document').evaluate((element) => {
    const doc = (element as HTMLIFrameElement).contentDocument;
    const paragraph = doc?.querySelector('p');
    const range = doc?.createRange();
    if (!doc || !paragraph || !range) return '';
    range.selectNodeContents(paragraph);
    const selection = doc.defaultView?.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    return selection?.toString() ?? '';
  });
  expect(selected).toBe('Text im Mailbody');
});

// #125: a hidden "!" turned the label into prose, so the domain mismatch went unchecked.
test('compares a link with the label the reader sees, hidden characters left out', async ({ page }) => {
  await renderMail(page, `
    <style>.tail { display: none }</style>
    <p>Bitte bestätigen:</p>
    <a href="https://evil.example/login">paypal.com<span class="tail">!</span></a>
  `);

  const point = await mailLinkPoint(page, 'paypal.com!');
  await page.mouse.click(point.x, point.y);
  const dialog = page.getByRole('alertdialog', { name: 'Check the link before opening' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText('The shown domain and the actual domain do not match.')).toBeVisible();
  // Trust never overrides a warning, so it is not offered on one.
  await expect(dialog.getByRole('switch')).toHaveCount(0);
});

/**
 * A phone-width message must not sit inside a frame.
 *
 * Two cards wrapped a message: one around the whole bubble, one around the HTML
 * document inside it. Removing the first left the second in place and looking
 * exactly like the thing that was supposed to be gone — a grey rounded panel
 * around the mail. The border and the radius are what separate a document from
 * things beside it, and on a phone nothing is beside it.
 */
test('drops the document frame and nested canvas background at phone width', async ({ page }) => {
  await page.setViewportSize({ width: 411, height: 900 });
  await renderMail(page, '<p>Hallo</p>');

  const shell = page.locator('.html-document-shell').first();
  await expect(shell).toBeVisible();

  const style = await shell.evaluate((element) => {
    const computed = getComputedStyle(element);
    return {
      borderTopWidth: computed.borderTopWidth,
      borderRadius: computed.borderTopLeftRadius,
      background: computed.backgroundColor,
    };
  });

  expect(style.borderTopWidth).toBe('0px');
  expect(style.borderRadius).toBe('0px');
  // Mobile mail belongs directly on the conversation surface. Auto-dark still
  // normalises sender colours inside the isolated document, so an extra grey
  // canvas would only recreate the nested card this breakpoint removes.
  expect(style.background).toBe('rgba(0, 0, 0, 0)');
});

test('keeps the document frame where there is room beside it', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await renderMail(page, '<p>Hallo</p>');

  const shell = page.locator('.html-document-shell').first();
  const radius = await shell.evaluate(
    (element) => getComputedStyle(element).borderTopLeftRadius,
  );
  expect(radius).toBe('12px');
});

const cssBypassPayload = '<p>Hi</p><style></sty@import;le><link rel="preconnect" href="https://u123.t.example"><meta http-equiv="refresh" content="0;url=https://evil.example/login"></style>';
const mimeBypassPayload = 'image/png"><link rel=preconnect href=https://u1.t.example>';

for (const [label, html] of [
  ['issue 90', cssBypassPayload],
  ['issue 90 stylesheet breakout', cssBypassPayload.replace('@import;', '')],
  ['issue 91', '<img src="cid:x">'],
]) {
  test(`${label}: mail metadata cannot reach the iframe`, async ({ page }) => {
    await page.goto('/render-fidelity.html');
    const payload = mailPayload(html);
    payload.attachments = [{
      id: 'malicious-inline', filename: 'logo.png', contentType: mimeBypassPayload,
      contentId: 'x', size: 4, isInline: true,
      dataUrl: `data:${mimeBypassPayload};base64,AAAA`,
    }];
    await page.waitForFunction(() => typeof window.__setSatchelRenderFidelityPayload === 'function');
    await page.evaluate(async (value) => window.__setSatchelRenderFidelityPayload!(value), payload);
    const frame = page.locator('iframe.satchel-email-document');
    await expect(frame).toBeVisible();
    const mail = frame.contentFrame();
    await expect(mail.locator('link')).toHaveCount(0);
    // The app's own CSP and charset metas remain; no sender meta is accepted.
    await expect(mail.locator('meta[http-equiv="refresh"], meta[content*="evil.example"]')).toHaveCount(0);
    await expect(mail.locator('meta')).toHaveCount(3);
    await expect(mail.locator('.satchel-email-body meta')).toHaveCount(0);
  });
}

test('an allow-listed sender domain needs a trusted DMARC pass before fetching pixels', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('satchel:images-allowed-domains', '["example.test"]'));
  const requests: string[] = [];
  await page.route('https://tracker.example.test/**', async (route) => {
    requests.push(route.request().url());
    await route.fulfill({ status: 200, contentType: 'image/png', body: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aN9sAAAAASUVORK5CYII=', 'base64') });
  });
  const mail = await renderMail(page, '<p>Forged sender</p><img src="https://tracker.example.test/forged">');
  await expect(mail.locator('img')).toHaveAttribute('data-blocked-src', 'https://tracker.example.test/forged');
  await expect(page.getByText(/1 external image blocked/)).toBeVisible();
  expect(requests).toEqual([]);
  await page.evaluate(async (payload) => window.__setSatchelRenderFidelityPayload!({ ...payload, trustedDmarcPass: true }), mailPayload('<p>Authenticated sender</p><img src="https://tracker.example.test/authenticated">'));
  await expect.poll(() => requests).toEqual(['https://tracker.example.test/authenticated']);
});

test('resend prefill mounts the real editor without fetching foreign tracking pixels', async ({ page }) => {
  const requests: string[] = [];
  await page.route('https://tracker.example.test/**', async (route) => {
    requests.push(route.request().url());
    await route.fulfill({ status: 204 });
  });
  await page.goto('/render-fidelity.html');
  await page.evaluate(async () => {
    const load = (path: string) => import(/* @vite-ignore */ path);
    const { createApp, h } = await load('/node_modules/.vite/deps/vue.js');
    const { i18n } = await load('/src/i18n/index.ts');
    const { default: Editor } = await load('/src/components/mail/RichTextEditor.vue');
    const { resendEditorHtml } = await load('/src/utils/composerThreadPrefill.ts');
    const root = document.createElement('div');
    document.body.append(root);
    const html = resendEditorHtml('<p>Resend body</p><input type=image src=https://tracker.example.test/input><img src=https://tracker.example.test/plain><img src="&#104;ttps://tracker.example.test/encoded"><img srcset="https://tracker.example.test/srcset 2x">');
    createApp({ render: () => h(Editor, { modelValue: '', html }) }).use(i18n).mount(root);
  });
  await expect(page.locator('.tiptap')).toContainText('Resend body');
  await expect(page.locator('.tiptap img')).toHaveCount(0);
  expect(requests).toEqual([]);
});
