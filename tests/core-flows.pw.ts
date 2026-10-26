import type { Locator } from '@playwright/test';
import { expect, test } from './mock-contract.fixture';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
});

test('renders the inbox and opens a thread', async ({ page }) => {
  await page.getByRole('button', { name: 'View', exact: true }).click();
  await page.getByRole('menuitemradio', { name: 'List', exact: true }).click();
  await page.getByText('Google Cloud billing threshold reached', { exact: true }).first().click();
  await expect(page.getByText('Your configured billing threshold for Project Atlas has been reached.').first()).toBeVisible();
});

test('opens a reply composer for the active conversation', async ({ page }) => {
  await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  await expect(composer).toBeVisible();
  await expect(composer.getByText('Reply', { exact: true })).toBeVisible();
});

test('tabs from addressing through the subject to the message body', async ({ page }) => {
  await page.goto('/compose');
  await page.getByLabel('Sender', { exact: true }).focus();
  await page.keyboard.press('Tab');
  const to = page.getByPlaceholder('recipient@example.com');
  await expect(to).toBeFocused();
  await to.fill('person@example.com');
  await page.keyboard.press('Enter');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Cc/Bcc', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByLabel('Subject', { exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.locator('.ProseMirror')).toBeFocused();
});

test('keeps long composer text from covering the conversation history', async ({ page }) => {
  await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  const editor = composer.locator('.ProseMirror');
  const historyToggle = composer.getByRole('button', { name: /Show history/ });

  await editor.fill(Array.from({ length: 12 }, (_, index) => `Absatz ${index + 1}`).join('\n\n'));

  const editorBox = await editor.boundingBox();
  const toggleBox = await historyToggle.boundingBox();
  expect(editorBox).not.toBeNull();
  expect(toggleBox).not.toBeNull();
  expect(editorBox!.y + editorBox!.height).toBeLessThanOrEqual(toggleBox!.y);

  await historyToggle.click();
  await expect(composer.getByRole('button', { name: 'Hide history' })).toBeVisible();
});

test('archives the active thread and advances to the next one', async ({ page }) => {
  // The detail-pane action, not the hover quick action a priority-card row
  // also carries. Scoped to the toolbar: Done reads its shortcut from the
  // title now, so both buttons answer to the same accessible name.
  await page.locator('.thread-toolbar').getByRole('button', { name: 'Done' }).click();
  await expect(page.getByText('Staging login 500 after deployment', { exact: true })).toHaveCount(0);
  await expect(page.getByTitle('More actions')).toBeVisible();
});

test('searches the local mailbox deterministically', async ({ page }) => {
  await page.getByTitle('Search (/)').click();
  const search = page.getByLabel('Search term');
  await search.fill('Invoice March');
  const results = page.getByRole('listbox', { name: 'Search results' });
  await expect(
    results.getByText('Invoice March (waiting for vendor correction)', { exact: true }),
  ).toBeVisible();
});

test('filters mailboxes from the sidebar and restores the global scope', async ({ page }) => {
  const sidebar = page.locator('aside').first();
  const filter = sidebar.getByRole('button', { name: 'Filter mailboxes' });
  await filter.click();
  const menu = page.getByRole('menu', { name: 'Filter mailboxes' });
  const account = menu.getByRole('menuitemcheckbox').nth(1);
  const accountEmail = await account.getAttribute('title');
  await account.click();
  await expect(menu).toHaveCount(0);
  await expect(page.locator('.pane-header')).toContainText(accountEmail!);

  await filter.click();
  await expect(menu.getByRole('menuitemcheckbox').nth(1)).toHaveAttribute('aria-checked', 'true');
  await menu.getByRole('menuitemcheckbox', { name: 'All accounts' }).click();
  await expect(page.locator('.pane-header')).toContainText('All accounts');
  await expect(filter).toBeFocused();
});

test('composes from the sidebar plus button', async ({ page }) => {
  await page.locator('aside').first().getByRole('button', { name: 'Write a new message' }).click();
  await expect(page.getByRole('dialog', { name: 'Compose email' })).toBeVisible();
});

test('reports no hits instead of failing silently', async ({ page }) => {
  await page.getByTitle('Search (/)').click();
  await page.getByLabel('Search term').fill('zzz-definitely-not-in-this-mailbox');
  await expect(page.getByText('No hits found.')).toBeVisible();
});

