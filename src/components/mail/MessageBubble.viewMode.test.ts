// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import type { Message } from '@/types/mail';
import { useSettingsStore } from '@/stores/settings';
import { useUiStore } from '@/stores/ui';
import MessageBubble from './MessageBubble.vue';

/*
 * The body's Original / Auto-dark / Text switch. The desktop keeps the
 * segmented control above the body. On the phone it moved into the message's
 * ⋯ sheet as a "Show as" choice; above the body only a muted line remains,
 * and only while the mail is not shown the way it opened.
 */

let wrapper: VueWrapper | null = null;

function message(): Message {
  return {
    id: 'm-1',
    threadId: 'thread-1',
    from: { name: 'METRO', email: 'newsletter@metro.example' },
    to: [{ name: 'Tom', email: 'tom@example.com' }],
    subject: 'Angebote',
    bodyText: '',
    bodyHtml: '<table width="600"><tr><td><b>Wochenangebote</b> für Büro</td></tr></table>',
    bodyLoaded: true,
    timestamp: '2026-09-03T10:00:03Z',
    isRead: true,
    attachments: [],
  };
}

function render() {
  return mount(MessageBubble, {
    attachTo: document.body,
    props: { message: message(), canReply: true },
    global: { stubs: { SenderFilingMenu: true } },
  });
}

/** The sheet's rows load with the first sheet that opens. */
async function openActions(target: VueWrapper) {
  await target.get('.message-header [aria-label="Message actions"]').trigger('click');
  await flushPromises();
  await vi.dynamicImportSettled();
  await flushPromises();
}

function sheetChoices(): HTMLButtonElement[] {
  return Array.from(document.body.querySelectorAll<HTMLButtonElement>('[role="group"][aria-label="Show as"] [role="menuitemradio"]'));
}

beforeEach(() => {
  setActivePinia(createPinia());
  useSettingsStore().theme = 'light';
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  vi.unstubAllGlobals();
  document.body.innerHTML = '';
});

describe('the render-mode switch', () => {
  it('stays a segmented control above the body on the desktop', async () => {
    useUiStore().isMobile = false;
    wrapper = render();
    await flushPromises();

    const segmented = wrapper.get('.segmented');
    expect(segmented.attributes('aria-label')).toBe('Message rendering');
    expect(segmented.findAll('button').map((button) => button.text())).toEqual(['Original', 'Text']);
    expect(wrapper.find('.body-view-note').exists()).toBe(false);
  });

  it('moves into the ⋯ sheet on the phone and leaves a way back above the body', async () => {
    useUiStore().isMobile = true;
    wrapper = render();
    await flushPromises();

    // Nothing stands between the sender and the mail while it shows as sent.
    expect(wrapper.find('.segmented').exists()).toBe(false);
    expect(wrapper.find('.body-view-note').exists()).toBe(false);
    expect(wrapper.find('iframe.satchel-email-document').exists()).toBe(true);

    await openActions(wrapper);
    const choices = sheetChoices();
    expect(choices.map((choice) => choice.textContent?.trim())).toEqual(['Original', 'Text']);
    expect(choices.map((choice) => choice.getAttribute('aria-checked'))).toEqual(['true', 'false']);

    choices[1]!.click();
    await flushPromises();

    expect(wrapper.find('iframe.satchel-email-document').exists()).toBe(false);
    expect(wrapper.get('.plain-document-group').text()).toContain('Wochenangebote für Büro');
    const note = wrapper.get('.body-view-note');
    expect(note.text()).toContain('Shown as text');
    const back = note.get('button');
    expect(back.text()).toBe('Original');
    expect(back.attributes('aria-label')).toBe('Show as Original');

    await back.trigger('click');
    await flushPromises();

    expect(wrapper.find('iframe.satchel-email-document').exists()).toBe(true);
    expect(wrapper.find('.body-view-note').exists()).toBe(false);
  });

  it('leads back to the mode the mail opened in, auto-dark in the dark theme', async () => {
    const settings = useSettingsStore();
    settings.theme = 'dark';
    settings.autoDarkHtmlMails = true;
    useUiStore().isMobile = true;
    wrapper = render();
    await flushPromises();

    await openActions(wrapper);
    const choices = sheetChoices();
    expect(choices.map((choice) => choice.textContent?.trim())).toEqual(['Original', 'Auto-dark', 'Text']);
    expect(choices[1]!.getAttribute('aria-checked')).toBe('true');

    choices[0]!.click();
    await flushPromises();

    const note = wrapper.get('.body-view-note');
    expect(note.text()).toContain('Shown as original');
    expect(note.get('button').text()).toBe('Auto-dark');
  });

  it('offers no choice for a body with nothing to switch to', async () => {
    useUiStore().isMobile = true;
    wrapper = mount(MessageBubble, {
      attachTo: document.body,
      props: { message: { ...message(), bodyHtml: '', bodyText: 'Just text.' }, canReply: true },
      global: { stubs: { SenderFilingMenu: true } },
    });
    await flushPromises();
    await openActions(wrapper);

    expect(sheetChoices()).toHaveLength(0);
    expect(document.body.textContent).not.toContain('Show as');
  });
});
