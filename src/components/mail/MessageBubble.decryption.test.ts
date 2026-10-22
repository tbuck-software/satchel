// @vitest-environment jsdom
import { defineComponent, h } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { expect, it, vi } from 'vitest';
import type { Message } from '@/types/mail';

const backend = vi.hoisted(() => ({
  unlock: vi.fn(async () => ({ unlocked: 1, key_ids: ['private-key'] })),
  bodies: vi.fn(async () => [{
    id: 'encrypted-message', snippet: 'Readable mail', body_text: 'Readable mail',
    body_html: null, body_loaded: true, pgp_state: 'decrypted',
  }]),
}));

vi.mock('@/services/backend', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/services/backend')>(),
  backendUnlockPgpKeys: backend.unlock,
  backendListMessageBodies: backend.bodies,
  backendListAttachmentsForMessages: vi.fn(async () => []),
}));

import { useMessagesStore } from '@/stores/messages';
import MessageBubble from './MessageBubble.vue';

it('decrypts a cached message from its button even when the plugin supplied no PGP status', async () => {
  setActivePinia(createPinia());
  const messages = useMessagesStore();
  const message: Message = {
    id: 'encrypted-message', threadId: 'thread', subject: 'Encrypted mail',
    from: { name: 'Alex', email: 'alex@example.test' },
    to: [{ name: 'Sam', email: 'sam@example.test' }],
    bodyText: '-----BEGIN PGP MESSAGE-----\nciphertext\n-----END PGP MESSAGE-----',
    bodyLoaded: true, timestamp: '2026-10-22T08:13:00Z', isRead: true, attachments: [],
  };
  messages.messagesByThread.thread = [message];
  const wrapper = mount(defineComponent({
    setup: () => () => h(MessageBubble, { message: messages.getByThread('thread')[0]! }),
  }));

  await wrapper.get('.pgp-segment-action').trigger('click');
  await flushPromises();

  expect(backend.unlock).toHaveBeenCalledOnce();
  expect(backend.bodies).toHaveBeenCalledWith(['encrypted-message']);
  expect(wrapper.text()).toContain('Readable mail');
  expect(wrapper.text()).not.toContain('BEGIN PGP MESSAGE');
  expect(wrapper.find('.pgp-segment-action').exists()).toBe(false);
  wrapper.unmount();
});
