// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { useUiStore } from '@/stores/ui';
import AttachmentPreview from '@/components/mail/AttachmentPreview.vue';
import MessageSourceDialog from '@/components/mail/MessageSourceDialog.vue';
import SendLaterPicker from '@/components/mail/SendLaterPicker.vue';
import ShortcutHelper from '@/components/navigation/ShortcutHelper.vue';
import TemplatePicker from '@/components/mail/TemplatePicker.vue';

/*
 * The shell had a test, a doc comment and no callers. These are the callers:
 * every dialog that used to build its own `position: fixed` scrim now asks the
 * shell for one, and each of them still does what it did before — render its
 * content, close on the backdrop, and close from the overlay stack when Escape
 * or the system back gesture reaches it.
 */

vi.mock('@/stores/templates', () => ({
  useTemplatesStore: () => ({
    items: [{ id: 'greeting', name: 'Greeting', shortcut: 'gr', body_text: 'Hi there' }],
    load: vi.fn(),
    render: vi.fn(),
  }),
}));

vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({
    loadMessageSource: vi.fn().mockResolvedValue({
      message_id: 'message-1',
      subject: 'Signed message',
      raw_source: 'From: sender@example.com\r\n\r\nHello',
      raw_source_status: 'loaded',
      body_text: 'Hello',
    }),
  }),
}));

vi.mock('@/composables/useAttachmentActions', () => ({
  useAttachmentActions: () => ({
    downloadAttachment: vi.fn().mockResolvedValue({
      filename: 'notes.txt',
      mime_type: 'text/plain',
      data_base64: 'SGVsbG8=',
    }),
    nestedMessagePath: () => null,
    openMessageFile: vi.fn(),
    saveDownloadedAttachment: vi.fn(),
    openAttachmentDefault: vi.fn(),
  }),
}));

vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }));

let wrapper: VueWrapper | null = null;

/** The backdrop may be teleported, so it is looked up in the document. */
function backdrop(): HTMLElement {
  const element = document.body.querySelector<HTMLElement>('.shell-backdrop');
  if (!element) throw new Error('no modal shell on screen');
  return element;
}

function panel(): HTMLElement {
  const element = document.body.querySelector<HTMLElement>('.shell-panel');
  if (!element) throw new Error('no modal shell on screen');
  return element;
}

function clickBackdrop() {
  backdrop().dispatchEvent(new MouseEvent('click', { bubbles: true }));
}

beforeEach(() => {
  setActivePinia(createPinia());
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  document.body.innerHTML = '';
  vi.clearAllMocks();
});

describe('template picker on the shell', () => {
  beforeEach(() => {
    wrapper = mount(TemplatePicker, { attachTo: document.body });
  });

  it('renders its list inside the shared dialog frame', () => {
    expect(panel().getAttribute('aria-modal')).toBe('true');
    expect(panel().textContent).toContain('Greeting');
  });

  it('still closes on the backdrop', () => {
    clickBackdrop();
    expect(wrapper?.emitted('close')).toHaveLength(1);
  });

  it('still closes from the overlay stack', () => {
    expect(useUiStore().dismissTopmostOverlay()).toBe(true);
    expect(wrapper?.emitted('close')).toHaveLength(1);
  });
});

describe('send-later picker on the shell', () => {
  beforeEach(() => {
    wrapper = mount(SendLaterPicker, { attachTo: document.body, props: { value: null } });
  });

  it('renders the custom minute picker inside the shared dialog frame', () => {
    expect(panel().getAttribute('role')).toBe('dialog');
    expect(panel().querySelector('input')?.getAttribute('step')).toBe('60');
  });

  it('still closes on the backdrop', () => {
    clickBackdrop();
    expect(wrapper?.emitted('close')).toHaveLength(1);
  });

  it('still closes from the overlay stack', () => {
    expect(useUiStore().dismissTopmostOverlay()).toBe(true);
    expect(wrapper?.emitted('close')).toHaveLength(1);
  });
});

describe('attachment preview on the shell', () => {
  beforeEach(async () => {
    wrapper = mount(AttachmentPreview, {
      attachTo: document.body,
      props: { attachmentId: 'attachment-1', filename: 'notes.txt', mimeType: 'text/plain' },
    });
    await flushPromises();
  });

  it('renders the decoded attachment inside the shared dialog frame', () => {
    expect(panel().getAttribute('aria-label')).toBe('Attachment preview');
    expect(panel().querySelector('pre')?.textContent).toBe('Hello');
  });

  it('still closes on the backdrop', () => {
    clickBackdrop();
    expect(wrapper?.emitted('close')).toHaveLength(1);
  });

  it('still closes from the overlay stack', () => {
    expect(useUiStore().dismissTopmostOverlay()).toBe(true);
    expect(wrapper?.emitted('close')).toHaveLength(1);
  });
});

describe('message source dialog on the shell', () => {
  beforeEach(async () => {
    wrapper = mount(MessageSourceDialog, {
      attachTo: document.body,
      props: { open: true, messageId: 'message-1' },
    });
    await flushPromises();
  });

  it('renders the raw source inside the shared dialog frame', () => {
    expect(panel().getAttribute('aria-modal')).toBe('true');
    expect(panel().querySelector('pre')?.textContent).toContain('sender@example.com');
  });

  it('still closes on the backdrop', () => {
    clickBackdrop();
    expect(wrapper?.emitted('close')).toHaveLength(1);
  });

  it('still closes from the overlay stack', () => {
    expect(useUiStore().dismissTopmostOverlay()).toBe(true);
    expect(wrapper?.emitted('close')).toHaveLength(1);
  });
});

describe('shortcut helper on the shell', () => {
  beforeEach(async () => {
    const ui = useUiStore();
    ui.shortcutsOpen = true;
    wrapper = mount(ShortcutHelper, { attachTo: document.body });
    await flushPromises();
  });

  it('renders its groups inside the shared dialog frame', () => {
    expect(panel().getAttribute('role')).toBe('dialog');
    expect(panel().textContent).toContain('Keyboard shortcuts');
  });

  it('still closes on the backdrop', async () => {
    clickBackdrop();
    await flushPromises();
    expect(useUiStore().shortcutsOpen).toBe(false);
  });

  it('still closes from the overlay stack', async () => {
    expect(useUiStore().dismissTopmostOverlay()).toBe(true);
    await flushPromises();
    expect(useUiStore().shortcutsOpen).toBe(false);
  });
});

/*
 * The link dialog lives inside a Tiptap bubble menu, which never renders in
 * jsdom, so it is checked where it can be: in the source. The rule is the same
 * one the other five keep — the shell owns the frame, nobody rebuilds it.
 */
describe('no dialog builds its own frame any more', () => {
  const converted = [
    'src/components/mail/AttachmentPreview.vue',
    'src/components/mail/MessageSourceDialog.vue',
    'src/components/mail/RichTextEditor.vue',
    'src/components/mail/SendLaterPicker.vue',
    'src/components/mail/TemplatePicker.vue',
    'src/components/navigation/ShortcutHelper.vue',
  ];

  it.each(converted)('%s asks the shell for its frame', (file) => {
    const source = readFileSync(join(process.cwd(), file), 'utf8');

    expect(source).toContain('<ModalShell');
    expect(source).not.toMatch(/position:\s*fixed;\s*\n?\s*inset:\s*0/);
  });
});
