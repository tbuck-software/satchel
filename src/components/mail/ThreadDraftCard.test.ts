// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import type { Message } from '@/types/mail';
import { toastObstacleRects } from '@/composables/useToastClearance';
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

  it('offers recovery instead of opening or previewing an unavailable draft', async () => {
    const wrapper = render({ message: draft({
      bodyText: 'stale draft text',
      contentError: JSON.stringify({ key: 'backend.errors.imapUidValidityChanged', params: { folder: 'Work/Drafts' } }),
    }) });
    try {
      expect(wrapper.get('[role="alert"]').text()).toContain('Work/Drafts');
      expect(wrapper.text()).toContain('Sync the folder');
      expect(wrapper.text()).not.toContain('stale draft text');
      expect(wrapper.find('.draft-card__body').exists()).toBe(false);
      const edit = wrapper.findAll('button').find(button => button.text() === 'Continue editing')!;
      expect(edit.attributes('disabled')).toBeDefined();
      await edit.trigger('click');
      await wrapper.get('.draft-card__actions').trigger('click');
      expect(wrapper.emitted('edit')).toBeUndefined();
      await wrapper.get('.error-state button').trigger('click');
      expect(wrapper.emitted('retry')).toHaveLength(1);
      await wrapper.setProps({ message: draft() });
      await wrapper.get('.draft-card__body').trigger('click');
      expect(wrapper.emitted('edit')).toHaveLength(1);
    } finally {
      wrapper.unmount();
    }
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

  /*
   * The card replaces the reply bar at the foot of the reader, in the corner
   * the toast stack starts in. At 900x700 the stack lay on Continue editing.
   */
  it('keeps the toast stack off itself while it is mounted', () => {
    const rect = { top: 540, right: 860, bottom: 700, left: 280, width: 580, height: 160, x: 280, y: 540 };
    const spy = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect')
      .mockImplementation(function (this: HTMLElement) {
        return (this.classList.contains('draft-card') ? rect : { ...rect, width: 0, height: 0 }) as DOMRect;
      });
    // Cards the tests above mounted are still registered.
    const before = toastObstacleRects().length;
    const wrapper = render();
    expect(toastObstacleRects()).toHaveLength(before + 1);
    expect(toastObstacleRects()).toContainEqual(rect);
    wrapper.unmount();
    expect(toastObstacleRects()).toHaveLength(before);
    spy.mockRestore();
  });
});
