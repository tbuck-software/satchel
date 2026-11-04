// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { enableAutoUnmount, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useUiStore } from '@/stores/ui';
import de from '@/i18n/de.json';
import ThreadMobileActions from './ThreadMobileActions.vue';

enableAutoUnmount(afterEach);

type Wrapper = ReturnType<typeof mount>;

/** A bar action by its visible label, which is also its accessible name. */
function findBar(wrapper: Wrapper, label: string) {
  return wrapper.findAll('.bar-btn').find((cell) => cell.get('.toolbar-cell__label').text() === label);
}

function bar(wrapper: Wrapper, label: string) {
  const cell = findBar(wrapper, label);
  if (!cell) throw new Error(`no bar action labelled ${label}`);
  return cell;
}

function hasBar(wrapper: Wrapper, label: string) {
  return findBar(wrapper, label) !== undefined;
}

const baseProps = {
  visible: true,
  canReply: true,
  isUnread: true,
  isDone: false,
};

describe('ThreadMobileActions', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    useUiStore().isSinglePane = true;
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('keeps Reply, Read and Later beside the pill, and offers the inbox on a done thread', async () => {
    const wrapper = mount(ThreadMobileActions, {
      props: { ...baseProps, isDone: true },
    });

    expect(hasBar(wrapper, 'Reply')).toBe(true);
    expect(hasBar(wrapper, 'Read')).toBe(true);
    expect(hasBar(wrapper, 'Later')).toBe(true);
    expect(wrapper.get('.bar-done').attributes('disabled')).toBeUndefined();
    expect(wrapper.get('.bar-done').text()).toBe('Move to inbox');
    await wrapper.get('.bar-done').trigger('click');
    expect(wrapper.emitted('moveToInbox')).toHaveLength(1);
    expect(wrapper.emitted('archive')).toBeUndefined();
  });

  it('opens the thread\'s one menu from More, under the thumb', async () => {
    const wrapper = mount(ThreadMobileActions, { props: baseProps });

    const more = bar(wrapper, 'More');
    expect(more.attributes('aria-haspopup')).toBe('dialog');
    expect(more.attributes('aria-expanded')).toBe('false');
    // The menu itself belongs to the thread view; the bar only asks for it.
    expect(document.body.querySelector('.sheet-panel')).toBeNull();
    await more.trigger('click');
    expect(wrapper.emitted('more')).toHaveLength(1);
  });

  it('labels the four actions under their icons beside the Done pill', () => {
    const wrapper = mount(ThreadMobileActions, { props: baseProps });

    // Four whole-cell buttons, icon over a short label…
    const cells = wrapper.findAll('.mobile-bar-actions > .toolbar-cell');
    expect(cells.map((cell) => cell.get('.toolbar-cell__label').text()))
      .toEqual(['More', 'Reply', 'Read', 'Later']);
    for (const cell of cells) {
      expect(cell.element.tagName).toBe('BUTTON');
      expect(cell.find('.toolbar-cell__icon svg').exists()).toBe(true);
      // The visible label is the accessible name; no aria-label overrides it.
      expect(cell.attributes('aria-label')).toBeUndefined();
    }
    // …and Done stays the prominent pill, outside the cells.
    const done = wrapper.get('.mobile-bar > .bar-done');
    expect(done.classes()).toEqual(expect.arrayContaining(['app-button', 'app-button--primary']));
    expect(done.text()).toBe('Done');
    // A short label keeps the long phrase as its title.
    expect(bar(wrapper, 'Read').attributes('title')).toBe('Mark as read');
    expect(bar(wrapper, 'Later').attributes('title')).toBe('Snooze');
  });

  it('has short German labels for every cell', () => {
    // Tests run in English; the catalogue says what a German phone shows.
    const label = (key: string) => key.split('.').reduce<unknown>(
      (node, part) => (node as Record<string, unknown> | undefined)?.[part],
      de,
    );
    expect(['common.reply', 'thread.actions.unread', 'thread.actions.read', 'thread.actions.later', 'common.done'].map(label))
      .toEqual(['Antworten', 'Ungelesen', 'Gelesen', 'Später', 'Erledigt']);
  });

  it('hides Reply when replying is unavailable and emits the bar intents', async () => {
    const wrapper = mount(ThreadMobileActions, {
      props: { ...baseProps, canReply: false, isUnread: false },
    });

    expect(hasBar(wrapper, 'Reply')).toBe(false);
    expect(hasBar(wrapper, 'Unread')).toBe(true);

    await bar(wrapper, 'Unread').trigger('click');
    await bar(wrapper, 'Later').trigger('click');
    await wrapper.get('.bar-done').trigger('click');
    expect(wrapper.emitted('toggleRead')).toEqual([[]]);
    expect(wrapper.emitted('snooze')).toEqual([[]]);
    expect(wrapper.emitted('archive')).toEqual([[]]);
  });

  it('emits Reply and hides with the viewport', async () => {
    const wrapper = mount(ThreadMobileActions, { props: baseProps });

    await bar(wrapper, 'Reply').trigger('click');
    expect(wrapper.emitted('reply')).toEqual([[]]);

    await wrapper.setProps({ visible: false });
    await nextTick();
    expect(wrapper.find('.mobile-bar').exists()).toBe(false);
  });
});
