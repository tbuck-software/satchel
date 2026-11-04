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
  isStarred: false,
  isAside: false,
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

  it('preserves mobile action labels and visibility, and offers the inbox on a done thread', async () => {
    const wrapper = mount(ThreadMobileActions, {
      props: { ...baseProps, isStarred: true, isAside: true, isDone: true },
    });

    expect(hasBar(wrapper, 'More')).toBe(true);
    expect(hasBar(wrapper, 'Reply')).toBe(true);
    expect(hasBar(wrapper, 'Read')).toBe(true);
    expect(hasBar(wrapper, 'Later')).toBe(true);
    expect(wrapper.get('.bar-done').attributes('disabled')).toBeUndefined();
    expect(wrapper.get('.bar-done').text()).toBe('Move to inbox');
    await wrapper.get('.bar-done').trigger('click');
    expect(wrapper.emitted('moveToInbox')).toHaveLength(1);
    expect(wrapper.emitted('archive')).toBeUndefined();

    await bar(wrapper, 'More').trigger('click');
    expect(bar(wrapper, 'More').attributes('aria-haspopup')).toBe('dialog');
    expect(bar(wrapper, 'More').attributes('aria-expanded')).toBe('true');
    expect(wrapper.get('[role="dialog"]').attributes('aria-label')).toBe('More actions');
    expect(wrapper.get('.sheet-panel').text()).toContain('Unpin');
    expect(wrapper.get('.sheet-panel').text()).toContain('Back to the inbox');
    expect(wrapper.get('.sheet-panel').text()).toContain('Forward');
    expect(wrapper.get('.sheet-panel').text()).toContain('Delete');
  });

  it('closes the sheet before emitting an intent and keeps it across resize', async () => {
    const ui = useUiStore();
    const wrapper = mount(ThreadMobileActions, {
      props: baseProps,
      global: { stubs: { Transition: { template: '<slot />' } } },
    });

    await bar(wrapper, 'More').trigger('click');
    expect(ui.overlayDepth).toBe(1);
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);

    await wrapper.setProps({ visible: false });
    expect(wrapper.find('.mobile-bar').exists()).toBe(false);
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);

    ui.isSinglePane = false;
    await nextTick();
    expect(wrapper.find('.sheet-root--anchored').exists()).toBe(true);

    await wrapper.setProps({ visible: true });
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);

    await wrapper.get('.sheet-item').trigger('click');
    await nextTick();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(ui.overlayDepth).toBe(0);
    expect(wrapper.emitted('togglePin')).toEqual([[]]);
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
    expect(['common.more', 'common.reply', 'thread.actions.unread', 'thread.actions.read', 'thread.actions.later', 'common.done'].map(label))
      .toEqual(['Mehr', 'Antworten', 'Ungelesen', 'Gelesen', 'Später', 'Erledigt']);
  });

  it('hides reply actions when replying is unavailable and emits bar intents', async () => {
    const wrapper = mount(ThreadMobileActions, {
      props: { ...baseProps, canReply: false, isUnread: false },
    });

    expect(hasBar(wrapper, 'Reply')).toBe(false);
    expect(hasBar(wrapper, 'Unread')).toBe(true);

    await bar(wrapper, 'Unread').trigger('click');
    await bar(wrapper, 'Later').trigger('click');
    expect(wrapper.emitted('toggleRead')).toEqual([[]]);
    expect(wrapper.emitted('snooze')).toEqual([[]]);

    await bar(wrapper, 'More').trigger('click');
    expect(wrapper.get('.sheet-panel').text()).not.toContain('Forward');
  });

  it('emits the remaining bar and sheet intents and dismisses the overlay', async () => {
    const wrapper = mount(ThreadMobileActions, {
      props: baseProps,
      global: { stubs: { Transition: { template: '<slot />' } } },
    });
    const clickSheetAction = async (label: string) => {
      await bar(wrapper, 'More').trigger('click');
      const action = wrapper.findAll('.sheet-item').find((button) => button.text() === label);
      expect(action).toBeDefined();
      await action!.trigger('click');
      expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    };

    await bar(wrapper, 'Reply').trigger('click');
    await wrapper.get('.bar-done').trigger('click');
    await clickSheetAction('Set aside');
    await clickSheetAction('Forward');
    await clickSheetAction('Delete');

    expect(wrapper.emitted('reply')).toEqual([[]]);
    expect(wrapper.emitted('archive')).toEqual([[]]);
    expect(wrapper.emitted('setAside')).toEqual([[]]);
    expect(wrapper.emitted('forward')).toEqual([[]]);
    expect(wrapper.emitted('delete')).toEqual([[]]);

    await bar(wrapper, 'More').trigger('click');
    await wrapper.get('.sheet-scrim').trigger('click');
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  });

  it('delegates drag dismissal to the shared bottom sheet', async () => {
    const ui = useUiStore();
    const wrapper = mount(ThreadMobileActions, {
      props: baseProps,
      global: { stubs: { Transition: { template: '<slot />' } } },
    });

    await bar(wrapper, 'More').trigger('click');
    const sheet = wrapper.get('[role="dialog"]');

    await sheet.trigger('pointerdown', {
      pointerType: 'mouse', pointerId: 1, button: 0, clientX: 160, clientY: 220,
    });
    await sheet.trigger('pointermove', {
      pointerType: 'mouse', pointerId: 1, clientX: 160, clientY: 320,
    });
    await sheet.trigger('pointerup', {
      pointerType: 'mouse', pointerId: 1, clientX: 160, clientY: 320,
    });

    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(ui.overlayDepth).toBe(0);
  });
});
