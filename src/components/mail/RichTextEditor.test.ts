// @vitest-environment jsdom
import { defineComponent, h, nextTick, ref } from 'vue';
import { EditorContent, type Editor } from '@tiptap/vue-3';
import { createPinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import RichTextEditor from './RichTextEditor.vue';
import { insertMailTable, mailTableHtml } from '@/utils/composerTable';

const clipboard = vi.hoisted(() => ({ text: null as string | null }));
vi.mock('@/services/clipboard', () => ({
  readClipboardText: () => Promise.resolve(clipboard.text),
}));

/** Binds the editor the way ComposeView does: the emitted HTML comes straight back. */
function render(initialHtml: string) {
  const html = ref<string | null>(initialHtml);
  const text = ref('');
  const wrapper = mount(defineComponent({
    setup() {
      return () => h(RichTextEditor, {
        modelValue: text.value,
        html: html.value,
        'onUpdate:modelValue': (value: string) => { text.value = value; },
        'onUpdate:html': (value: string) => { html.value = value; },
      });
    },
  }), { global: { plugins: [createPinia()] } });
  const editor = wrapper.getComponent(EditorContent).props('editor') as Editor;
  return { wrapper, editor, html, text };
}

describe('RichTextEditor per-keystroke work', () => {
  it('serialises the document once per edit, not again for the echoed prop', async () => {
    const { wrapper, editor, html } = render('<p>Hello</p>');
    const getHTML = vi.spyOn(editor, 'getHTML');

    editor.commands.insertContent(' world');
    await nextTick();
    await nextTick();

    expect(getHTML).toHaveBeenCalledTimes(1);
    expect(html.value).toContain('world');
    wrapper.unmount();
  });

  it('still applies a replacement from outside, including a return to an earlier emit', async () => {
    const { wrapper, editor, html } = render('<p>Hello</p>');
    editor.commands.insertContent(' world');
    await nextTick();
    const typed = html.value;

    html.value = '<p>Template</p>';
    await nextTick();
    expect(editor.getText()).toBe('Template');

    html.value = typed;
    await nextTick();
    expect(editor.getText()).toContain('world');
    wrapper.unmount();
  });
});

describe('RichTextEditor tables', () => {
  it('emits a table the recipient can read without a stylesheet', async () => {
    const { wrapper, editor, html, text } = render('<p></p>');
    editor.commands.setTextSelection(1);
    insertMailTable(editor);
    editor.commands.insertContent('Item');
    await nextTick();

    const template = document.createElement('template');
    template.innerHTML = html.value ?? '';
    const table = template.content.querySelector('table')!;
    expect(table.getAttribute('style')).toMatch(/border-collapse:\s*collapse/);
    const header = table.querySelector('th')!;
    expect(header.getAttribute('style')).toMatch(/border:\s*1px solid/);
    expect(header.getAttribute('style')).toMatch(/padding:\s*6px 10px/);
    // The flattening happens on the way out of the component, not in the schema.
    expect(header.querySelector('p')?.getAttribute('style')).toBe('margin:0');
    expect(text.value).toContain('| Item |');
    wrapper.unmount();
  });

  it('recognises a reopened draft it already shows instead of loading it again', async () => {
    // A draft as saved: the component's own emitted HTML.
    const first = render('<p></p>');
    first.editor.commands.setTextSelection(1);
    insertMailTable(first.editor);
    first.editor.commands.insertContent('Q1');
    await nextTick();
    const saved = first.html.value!;
    first.wrapper.unmount();

    // Reopened: the same table arrives through the prop, first as the stored
    // HTML and then once more from the composer restoring the draft.
    const { wrapper, editor, html } = render(saved.replace(/ style="margin:0"/g, ''));
    const shown = editor.state.doc;
    html.value = saved;
    await nextTick();

    expect(editor.state.doc).toBe(shown);
    expect(mailTableHtml(editor.getHTML())).toBe(saved);
    wrapper.unmount();
  });
});

describe('RichTextEditor paste without formatting', () => {
  it('inserts the clipboard as plain paragraphs on Cmd/Ctrl+Shift+V', async () => {
    const { wrapper, editor, html } = render('<p>Intro</p>');
    editor.commands.focus('end');
    clipboard.text = 'Tenant-ID\nClient-ID';
    // jsdom has no ClipboardEvent; ProseMirror builds one for the synthetic paste.
    vi.stubGlobal('ClipboardEvent', class extends Event {});

    const event = new KeyboardEvent('keydown', { key: 'V', code: 'KeyV', shiftKey: true, ctrlKey: true, cancelable: true });
    editor.view.dom.dispatchEvent(event);
    await flushPromises();

    expect(event.defaultPrevented).toBe(true);
    expect(html.value).toBe('<p>IntroTenant-ID</p><p>Client-ID</p>');
    wrapper.unmount();
    vi.unstubAllGlobals();
  });
});
