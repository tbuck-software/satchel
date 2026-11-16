// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { enableAutoUnmount, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useUiStore } from '@/stores/ui';
import ThreadMenuSheet from './ThreadMenuSheet.vue';
import type { MessageMenu } from './threadMessageMenu';

enableAutoUnmount(afterEach);

const transition = { stubs: { Transition: { template: '<slot />' } } };

function messageMenu(overrides: Partial<MessageMenu> = {}): MessageMenu {
  return {
    canReply: true,
    canReplyAll: false,
    collapsed: false,
    bodyView: null,
    images: null,
    translation: null,
    run: vi.fn(),
    ...overrides,
  };
}

function render(message: MessageMenu | null, props: { isStarred?: boolean; isAside?: boolean } = {}) {
  return mount(ThreadMenuSheet, {
    props: { open: true, isStarred: false, isAside: false, message, ...props },
    global: transition,
  });
}

function group(wrapper: ReturnType<typeof render>, label: string) {
  return wrapper.get(`.thread-menu > [role="group"][aria-label="${label}"]`);
}

function rows(wrapper: ReturnType<typeof render>, label: string) {
  return group(wrapper, label).findAll('.menu-item').map((row) => row.text());
}

function row(wrapper: ReturnType<typeof render>, text: string) {
  const found = wrapper.findAll('.menu-item').find((item) => item.text() === text);
  if (!found) throw new Error(`no row ${text}`);
  return found;
}

describe('ThreadMenuSheet', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    useUiStore().isSinglePane = true;
  });

  it('holds the thread actions the bottom bar\'s More used to, under their own heading', () => {
    const wrapper = render(null, { isStarred: true, isAside: true });

    expect(wrapper.get('[role="dialog"]').attributes('aria-label')).toBe('More actions');
    expect(wrapper.findAll('.thread-menu__heading').map((heading) => heading.text())).toEqual(['This conversation']);
    expect(rows(wrapper, 'This conversation')).toEqual(['Unpin', 'Back to the inbox', 'Delete']);
    expect(row(wrapper, 'Delete').classes()).toContain('menu-item--danger');
  });

  it('adds the newest message\'s actions, without the Reply the bottom bar holds', () => {
    const wrapper = render(messageMenu({ canReplyAll: true }));

    expect(wrapper.findAll('.thread-menu__heading').map((heading) => heading.text()))
      .toEqual(['This conversation', 'Latest message']);
    expect(rows(wrapper, 'This conversation')).toEqual(['Pin', 'Set aside', 'Delete']);
    expect(rows(wrapper, 'Latest message'))
      .toEqual(['Reply all', 'Forward', 'Add to topic', 'Show source', 'Collapse message']);
    // Forward once: the thread group no longer has its own.
    expect(wrapper.findAll('.menu-item').filter((item) => item.text() === 'Forward')).toHaveLength(1);
  });

  it('leaves out reply all for a two-person conversation and the reply commands where replying is off', () => {
    expect(rows(render(messageMenu()), 'Latest message'))
      .toEqual(['Forward', 'Add to topic', 'Show source', 'Collapse message']);
    expect(rows(render(messageMenu({ canReply: false, canReplyAll: true })), 'Latest message'))
      .toEqual(['Add to topic', 'Show source', 'Collapse message']);
  });

  it('offers Show as and the always-allow choices of the newest message', () => {
    const setMode = vi.fn();
    const allowSender = vi.fn();
    const allowDomain = vi.fn();
    const wrapper = render(messageMenu({
      bodyView: {
        options: [{ value: 'html', label: 'Original' }, { value: 'text', label: 'Text' }],
        mode: 'html',
        defaultMode: 'html',
        set: setMode,
      },
      images: { senderEmail: 'newsletter@metro.de', domain: 'metro.de', allowSender, allowDomain },
    }));

    const latest = group(wrapper, 'Latest message');
    const showAs = latest.get('[role="group"][aria-label="Show as"]');
    expect(showAs.findAll('[role="menuitemradio"]').map((choice) => [choice.text(), choice.attributes('aria-checked')]))
      .toEqual([['Original', 'true'], ['Text', 'false']]);
    const images = latest.get('[role="group"][aria-label="Always load images"]');
    expect(images.text()).toContain('Allow sender');
    expect(images.text()).toContain('newsletter@metro.de');
    expect(images.text()).toContain('Allow domain @metro.de');

    showAs.findAll('[role="menuitemradio"]')[1]!.element.dispatchEvent(new MouseEvent('click'));
    expect(setMode).toHaveBeenCalledWith('text');
    images.findAll('.menu-item')[1]!.element.dispatchEvent(new MouseEvent('click'));
    expect(allowDomain).toHaveBeenCalledTimes(1);
    expect(allowSender).not.toHaveBeenCalled();
  });

  it('closes before it runs an action', async () => {
    const run = vi.fn();
    const wrapper = render(messageMenu({ run }));

    await row(wrapper, 'Show source').trigger('click');
    expect(wrapper.emitted('dismiss')).toHaveLength(1);
    expect(run).toHaveBeenCalledWith('source');

    await row(wrapper, 'Set aside').trigger('click');
    await row(wrapper, 'Pin').trigger('click');
    await row(wrapper, 'Delete').trigger('click');
    expect(wrapper.emitted('dismiss')).toHaveLength(4);
    expect(wrapper.emitted('setAside')).toEqual([[]]);
    expect(wrapper.emitted('togglePin')).toEqual([[]]);
    expect(wrapper.emitted('delete')).toEqual([[]]);
  });
});
