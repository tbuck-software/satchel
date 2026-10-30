// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { expect, it, vi } from 'vitest';
import type { Message } from '@/types/mail';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { clearPreparedEmailBodies } from '@/utils/preparedEmailBody';
import MessageBubble from './MessageBubble.vue';

it.each(['imapUidValidityChanged', 'imapUidValidityMissing'])('shows the folder recovery guidance for %s', (key) => {
  setActivePinia(createPinia());
  const message: Message = {
    id: 'message', threadId: 'thread', subject: 'Message',
    from: { name: 'Alex', email: 'alex@example.test' },
    to: [{ name: 'Sam', email: 'sam@example.test' }],
    bodyText: '', bodyLoaded: true,
    bodyLoadError: backendErrorLabel({ key: `backend.errors.${key}`, params: { folder: 'INBOX' } }),
    timestamp: '2026-10-22T08:13:00Z', isRead: true, attachments: [],
  };
  const wrapper = mount(MessageBubble, { props: { message } });
  try {
    const alert = wrapper.get('[role="alert"]');
    expect(alert.text()).toContain('INBOX');
    expect(alert.text()).toMatch(/sync.*folder/i);
  } finally {
    wrapper.unmount();
  }
});

it('renders fresh inline bytes after a generation change with the same local ID and image length', async () => {
  setActivePinia(createPinia());
  clearPreparedEmailBodies();
  let nextUrl = 0;
  const created = vi.spyOn(URL, 'createObjectURL').mockImplementation(() => `blob:generation/${nextUrl++}`);
  const message: Message = {
    id: 'same-message', threadId: 'thread', subject: 'Message', remoteUidValidity: 7,
    from: { name: 'Alex', email: 'alex@example.test' },
    to: [{ name: 'Sam', email: 'sam@example.test' }],
    bodyText: 'Body', bodyHtml: '<p>Body</p><img src="cid:logo@host">', bodyLoaded: true,
    timestamp: '2026-10-22T08:13:00Z', isRead: true,
    attachments: [{ id: 'same-part', filename: 'logo.png', contentType: 'image/png', size: 3,
      contentId: 'logo@host', isInline: true, dataUrl: 'data:image/png;base64,b2xk' }],
  };
  const wrapper = mount(MessageBubble, { props: { message } });
  try {
    expect(wrapper.get('iframe').attributes('srcdoc')).toContain('blob:generation/0');
    await wrapper.setProps({ message: { ...message, remoteUidValidity: 8,
      attachments: [{ ...message.attachments[0]!, dataUrl: 'data:image/png;base64,bmV3' }] } });
    expect(wrapper.get('iframe').attributes('srcdoc')).toContain('blob:generation/1');
    expect(created).toHaveBeenCalledTimes(2);
  } finally {
    wrapper.unmount();
    clearPreparedEmailBodies();
    created.mockRestore();
  }
});
