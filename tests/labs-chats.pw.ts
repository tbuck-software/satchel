import { expect, test, type Page } from './mock-contract.fixture';

/*
 * Labs → Chats against the mock transport: the switch in Settings, the Chats
 * row it brings into the sidebar, the chat view and its quick reply. The mock
 * adds two Delta Chat threads when the flag is on.
 */

async function openLabs(page: Page) {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('button', { name: 'Labs', exact: true }).click();
}

const sidebarChats = (page: Page) => page.locator('[data-nav="chats"]');

test.describe('desktop', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('off by default; the switch brings Chats, and Chats reads and replies', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByText('Staging login 500 after deployment', { exact: true }).first()).toBeVisible();
    await expect(sidebarChats(page)).toHaveCount(0);
    // With the flag off a deep link lands in the Inbox.
    await page.goto('/chats');
    await expect(page).toHaveURL(/\/inbox/);

    await openLabs(page);
    await expect(page.getByText('These features are not finished.', { exact: false })).toBeVisible();
    const row = page.locator('[data-labs-flag="chats"]');
    await expect(row.getByRole('switch')).not.toBeChecked();
    await row.locator('.toggle-switch').click();
    await expect(row.getByRole('switch')).toBeChecked();
    // The first Escape only leaves the focused switch; close the dialog itself.
    await page.getByRole('dialog', { name: 'Settings' }).getByRole('button', { name: /^Close/ }).click();

    await sidebarChats(page).click();
    await expect(page).toHaveURL(/\/chats/);
    const list = page.getByRole('region', { name: 'Chats', exact: true });
    await expect(list.getByText('Lena Brandt', { exact: true })).toBeVisible();
    await expect(list.getByText('Familie', { exact: true })).toBeVisible();

    await list.getByText('Lena Brandt', { exact: true }).click();
    const chat = page.getByRole('region', { name: 'Lena Brandt', exact: true });
    await expect(chat.getByText('End-to-end encrypted', { exact: true })).toBeVisible();
    await expect(chat.locator('[data-chat-bubble="peer"]').last()).toContainText('Passt, ich bring den Schlüssel mit');
    // The Delta Chat footer after "-- " is not part of the bubble.
    await expect(chat.getByText('Sent with my Delta Chat Messenger', { exact: false })).toHaveCount(0);
    await expect(chat.locator('[data-chat-bubble="own"]')).toHaveCount(2);

    const input = chat.locator('[data-chat-input]');
    await input.fill('Bis gleich');
    await input.press('Shift+Enter');
    await input.pressSequentially('Klingel ist kaputt');
    await input.press('Enter');
    await expect(chat.locator('[data-chat-bubble="own"]')).toHaveCount(3);
    await expect(chat.locator('[data-chat-bubble="own"]').last()).toContainText('Bis gleich\nKlingel ist kaputt');
    await expect(input).toHaveValue('');

    // A group names its members' messages and says it is not encrypted.
    await list.getByText('Familie', { exact: true }).click();
    const group = page.getByRole('region', { name: 'Familie', exact: true });
    await expect(group.getByText('4 members', { exact: true })).toBeVisible();
    await expect(group.getByText('Not encrypted', { exact: true })).toBeVisible();
    await expect(group.locator('.chat-bubble__sender').first()).toHaveText('Mia');

    // Escape leaves the chat, then Chats.
    await group.locator('[data-chat-input]').focus();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('region', { name: 'Familie', exact: true })).toHaveCount(0);
    await page.locator('body').click({ position: { x: 5, y: 5 } });
    await page.keyboard.press('Escape');
    await expect(page).toHaveURL(/\/inbox/);

    // g h opens Chats while the flag is on (g c stays Calendar).
    await page.keyboard.press('g');
    await page.keyboard.press('h');
    await expect(page).toHaveURL(/\/chats/);

    // The chat mail stays in the Inbox.
    await page.goto('/inbox');
    await expect(page.getByText('Message from Lena Brandt', { exact: true }).first()).toBeVisible();
  });
});

