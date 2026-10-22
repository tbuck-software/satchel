// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import type { Message } from '@/types/mail';
import { useUiStore } from '@/stores/ui';
import AttachmentChip from './AttachmentChip.vue';
import MessageSecurityBadge from './MessageSecurityBadge.vue';
import MessageBubble from './MessageBubble.vue';
import PgpStatusCard from './PgpStatusCard.vue';

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
  it('offers private key import for an encrypted message with no matching key', async () => {
    const message = signedMessage();
    message.pgp = { state: 'encrypted_no_key' };
    message.bodyText = '-----BEGIN PGP MESSAGE-----\nencrypted\n-----END PGP MESSAGE-----';
    message.attachments = [];
    wrapper = mount(MessageBubble, {
      props: { message, canReply: false },
      global: {
        stubs: {
          EmailRenderer: true,
          InvitationCard: true,
          ItemRenderer: true,
          SenderFilingMenu: true,
          UnsubscribeBanner: true,
        },
      },
    });

    const card = wrapper.getComponent(PgpStatusCard);
    const importButton = card.findAll('button').find((button) => button.text() === 'Import key');
    expect(importButton).toBeDefined();
    await importButton!.trigger('click');

    expect(useUiStore().settingsOpen).toBe(true);
    expect(useUiStore().settingsSection).toBe('pgp');
  });

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
