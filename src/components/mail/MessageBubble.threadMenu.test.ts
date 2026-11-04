// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { computed, shallowRef } from 'vue';
import type { Message } from '@/types/mail';
import { useImagePolicy } from '@/composables/useImagePolicy';
import { useAccountsStore } from '@/stores/accounts';
import { useUiStore } from '@/stores/ui';
import MessageBubble from './MessageBubble.vue';
import ImageGuardPhoneActions from './ImageGuardPhoneActions.vue';
import { IMAGE_GUARD_PHONE_ACTIONS } from './imageGuardActions';
import { THREAD_MESSAGE_MENU, type MessageMenu, type ThreadMessageMenuHost } from './threadMessageMenu';

/*
 * In the phone's thread reader the newest message hands its actions to the
 * nav bar's ⋯ and the reader decides which messages keep a ⋯ beside their
 * sender. The blocked-images notice keeps only Show; its always-allow choices
 * travel with the message's actions.
 */

let wrapper: VueWrapper | null = null;

function message(overrides: Partial<Message> = {}): Message {
  return {
    id: 'm-1',
    threadId: 'thread-1',
    from: { name: 'METRO', email: 'newsletter@metro.de' },
    to: [{ name: 'Tom', email: 'tom@example.com' }],
    subject: 'Angebote',
    bodyText: 'Angebote',
    bodyHtml: '<p>Angebote</p><img src="https://cdn.metro.de/a.png">',
    bodyLoaded: true,
    timestamp: '2026-09-03T10:00:03Z',
    isRead: true,
    attachments: [],
    ...overrides,
  };
}

function host(hostId: string, inline: string[] = []): ThreadMessageMenuHost {
  return {
    hostId: computed(() => hostId),
    inlineIds: computed(() => new Set(inline)),
    menu: shallowRef<MessageMenu | null>(null),
  };
}

async function settle() {
  await flushPromises();
  await vi.dynamicImportSettled();
  await flushPromises();
}

async function render(threadMenu: ThreadMessageMenuHost | null, overrides: Partial<Message> = {}) {
  wrapper = mount(MessageBubble, {
    attachTo: document.body,
    props: { message: message(overrides), canReply: true },
    global: {
      stubs: { SenderFilingMenu: true, ExternalLinkDialog: true, LinkDestinationPreview: true, IsolatedEmailDocument: true },
      provide: {
        [IMAGE_GUARD_PHONE_ACTIONS as symbol]: ImageGuardPhoneActions,
        ...(threadMenu ? { [THREAD_MESSAGE_MENU as symbol]: threadMenu } : {}),
      },
    },
  });
  await settle();
  return wrapper;
}

const moreButton = (target: VueWrapper) => target.find('.message-header [aria-label="Message actions"]');

beforeEach(() => {
  setActivePinia(createPinia());
  useUiStore().isMobile = true;
  // Tom is us: a reply to METRO alone is not a reply all.
  useAccountsStore().accounts = [{ id: 'account-1', email: 'tom@example.com' }] as never;
  const policy = useImagePolicy();
  policy.setMode('allowlist');
  policy.revoke('newsletter@metro.de');
  policy.revokeDomain('metro.de');
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  vi.unstubAllGlobals();
  document.body.innerHTML = '';
});

describe('a message in the phone thread reader', () => {
  it('hands its actions to the nav bar and drops its own ⋯ unless the reader keeps it', async () => {
    const threadMenu = host('m-1');
    const bubble = await render(threadMenu);

    expect(moreButton(bubble).exists()).toBe(false);
    const menu = threadMenu.menu.value!;
    expect(menu).not.toBeNull();
    expect(menu.canReply).toBe(true);
    expect(menu.canReplyAll).toBe(false);
    expect(menu.collapsed).toBe(false);

    menu.run('replyAll');
    menu.run('forward');
    menu.run('toggleCollapsed');
    expect(bubble.emitted('replyAll')).toHaveLength(1);
    expect(bubble.emitted('forward')).toHaveLength(1);
    expect(bubble.emitted('toggleCollapsed')).toHaveLength(1);

    bubble.unmount();
    wrapper = null;
    expect(threadMenu.menu.value).toBeNull();
  });

  it('keeps its ⋯ where the reader names it, and reports nothing it does not host', async () => {
    const threadMenu = host('m-9', ['m-1']);
    const bubble = await render(threadMenu);

    expect(moreButton(bubble).exists()).toBe(true);
    expect(threadMenu.menu.value).toBeNull();
  });

  it('offers reply all once someone besides the sender is on it', async () => {
    const threadMenu = host('m-1');
    await render(threadMenu, { cc: [{ name: 'Ana', email: 'ana@example.com' }] });

    expect(threadMenu.menu.value?.canReplyAll).toBe(true);
  });

  it('leaves Show on the notice and moves the always-allow choices into its actions', async () => {
    const threadMenu = host('m-1');
    const bubble = await render(threadMenu);

    const guard = bubble.get('.image-guard');
    expect(guard.find('.image-guard__show').exists()).toBe(true);
    expect(guard.find('.image-guard__choices').exists()).toBe(false);
    const images = threadMenu.menu.value?.images;
    expect(images).toMatchObject({ senderEmail: 'newsletter@metro.de', domain: 'metro.de' });

    images!.allowSender();
    await settle();

    expect(useImagePolicy().isAllowed('newsletter@metro.de')).toBe(true);
    expect(bubble.find('.image-guard').exists()).toBe(false);
    expect(threadMenu.menu.value?.images).toBeNull();
  });

  it('lists the always-allow choices in its own ⋯ sheet too', async () => {
    const bubble = await render(host('m-9', ['m-1']));

    await moreButton(bubble).trigger('click');
    await settle();

    const images = document.body.querySelector('[role="group"][aria-label="Always load images"]');
    expect(images?.textContent).toContain('Allow sender');
    expect(images?.textContent).toContain('Allow domain @metro.de');
  });
});