test.describe('locked own key', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('asks to unlock before an encrypted reply, and sends unsigned only when chosen', async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem('satchel.mock.labs', JSON.stringify({ chats: true })));
    await page.goto('/?mockChatKeyLocked=1');
    await sidebarChats(page).click();
    await page.locator('[data-chat-row]', { hasText: 'Lena Brandt' }).click();
    const chat = page.getByRole('region', { name: 'Lena Brandt', exact: true });
    const own = chat.locator('[data-chat-bubble="own"]');
    await expect(own).toHaveCount(2);
    const input = chat.locator('[data-chat-input]');
    const unlock = page.getByRole('dialog', { name: 'Unlock your key to send' });
    const unsigned = page.getByRole('dialog', { name: 'Send without your signature?' });

    // A wrong passphrase asks again; declining both questions keeps the text and sends nothing.
    await input.fill('Bin gleich da');
    await input.press('Enter');
    await unlock.getByLabel('Passphrase').fill('wrong');
    await unlock.getByRole('button', { name: 'Unlock and send' }).click();
    await expect(page.getByText('That passphrase does not unlock the key.')).toBeVisible();
    await unlock.getByRole('button', { name: 'Cancel' }).click();
    await unsigned.getByRole('button', { name: 'Cancel' }).click();
    await expect(own).toHaveCount(2);
    await expect(input).toHaveValue('Bin gleich da');

    // The right passphrase unlocks the key and the reply goes out (signed).
    await input.press('Enter');
    await unlock.getByLabel('Passphrase').fill('correct horse');
    await unlock.getByRole('button', { name: 'Unlock and send' }).click();
    await expect(own).toHaveCount(3);
    await expect(input).toHaveValue('');

    // Locked again (a restart): declining the unlock and choosing unsigned sends it.
    await page.goto('/?mockChatKeyLocked=1');
    await sidebarChats(page).click();
    await page.locator('[data-chat-row]', { hasText: 'Lena Brandt' }).click();
    await chat.locator('[data-chat-input]').fill('Ohne Signatur');
    await chat.locator('[data-chat-input]').press('Enter');
    await unlock.getByRole('button', { name: 'Cancel' }).click();
    await unsigned.getByRole('button', { name: 'Send unsigned' }).click();
    await expect(chat.locator('[data-chat-bubble="own"]').last()).toContainText('Ohne Signatur');
  });
});

test.describe('who a reply goes to', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  async function openFamily(page: Page, param: string) {
    await page.addInitScript(() => localStorage.setItem('satchel.mock.labs', JSON.stringify({ chats: true })));
    await page.goto(`/?${param}=1`);
    await sidebarChats(page).click();
    await page.locator('[data-chat-row]', { hasText: 'Familie' }).click();
    return page.getByRole('region', { name: 'Familie', exact: true });
  }

  test('a crafted message in the group changes nothing', async ({ page }) => {
    const group = await openFamily(page, 'mockChatHijack');
    await expect(group.getByText('Neue Gruppe, bitte hier antworten')).toBeVisible();
    // Shown, but marked: the sender is not part of the chat.
    await expect(group.locator('[data-chat-outsider]')).toHaveText('Mia · not in this chat');
    await expect(group.getByText('4 members', { exact: true })).toBeVisible();
    await group.locator('[data-chat-input]').fill('Danke!');
    await group.locator('[data-chat-input]').press('Enter');
    await expect(group.locator('[data-chat-bubble="own"]').last()).toContainText('Danke!');
    await expect(page.getByRole('dialog')).toHaveCount(0);
  });

  test('new recipients and plain text in an encrypted chat wait for a yes', async ({ page }) => {
    const group = await openFamily(page, 'mockChatNewMember');
    const input = group.locator('[data-chat-input]');
    const own = group.locator('[data-chat-bubble="own"]');
    await expect(own).toHaveCount(1);
    const recipients = page.getByRole('dialog', { name: 'Send to these people?' });
    const plain = page.getByRole('dialog', { name: 'Send without encryption?' });

    await input.fill('Willkommen, Noah');
    await input.press('Enter');
    await expect(recipients.getByText('noah@example.test', { exact: false })).toBeVisible();
    await recipients.getByRole('button', { name: 'Cancel' }).click();
    await expect(own).toHaveCount(1);
    await expect(input).toHaveValue('Willkommen, Noah');

    await input.press('Enter');
    await recipients.getByRole('button', { name: 'Send', exact: true }).click();
    await plain.getByRole('button', { name: 'Send unencrypted' }).click();
    await expect(own).toHaveCount(2);
    await expect(own.last()).toContainText('Willkommen, Noah');
  });
});

test.describe('phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });

  test('a chat opens as its own screen with a labelled send button', async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem('satchel.mock.labs', JSON.stringify({ chats: true })));
    await page.goto('/chats');
    const list = page.getByRole('region', { name: 'Chats', exact: true });
    await list.getByText('Lena Brandt', { exact: true }).click();
    const chat = page.getByRole('region', { name: 'Lena Brandt', exact: true });
    await expect(chat).toBeVisible();
    await expect(list).toBeHidden();
    const send = chat.getByRole('button', { name: /^Send/ });
    await expect(send).toBeVisible();
    const box = await send.boundingBox();
    expect(box!.height).toBeGreaterThanOrEqual(44);
    await chat.locator('[data-chat-input]').fill('Bin unterwegs');
    await send.click();
    await expect(chat.locator('[data-chat-bubble="own"]').last()).toContainText('Bin unterwegs');
    await chat.getByRole('button', { name: 'Back', exact: true }).click();
    await expect(list).toBeVisible();
  });
});
