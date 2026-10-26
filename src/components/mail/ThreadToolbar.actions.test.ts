// @vitest-environment jsdom
import { mount, type VueWrapper } from '@vue/test-utils';
import { afterEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { useUiStore } from '@/stores/ui';
import ThreadToolbar from './ThreadToolbar.vue';

const wrappers: VueWrapper[] = [];

afterEach(() => {
  while (wrappers.length > 0) wrappers.pop()?.unmount();
});

function render(overrides: Record<string, unknown> = {}) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const wrapper = mount(ThreadToolbar, {
    props: {
      threadId: 'thread-1',
      subject: 'Betreff',
      canReply: false,
      ...overrides,
    },
    attachTo: document.body,
    global: { plugins: [pinia], stubs: { PluginSlotHost: true, Teleport: true } },
  });
  wrappers.push(wrapper);
  return wrapper;
}

/** Lets the menu place itself and move focus, which happens a tick after opening. */
async function settle() {
  for (let index = 0; index < 4; index += 1) await nextTick();
}

function labelOf(item: ReturnType<VueWrapper['find']>) {
  const label = item.find('.menu-item__label');
  return (label.exists() ? label.text() : item.attributes('aria-label') ?? '').replace(/\s+/g, ' ').trim();
}

/** Menu rows by their accessible name: role and visible label, nothing else. */
function menuItem(wrapper: VueWrapper, name: string) {
  return wrapper
    .findAll('[role="menuitem"]')
    .find((item) => labelOf(item).startsWith(name));
}

async function openMenu(wrapper: VueWrapper) {
  const trigger = wrapper.get('button[title="More actions"]');
  (trigger.element as HTMLElement).focus();
  await trigger.trigger('click');
}

describe('thread toolbar filing actions', () => {
  it('keeps pinning and set-aside in the three-dot menu instead of direct buttons', async () => {
    const wrapper = render();

    expect(wrapper.find('button[title="Pin"]').exists()).toBe(false);
    expect(wrapper.find('button[title="Set aside"]').exists()).toBe(false);

    await openMenu(wrapper);
    await menuItem(wrapper, 'Pin')!.trigger('click');
    await openMenu(wrapper);
    await menuItem(wrapper, 'Set aside')!.trigger('click');

    expect(wrapper.emitted('togglePin')).toHaveLength(1);
    expect(wrapper.emitted('setAside')).toHaveLength(1);
    expect(wrapper.text()).not.toContain('Star');
    expect(wrapper.text()).not.toContain('Favourite');
  });

  it('uses inverse labels in the menu for pinned and set-aside threads', async () => {
    const wrapper = render({ isStarred: true, isAside: true });

    await openMenu(wrapper);
    expect(wrapper.text()).toContain('Unpin');
    expect(wrapper.text()).toContain('Back to the inbox');
  });

  it('turns the primary action into its inverse on a done thread', async () => {
    const open = render();
    await open.get('.thread-toolbar__done').trigger('click');
    expect(open.get('.thread-toolbar__done').text()).toContain('Done');
    expect(open.emitted('archive')).toHaveLength(1);

    const done = render({ isDone: true });
    const primary = done.get('.thread-toolbar__done');
    expect(primary.text()).toContain('Move to inbox');
    await primary.trigger('click');
    expect(done.emitted('moveToInbox')).toHaveLength(1);
    expect(done.emitted('archive')).toBeUndefined();
  });

  it('leaves Reply to the reply foot and the menu when the reader has one', async () => {
    const plain = render({ canReply: true });
    expect(plain.find('.thread-toolbar button[title^="Reply"]').exists()).toBe(true);

    const docked = render({ canReply: true, replyInFoot: true });
    expect(docked.find('.thread-toolbar button[title^="Reply"]').exists()).toBe(false);
    await openMenu(docked);
    const item = menuItem(docked, 'Reply');
    expect(item, 'no Reply in the menu').toBeTruthy();
    await item!.trigger('click');
    expect(docked.emitted('reply')).toHaveLength(1);
  });
});

/*
 * UI-14 moved 33 hand-rolled rows onto MenuItem. Every action has to still be
 * reachable by role and name, and still fire its own event — this is the net
 * under that rewrite.
 */
