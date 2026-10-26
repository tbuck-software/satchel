// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import AvatarUnreadDot from '@/components/ui/AvatarUnreadDot.vue';
import FolderPreviewCard from './FolderPreviewCard.vue';

describe('folder preview totals', () => {
  it('shows two names on one line and counts every other email, read ones included', () => {
    const wrapper = mount(FolderPreviewCard, {
      props: {
        label: 'Newsletter',
        count: 213,
        chips: Array.from({ length: 4 }, (_, index) => ({
          id: `mail-${index}`, unread: index < 2,
        })),
      },
      slots: { chip: '<span>Sender</span>' },
    });

    expect(wrapper.findAll('.folder-chip')).toHaveLength(2);
    expect(wrapper.findAll('.folder-chip-counter')).toHaveLength(1);
    expect(wrapper.get('.folder-chip-counter').text()).toBe('+211');
  });

  it('shows one name at the phone limit', () => {
    const wrapper = mount(FolderPreviewCard, {
      props: {
        label: 'Newsletter',
        count: 213,
        previewLimit: 1,
        chips: Array.from({ length: 4 }, (_, index) => ({ id: `mail-${index}`, unread: false })),
      },
    });

    expect(wrapper.findAll('.folder-chip')).toHaveLength(1);
    expect(wrapper.get('.folder-chip-counter').text()).toBe('+212');
  });

  it('includes emails the server did not send as previews', () => {
    const wrapper = mount(FolderPreviewCard, {
      props: {
        label: 'Notifications', count: 604,
        chips: [{ id: 'mail-1', unread: false }, { id: 'mail-2', unread: false }],
      },
    });
    expect(wrapper.get('.folder-chip-counter').text()).toBe('+602');
  });

  it('shows no remainder when every item fits and keeps unread previews first', () => {
    const wrapper = mount(FolderPreviewCard, {
      props: {
        label: 'Topics', count: 2,
        chips: [{ id: 'read', unread: false }, { id: 'unread', unread: true }],
      },
      slots: { chip: '<template #chip="{ chip }">{{ chip.id }}</template>' },
    });
    expect(wrapper.find('.folder-chip-counter').exists()).toBe(false);
    expect(wrapper.findAll('.folder-chip').map((chip) => chip.text())).toEqual(['unread', 'read']);
  });

  it('keeps bundle unread totals distinct from per-thread avatar dots', () => {
    const wrapper = mount(FolderPreviewCard, {
      props: { label: 'Notifications', count: 4, unreadCount: 2, chips: [] },
    });
    expect(wrapper.findComponent(AvatarUnreadDot).exists()).toBe(false);
    expect(wrapper.get('[title="2 unread"]').text()).toBe('4');
  });

  it('falls back to the supplied items when there is no total', () => {
    const wrapper = mount(FolderPreviewCard, {
      props: {
        label: 'Topics',
        chips: Array.from({ length: 6 }, (_, index) => ({ id: `topic-${index}`, unread: false })),
      },
    });
    expect(wrapper.findAll('.folder-chip')).toHaveLength(2);
    expect(wrapper.get('.folder-chip-counter').text()).toBe('+4');
  });
});
