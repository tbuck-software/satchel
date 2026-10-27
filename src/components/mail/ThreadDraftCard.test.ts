// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import type { Message } from '@/types/mail';
import ThreadDraftCard from './ThreadDraftCard.vue';

function draft(overrides: Partial<Message> = {}): Message {
  return {
    id: 'draft-1',
    threadId: 'thread-1',
    from: { name: 'Tom', email: 'tom@example.com' },
    to: [{ name: 'Sarah Kim', email: 'sarah@example.com' }],
    subject: 'Re: Staging',
    bodyText: 'The fix is on staging.\n\nOn Tue, Sarah Kim wrote:\n> Users cannot sign in',
    timestamp: '2026-10-27T08:46:00Z',
    isRead: true,
    isDraft: true,
    attachments: [],
    ...overrides,
  };
}

function render(props: Partial<InstanceType<typeof ThreadDraftCard>['$props']> = {}) {
  return mount(ThreadDraftCard, {
    props: { message: draft(), recipient: 'Sarah Kim', identity: 'Work', ...props },
  });
}

describe('ThreadDraftCard', () => {
  it('names the draft, its addressee and what is written, without the quoted history', () => {
    const wrapper = render();

    expect(wrapper.get('.draft-card__label').text()).toBe('Draft · not sent');
    expect(wrapper.get('.draft-card__recipient').text()).toBe('To Sarah Kim');
    expect(wrapper.get('.draft-card__identity').text()).toBe('as Work');
    expect(wrapper.get('.draft-card__saved').text()).toMatch(/^Last saved /);
    expect(wrapper.get('.draft-card__preview').text()).toBe('The fix is on staging.');
    expect(wrapper.get('article').attributes('data-thread-message-id')).toBe('draft-1');
  });

  it('says so when nothing is written yet', () => {
    const wrapper = render({ message: draft({ bodyText: '' }) });
    expect(wrapper.get('.draft-card__preview').text()).toBe('No text yet');
    expect(wrapper.get('.draft-card__preview').classes()).toContain('draft-card__preview--empty');
  });

  it('continues editing from the card, its button and the empty stretch beside them', async () => {
    const wrapper = render();

    await wrapper.get('.draft-card__body').trigger('click');
    await wrapper.findAll('button').find((button) => button.text() === 'Continue editing')!.trigger('click');
    await wrapper.get('.draft-card__actions').trigger('click');
    expect(wrapper.emitted('edit')).toHaveLength(3);
    expect(wrapper.get('.draft-card__body').attributes('aria-label')).toBe('Continue your draft to Sarah Kim');
  });

  it('discards once, and not while a discard is running', async () => {
    const wrapper = render();
    const discard = () => wrapper.findAll('button').find((button) => button.text() === 'Discard')!;

    await discard().trigger('click');
    expect(wrapper.emitted('discard')).toHaveLength(1);
    expect(wrapper.emitted('edit')).toBeUndefined();

    await wrapper.setProps({ busy: true });
    expect(discard().attributes('disabled')).toBeDefined();
  });
});