test('opens a search hit in the thread view', async ({ page }) => {
  await page.getByTitle('Search (/)').click();
  await page.getByLabel('Search term').fill('Invoice March');
  const results = page.getByRole('listbox', { name: 'Search results' });
  await results.getByRole('option').first().click();
  await expect(page.getByRole('listbox', { name: 'Search results' })).toHaveCount(0);
  await expect(page.getByTitle('More actions')).toBeVisible();
});

test('offers commands in the omni panel while the query is empty', async ({ page }) => {
  await page.keyboard.press('ControlOrMeta+k');
  const panel = page.getByRole('dialog', { name: 'Search and commands' });
  await expect(panel).toBeVisible();
  await expect(panel.getByRole('option', { name: 'Compose new email' })).toBeVisible();
});

test('hands a query over to the full search page', async ({ page }) => {
  await page.getByTitle('Search (/)').click();
  await page.getByLabel('Search term').fill('Invoice');
  await page.getByRole('option', { name: /Open full search/ }).click();
  await expect(page.getByRole('heading', { name: 'Search', exact: true })).toBeVisible();
  await expect(page.getByText('Invoice March (waiting for vendor correction)').first()).toBeVisible();
});

/**
 * The overlays put entries on the history stack so the back gesture can close
 * them. That once reclaimed the entry when a panel was closed from the UI —
 * and an overlay that closes *because* it navigated had its destination undone
 * a tick later. This pins the destination, not the mechanism.
 */
test('stays on the page an overlay navigated to', async ({ page }) => {
  await page.getByTitle('Search (/)').click();
  await page.getByLabel('Search term').fill('Invoice');
  await page.getByRole('option', { name: /Open full search/ }).click();
  await expect(page.getByRole('heading', { name: 'Search', exact: true })).toBeVisible();

  // Long enough for a rewind scheduled on the next tick to have fired.
  await page.waitForTimeout(500);
  await expect(page).toHaveURL(/\/search/);
  await expect(page.getByRole('heading', { name: 'Search', exact: true })).toBeVisible();
});

/*
 * Send, cancel and undo run against the mock command transport
 * (`src/services/backend/mock.ts`), which `dev:mock` puts behind
 * `services/backend/transport`. The store code above it is the code the app
 * ships — there is no longer a mock branch that skips the call. The failure
 * test below is what proves the call actually happens.
 */
test('sends a reply and shows it in the conversation', async ({ page }) => {
  await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  await composer.locator('.ProseMirror').fill('The OAuth fix is live on staging.');

  await composer.getByRole('button', { name: 'Send', exact: true }).click();

  await expect(composer).toHaveCount(0);
  await expect(page.getByText('The OAuth fix is live on staging.').first()).toBeVisible();
});

/**
 * The negative half. `?backendFailure=send_message` makes the transport reject
 * exactly that command; a suite that never reaches the backend would stay green
 * here, which is how a broken send path used to pass every gate.
 */
test('keeps the draft and reports the failure when the send command refuses', async ({ page }) => {
  await page.goto('/?backendFailure=send_message');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();

  await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  await composer.locator('.ProseMirror').fill('This one must not silently vanish.');
  await composer.getByRole('button', { name: 'Send', exact: true }).click();

  await expect(composer).toBeVisible();
  await expect(page.getByText(/Sending failed:/).first()).toBeVisible();
});

test('cancels a queued outbox item', async ({ page }) => {
  await page.goto('/outbox');
  const queued = page.getByRole('listitem').filter({ hasText: 'Angebot Websiterelaunch' });
  await expect(queued).toHaveCount(1);

  await queued.getByRole('button', { name: 'Discard', exact: true }).click();

  await expect(queued).toHaveCount(0);
});

test('reverts an archive through the undo affordance', async ({ page }) => {
  await page.locator('.thread-toolbar').getByRole('button', { name: 'Done' }).click();
  await expect(page.getByText('Staging login 500 after deployment', { exact: true })).toHaveCount(0);

  await page.getByRole('button', { name: /undo, \d+ seconds remaining/ }).click();

  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
});

/**
 * Shortcuts used to act on the inbox's active thread from every page: `e` in
 * Settings archived a mail nobody was looking at.
 */
