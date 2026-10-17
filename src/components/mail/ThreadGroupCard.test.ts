// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';

const { archiveMock, setAsideMock, setPinnedMock, trashMock, setThreadsReadMock } = vi.hoisted(() => ({
  archiveMock: vi.fn(),
  setAsideMock: vi.fn(),
  setPinnedMock: vi.fn(),
  trashMock: vi.fn(),
  setThreadsReadMock: vi.fn(),
}));

vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({
    archive: archiveMock,
    setAside: setAsideMock,
    setPinned: setPinnedMock,
    trash: trashMock,
    setRead: setThreadsReadMock,
  }),
}));

vi.mock('@/stores/settings', () => ({
  useSettingsStore: () => ({
    swipeActions: {
      rightShort: 'toggle-read',
      rightLong: 'done',
      leftShort: 'pin',
      leftLong: 'trash',
    },
  }),
}));

vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => ({ setThreadsRead: setThreadsReadMock }),
}));

import ThreadGroupCard from './ThreadGroupCard.vue';

const group = {
  id: 'group-1',
  senderName: 'GitHub',
  senderEmail: 'ci@github.com',
  title: 'CI failures',
  previewText: 'Build broke',
  count: 2,
  unreadCount: 2,
  lastMessageAt: '2026-08-20T10:00:00Z',
  lastMessageAtMs: Date.parse('2026-08-20T10:00:00Z'),
  threads: [
    { id: 't-1' },
    { id: 't-2' },
  ],
} as never;

function render() {
  return mount(ThreadGroupCard, {
    props: { group, targetKey: 'group:group-1' },
  });
}

beforeEach(() => {
  setActivePinia(createPinia());
  archiveMock.mockReset();
  setAsideMock.mockReset();
  setPinnedMock.mockReset();
  trashMock.mockReset();
});

afterEach(() => vi.useRealTimers());

describe('group card actions', () => {
  it('opens the stack from its collection row', async () => {
    const wrapper = render();
    await wrapper.get('[data-list-target-key="group:group-1"]').trigger('click');
    expect(wrapper.emitted('open')?.[0]).toEqual(['group-1']);
  });

  it('previews distinct subjects across the stack and announces them', () => {
    const wrapper = mount(ThreadGroupCard, {
      props: {
        group: {
          ...group as object,
          threads: [
            { id: 't-1', subject: 'Build failed' },
            { id: 't-2', subject: 'Deployment finished' },
            { id: 't-3', subject: 'Build failed' },
          ],
        } as never,
      },
    });
    expect(wrapper.get('.bundle-card__subjects').text()).toBe('Build failed · Deployment finished');
    expect(wrapper.get('.bundle-card__button').attributes('aria-label')).toContain('Build failed · Deployment finished');
  });

  it('labels a CI stack by project and checks off all of its members', async () => {
    const wrapper = mount(ThreadGroupCard, {
      props: {
        group: { ...group as object, senderName: 'Alex Morgan', ciProject: 'tbuck-software/satchel' } as never,
      },
    });
    expect(wrapper.get('.bundle-card__sender').text()).toBe('tbuck-software/satchel · CI');
    expect(wrapper.get('.bundle-card__button').attributes('aria-label')).toContain('tbuck-software/satchel · CI');
    await wrapper.get('.bundle-card__meta button').trigger('click');
    expect(wrapper.emitted('done')?.[0]).toEqual([['t-1', 't-2']]);
  });

  it('files every thread of the stack from the outermost check button', async () => {
    const wrapper = render();
    await wrapper.get('.bundle-card__meta button').trigger('click');
    expect(wrapper.emitted('done')?.[0]).toEqual([['t-1', 't-2']]);
  });

  it('drags the whole stack as one payload for folder drops', async () => {
    const setData = vi.fn();
    const wrapper = render();
    await wrapper.get('.bundle-card').trigger('dragstart', {
      dataTransfer: { setData, effectAllowed: '' },
    });
    expect(setData).toHaveBeenCalledWith('application/x-satchel-thread-ids', 't-1,t-2');
  });

  it('commits a long right swipe as done for the whole stack', async () => {
    vi.useFakeTimers();
    const wrapper = render();
    const card = wrapper.get('.bundle-card');

    await card.trigger('pointerdown', {
      pointerId: 1, pointerType: 'touch', clientX: 20, clientY: 40,
    });
    await card.trigger('pointermove', {
      pointerId: 1, pointerType: 'touch', clientX: 190, clientY: 42,
    });
    expect(wrapper.get('[data-swipe-action="done"]').text()).toContain('Done');

    await card.trigger('pointerup', {
      pointerId: 1, pointerType: 'touch', clientX: 190, clientY: 42,
    });
    await vi.advanceTimersByTimeAsync(120);

    expect(archiveMock).toHaveBeenCalledTimes(1);
    expect(archiveMock).toHaveBeenCalledWith(['t-1', 't-2']);
  });

  it('marks the whole stack read with the default short right swipe', async () => {
    vi.useFakeTimers();
    const wrapper = render();
    const card = wrapper.get('.bundle-card');

    await card.trigger('pointerdown', {
      pointerId: 2, pointerType: 'touch', clientX: 20, clientY: 40,
    });
    await card.trigger('pointermove', {
      pointerId: 2, pointerType: 'touch', clientX: 108, clientY: 42,
    });
    await card.trigger('pointerup', {
      pointerId: 2, pointerType: 'touch', clientX: 108, clientY: 42,
    });
    await vi.advanceTimersByTimeAsync(120);

    expect(setThreadsReadMock).toHaveBeenCalledWith(['t-1', 't-2'], true);
  });

  it('does not open the stack when releasing a swipe over the card', async () => {
    vi.useFakeTimers();
    const wrapper = render();
    const card = wrapper.get('.bundle-card');

    await card.trigger('pointerdown', {
      pointerId: 3, pointerType: 'touch', clientX: 200, clientY: 40,
    });
    await card.trigger('pointermove', {
      pointerId: 3, pointerType: 'touch', clientX: 30, clientY: 42,
    });
    await card.get('button:not(.bundle-card__done)').trigger('click');
    expect(wrapper.emitted('open')).toBeUndefined();

    await card.trigger('pointerup', {
      pointerId: 3, pointerType: 'touch', clientX: 30, clientY: 42,
    });
    await vi.advanceTimersByTimeAsync(120);
    expect(trashMock).toHaveBeenCalledWith(['t-1', 't-2']);
    expect(wrapper.emitted('open')).toBeUndefined();
  });
});
