import { expect, test } from './mock-contract.fixture';

// Faults and latency enter at the transport, leaving the real stores and UI intact.
test.beforeEach(async ({ page }) => {
  await page.route(/\/src\/services\/backend\/mock\.ts(?:\?.*)?$/, async (route) => {
    const response = await route.fetch();
    const source = await response.text();
    const dispatch = 'return handler(payload ?? {});';
    expect(source.includes(dispatch), 'mock transport dispatch marker').toBe(true);
    const body = source.replace(dispatch, `
      if (command === 'list_topic_memberships' && globalThis.__membershipFailure) throw new Error('membership test outage');
      if (command === 'list_templates') {
        if (globalThis.__holdTemplates) await new Promise(resolve => { globalThis.__releaseTemplates = resolve; });
        return [{ id: 'browser-template', name: 'Retained template', subject: '', body_text: 'Retained body', updated_at: '', created_at: '' }];
      }
      const result = handler(payload ?? {});
      if (command === 'add_topic_items' && globalThis.__delayAddReceipt) {
        globalThis.__delayAddReceipt = false;
        await new Promise(resolve => { globalThis.__releaseAddReceipt = resolve; });
      }
      return result;
    `);
    await route.fulfill({ response, body });
  });
});

test('thread and picker expose membership retry without presenting unknown toggles as unchecked', async ({ page }) => {
  await page.addInitScript(() => { Object.assign(globalThis, { __membershipFailure: true }); });
  await page.goto('/');
  // The reader is lazy. Wait for the selected thread before checking its memberships.
  await expect(page.getByRole('heading', { name: 'Staging login 500 after deployment', exact: true, level: 1 }))
    .toBeVisible({ timeout: 30_000 });
  const headerState = page.locator('.detail-pane .membership-state');
  const headerAlert = headerState.getByRole('alert');
  await expect(headerAlert).toContainText('Topic memberships could not be loaded');
  await page.evaluate(async () => {
    const path = '/src/stores/topics.ts';
    const { useTopicsStore } = await import(path);
    useTopicsStore().openPicker({ itemType: 'thread', itemIds: ['th-1'] });
  });
  const picker = page.getByRole('dialog', { name: 'File…', exact: true });
  await expect(picker).toBeVisible();
  await expect(picker.locator('.row-check')).toHaveCount(0);
  await expect(picker.getByRole('alert')).toContainText('Topic memberships could not be loaded');
  await page.evaluate(() => { Object.assign(globalThis, { __membershipFailure: false }); });
  await picker.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(picker.getByRole('alert')).toHaveCount(0);
  await expect(picker.locator('.row-check').first()).toBeVisible();
  await page.keyboard.press('Escape');
  // Trigger another outage to exercise the retry in the thread header.
  await page.evaluate(async () => {
    Object.assign(globalThis, { __membershipFailure: true });
    const path = '/src/stores/topics.ts';
    const threadsPath = '/src/stores/threads.ts';
    const { useTopicsStore } = await import(path);
    const { useThreadsStore } = await import(threadsPath);
    await useTopicsStore().loadMembershipsFor('thread', [useThreadsStore().activeThreadId]);
    Object.assign(globalThis, { __membershipFailure: false });
  });
  await headerState.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(headerAlert).toHaveCount(0);
  await expect(page.locator('.connection-banner')).toHaveCount(0);
});

test('a slow template refresh keeps rows in place and displays progress', async ({ page }) => {
  await page.goto('/settings?section=templates');
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await expect(settings.getByRole('heading', { name: 'Templates', exact: true, level: 3 }))
    .toBeVisible({ timeout: 30_000 });
  const list = settings.locator('.list');
  const row = list.getByText('Retained template', { exact: true });
  await expect(row).toBeVisible();
  const before = await row.boundingBox();
  await page.evaluate(async () => {
    Object.assign(globalThis, { __holdTemplates: true });
    const path = '/src/stores/templates.ts';
    const { useTemplatesStore } = await import(path);
    void useTemplatesStore().load();
  });
  const progress = settings.locator('.collection-refresh');
  await expect(progress).toBeVisible();
  await expect(list).toHaveAttribute('aria-busy', 'true');
  expect(await row.boundingBox()).toEqual(before);
  await expect(row).toBeVisible();
  await page.evaluate(() => {
    const state = globalThis as typeof globalThis & { __releaseTemplates: () => void };
    state.__releaseTemplates();
  });
  await expect(progress).toHaveCount(0);
  await expect(list).toHaveAttribute('aria-busy', 'false');
  await expect(row).toBeVisible();
});

test('the public add action Undo leaves a membership replaced by another window intact', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
  await page.evaluate(async () => {
    const topicsPath = '/src/stores/topics.ts';
    const actionsPath = '/src/composables/useThreadActions.ts';
    const mockPath = '/src/services/backend/mock.ts';
    const { useTopicsStore } = await import(topicsPath);
    const { useThreadActions } = await import(actionsPath);
    const { sendCommand } = await import(mockPath);
    const topicId = useTopicsStore().items[0].id;
    const input = { topic_id: topicId, item_type: 'thread', item_id: 'cross-window-thread' };
    Object.assign(globalThis, { __delayAddReceipt: true });
    const delayedAdd = useThreadActions().addToTopic([input.item_id], topicId);
    await sendCommand('remove_topic_item', { input });
    await sendCommand('add_topic_items', { input: { topic_id: topicId, item_type: 'thread', item_ids: [input.item_id] } });
    const state = globalThis as typeof globalThis & { __releaseAddReceipt: () => void };
    state.__releaseAddReceipt();
    await delayedAdd;
  });
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  const memberships = await page.evaluate(async () => {
    const path = '/src/services/backend/mock.ts';
    const { sendCommand } = await import(path);
    return sendCommand('list_topic_memberships', { input: { item_type: 'thread', item_ids: ['cross-window-thread'] } });
  });
  expect(memberships).toHaveLength(1);
});
