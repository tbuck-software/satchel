// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import type { Message } from '@/types/mail';
import { useUiStore } from '@/stores/ui';
import AttachmentChip from './AttachmentChip.vue';
import MessageSecurityBadge from './MessageSecurityBadge.vue';
import MessageBubble from './MessageBubble.vue';

let wrapper: VueWrapper | null = null;

function signedMessage(): Message {
  return {
    id: 'signed-message',
    threadId: 'thread-1',
    from: { name: 'GitLab', email: 'gitlab@code.example' },
    to: [{ name: 'Alex', email: 'alex@example.com' }],
    subject: 'Pipeline status',
    bodyText: 'Pipeline passed',
    bodyLoaded: true,
    timestamp: '2026-09-03T10:00:03Z',
    isRead: true,
    authDkim: 'pass',
    attachments: [{
      id: 'signature',
      filename: 'smime.p7s',
      contentType: 'application/x-pkcs7-signature',
      size: 2523,
      isInline: false,
    }],
  };
}

beforeEach(() => {
  setActivePinia(createPinia());
  useUiStore().isSinglePane = true;
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  document.body.innerHTML = '';
});

describe('message security affordances', () => {
  it('moves a detached S/MIME signature from attachments into sender details', async () => {
    wrapper = mount(MessageBubble, {
      attachTo: document.body,
      props: { message: signedMessage(), canReply: false },
      global: {
        stubs: {
          EmailRenderer: true,
          InvitationCard: true,
          ItemRenderer: true,
          PgpStatusCard: true,
          SenderFilingMenu: true,
          SmimeStatusCard: true,
          UnsubscribeBanner: true,
        },
      },
    });

    expect(wrapper.findAllComponents(AttachmentChip)).toHaveLength(0);
    const badge = wrapper.getComponent(MessageSecurityBadge);
    expect(badge.props('smimeState')).toBe('signature_unverified');
    expect(badge.props('authentication')).toBe('pass');

    await wrapper.get('.message-sender-email').trigger('click');
    await flushPromises();

    const details = document.body.querySelector('.recipient-details');
    expect(details?.textContent).toContain('S/MIME signature detected');
    expect(details?.textContent).toContain('The signature has not been verified yet.');
    expect(details?.textContent).toContain('DKIM');
    expect(details?.textContent).toContain('Passed');
  });
});