describe('thread toolbar menu actions', () => {
  const direct: Array<[string, string, unknown[]]> = [
    ['Reply', 'reply', []],
    ['Send again', 'resend', []],
    ['Forward', 'forward', []],
    ['Mark as read', 'toggleRead', []],
    ['Mute', 'toggleMuted', []],
    ['Mark as spam', 'spam', []],
    ['Copy link', 'copyLink', []],
    ['Save all attachments (2)', 'saveAttachments', []],
    ['All actions', 'allActions', []],
  ];

  it.each(direct)('"%s" fires %s', async (name, event, payload) => {
    const wrapper = render({ canReply: true, isUnread: true, attachmentCount: 2 });

    await openMenu(wrapper);
    const item = menuItem(wrapper, name);
    expect(item, `no menu item named ${name}`).toBeTruthy();
    await item!.trigger('click');

    expect(wrapper.emitted(event)).toEqual([payload]);
  });

  const nested: Array<[string, string, string, unknown[]]> = [
    ['Organise', 'Add to topic', 'addToTopic', []],
    ['Organise', 'Edit tags', 'organize', ['tags']],
    ['Block', 'This address', 'block', ['address']],
    ['Block', 'Whole domain', 'block', ['domain']],
    ['Move', 'Archive', 'move', ['folder-archive']],
    ['Print', 'Latest email only', 'print', ['latest']],
    ['Copy as Markdown', 'Copy conversation', 'copyMarkdown', ['conversation']],
    ['Export to', 'Latest email', 'export', ['eml']],
  ];

  it.each(nested)('"%s › %s" fires %s', async (parent, name, event, payload) => {
    const wrapper = render({ moveTargets: [{ id: 'folder-archive', label: 'Archive' }] });

    await openMenu(wrapper);
    const opener = menuItem(wrapper, parent);
    expect(opener, `no menu item named ${parent}`).toBeTruthy();
    expect(opener!.attributes('aria-haspopup')).toBe('menu');
    await opener!.trigger('click');
    expect(opener!.attributes('aria-expanded')).toBe('true');

    const submenu = wrapper.get('.context-menu--nested');
    const item = submenu
      .findAll('[role="menuitem"]')
      .find((entry) => labelOf(entry).startsWith(name));
    expect(item, `no submenu item named ${name}`).toBeTruthy();
    await item!.trigger('click');

    expect(wrapper.emitted(event)).toEqual([payload]);
  });

});

describe('thread toolbar menu keyboard', () => {
  it('moves focus into the menu and walks it with the arrow keys', async () => {
    const wrapper = render({ canReply: true });

    await openMenu(wrapper);
    await settle();
    expect(document.activeElement?.textContent).toContain('Reply');

    await wrapper.get('.context-menu').trigger('keydown', { key: 'ArrowDown' });
    expect(document.activeElement?.textContent).toContain('Send again');
  });

  it('opens a flyout with the right arrow and focuses its first item', async () => {
    const wrapper = render();

    await openMenu(wrapper);
    await settle();
    const print = menuItem(wrapper, 'Print')!;
    (print.element as HTMLElement).focus();
    await wrapper.get('.context-menu').trigger('keydown', { key: 'ArrowRight' });
    await settle();

    expect(print.attributes('aria-expanded')).toBe('true');
    expect(document.activeElement?.textContent).toContain('Latest email only');

    await wrapper.get('.context-menu--nested').trigger('keydown', { key: 'ArrowLeft' });
    await settle();
    expect(wrapper.find('.context-menu--nested').exists()).toBe(false);
    expect(document.activeElement).toBe(print.element);
  });

  /** Escape used to close the menu and then reach the global handler, which went back. */
  it('closes on Escape without letting the key reach the window', async () => {
    const wrapper = render();
    let reachedWindow = false;
    const listener = () => { reachedWindow = true; };
    window.addEventListener('keydown', listener);

    await openMenu(wrapper);
    await settle();
    await wrapper.get('.context-menu').trigger('keydown', { key: 'Escape' });
    window.removeEventListener('keydown', listener);
    await settle();

    expect(reachedWindow).toBe(false);
    expect(wrapper.find('.context-menu').exists()).toBe(false);
    expect(document.activeElement).toBe(wrapper.get('button[title="More actions"]').element);
  });

  /** Registered, so the global handler sees a covered page and Back closes the menu. */
  it('counts as an overlay while open', async () => {
    const wrapper = render();
    const ui = useUiStore();

    await openMenu(wrapper);
    expect(ui.modalOpen).toBe(true);

    expect(ui.dismissTopmostOverlay()).toBe(true);
    await settle();
    expect(ui.modalOpen).toBe(false);
    expect(wrapper.find('.context-menu').exists()).toBe(false);
  });
});