test('leaves the active thread alone when e is pressed in Settings', async ({ page }) => {
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page).toHaveURL(/\/settings/);

  await page.keyboard.press('e');
  await page.keyboard.press('#');

  await expect(page.getByRole('button', { name: /undo, \d+ seconds remaining/ })).toHaveCount(0);
  await page.goBack();
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
});

/** Enter and Space on the toolbar menu, arrows through it, Escape back to the trigger. */
test('works the More menu from the keyboard', async ({ page }) => {
  const more = page.getByRole('button', { name: 'More actions' });
  await more.focus();
  await page.keyboard.press('Enter');

  const menu = page.getByRole('menu', { name: 'More actions' });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'Reply' })).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(menu.getByRole('menuitem', { name: 'Send again' })).toBeFocused();

  await page.keyboard.press('Escape');
  await expect(menu).toHaveCount(0);
  await expect(more).toBeFocused();
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
});

/**
 * The mock transport throws on a command it has no answer for, and says so on
 * the console. Without this the suite would quietly drift back into proving
 * nothing as soon as a store starts calling something new.
 */
test('asks the backend for nothing it cannot answer', async ({ page }) => {
  const unimplemented: string[] = [];
  page.on('console', (message) => {
    if (message.text().includes('unimplemented command')) unimplemented.push(message.text());
  });

  await page.goto('/');
  await page.getByText('Google Cloud billing threshold reached', { exact: true }).first().click();
  await page.getByTitle('Search (/)').click();
  await page.getByLabel('Search term').fill('Invoice');
  await page.keyboard.press('Escape');
  await page.goto('/outbox');
  await page.goto('/drafts');

  expect(unimplemented).toEqual([]);
});

test('discards a failed move from the outbox', async ({ page }) => {
  await page.goto('/outbox');
  const move = page.getByRole('listitem').filter({ hasText: 'Project archive' });
  await move.click();
  await expect(page.getByText('Discard stops retrying this move and returns the conversation to its last confirmed folder. It does not delete any email.')).toBeVisible();
  await move.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(move).toHaveCount(0);
  await expect(page.getByText('Project archive', { exact: true })).toHaveCount(0);
});

test('opening a scheduled draft cancels sending before editing', async ({ page }) => {
  await page.goto('/drafts');
  const scheduled = page.getByRole('listitem').filter({ hasText: 'Wochenrückblick KW 31' });
  await scheduled.locator('.row__open').dblclick();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  await expect(composer).toBeVisible({ timeout: 30_000 });
  // The live region repeats the toast for screen readers; assert the visible toast.
  await expect(page.locator('.toast__message').filter({ hasText: 'Sending was cancelled so you can edit this draft. Choose Send later again to schedule the revised message.' })).toBeVisible();
  const queued = await page.evaluate(async () => {
    const modulePath = '/src/services/backend/outbox.ts';
    const { backendListOutbox } = await import(modulePath);
    return (await backendListOutbox()).some((item: { id: string }) => item.id === 'ob-scheduled');
  });
  expect(queued).toBe(false);
});


test('requires an explicit opt-in for third-party contact pictures and remembers it', async ({ page }) => {
  await page.goto('/settings?section=contacts');
  const consent = page.getByRole('switch', { name: 'Allow third-party contact pictures' });
  await expect(consent).not.toBeChecked();
  await expect(page.getByText(/share address hashes with Gravatar and sender domains with Google and Cloudflare/)).toBeVisible();
  await page.getByText("Allow third-party contact pictures", { exact: true }).click();
  await expect(consent).toBeChecked();
  await page.reload();
  await expect(consent).toBeChecked();
  await page.getByText("Allow third-party contact pictures", { exact: true }).click();
  await expect(consent).not.toBeChecked();
  await page.reload();
  await expect(consent).not.toBeChecked();
});

test('normalizes localized reply chains for display while retaining reply spelling', async ({ page }) => {
  await page.route(/\/src\/data\/mock\.ts(?:\?.*)?$/, async (route) => {
    const response = await route.fetch();
    const body = (await response.text()).replace(
      /subject: (['"])(?:Re: )?Staging login 500 after deployment\1/g,
      'subject: "SV: Odp: Antw: Staging login 500 after deployment"',
    );
    await route.fulfill({ response, body });
  });
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Staging login 500 after deployment', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
  await expect(page.getByRole('dialog', { name: 'Compose email' }).getByLabel('Subject', { exact: true }))
    .toHaveValue('SV: Staging login 500 after deployment');
});

// #164: Settings had no filter, and Esc did nothing there on the desktop.
test('filters Settings from the keyboard and leaves them with Escape', async ({ page }) => {
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page).toHaveURL(/\/settings/);

  await page.keyboard.press('/');
  await page.keyboard.type('undo');
  const nav = page.getByRole('navigation', { name: 'Settings' });
  await expect(nav.getByRole('button', { name: /^Sending/ })).toBeVisible();
  await expect(nav.getByRole('button', { name: /^Swipe gestures/ })).toHaveCount(0);
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/section=sending/);

  // The first Escape empties the filter, the second leaves Settings.
  await page.keyboard.press('Escape');
  await expect(nav.getByRole('button', { name: /^Swipe gestures/ })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page).not.toHaveURL(/\/settings/);
});

/** Whether the centre of `locator` is the element itself, not something on top of it. */
async function isUncovered(locator: Locator) {
  return locator.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return element.contains(document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2));
  });
}

/**
 * The mock starts with two error toasts (a failed sync, a failed send). The
 * stack sat in the corner where both the reply bar and the composer's send
 * row end, and at 900x700 it took the click meant for Send.
 */
test('keeps error toasts off the reply bar and the send button', async ({ page }) => {
  await expect(page.locator('.toast--error').first()).toBeVisible();
  // The active thread carries an unsent draft, so the bar resumes it.
  await expect.poll(() => isUncovered(page.locator('.reply-bar').getByRole('button', { name: 'Resume draft', exact: true }))).toBe(true);

  await page.setViewportSize({ width: 900, height: 700 });
  await page.getByText('Staging login 500 after deployment', { exact: true }).first().click();
  await page.getByRole('button', { name: 'Reply', exact: true }).first().click();
  const composer = page.getByRole('dialog', { name: 'Compose email' });
  await expect(page.locator('.toast--error').first()).toBeVisible();
  const send = composer.getByRole('button', { name: 'Send', exact: true });
  await expect.poll(() => isUncovered(send)).toBe(true);
});

/**
 * The reader is one column: the avatars stand on its left edge, and the
 * draft, the reply bar and the toolbar's first button keep to the same line.
 * The draft card and the reply bar used to hang into the gutter, wider than
 * the messages above them.
 */
test('keeps the draft, the reply bar and the toolbar on the reading column', async ({ page }) => {
  const column = await page.locator('.thread-reading-column').boundingBox();
  const sent = await page.locator('.message-surface:not(.message-surface--draft) .message-header').first().boundingBox();
  const draft = await page.locator('.message-surface--draft .message-header').first().boundingBox();
  const bar = await page.locator('.reply-bar').boundingBox();
  const done = await page.locator('.thread-toolbar__done').boundingBox();
  if (!column || !sent || !draft || !bar || !done) throw new Error('reader not laid out');

  expect(Math.abs(sent.x - column.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(draft.x - column.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(bar.x - column.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(bar.x + bar.width - (column.x + column.width))).toBeLessThanOrEqual(1);
  expect(Math.abs(done.x - column.x)).toBeLessThanOrEqual(1);
});

/**
 * Each section shares a grid row with its neighbour. With the default
 * `align-content` the taller column's surplus went to the shorter section's
 * heading track: "Mailbox focused" showed its title and, 300px further down
 * and out of view, its list.
 */
test('puts every shortcut list right under its heading', async ({ page }) => {
  await page.keyboard.press('?');
  const sections = page.locator('.groups > section');
  await expect(sections.first()).toBeVisible();

  const layout = await sections.evaluateAll((nodes) => nodes.map((node) => {
    const heading = node.querySelector('h4')!.getBoundingClientRect();
    const list = node.querySelector('ul')!.getBoundingClientRect();
    return {
      title: node.querySelector('h4')!.textContent ?? '',
      heading: heading.height,
      gap: list.top - heading.bottom,
    };
  }));

  expect(layout.length).toBeGreaterThan(3);
  for (const { title, heading, gap } of layout) {
    expect(heading, title).toBeLessThan(40);
    expect(gap, title).toBeLessThan(16);
  }
});

