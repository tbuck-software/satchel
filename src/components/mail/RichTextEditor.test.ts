// @vitest-environment jsdom
import { defineComponent, h, nextTick, ref } from 'vue';
import { EditorContent, type Editor } from '@tiptap/vue-3';
import { createPinia } from 'pinia';
import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import RichTextEditor from './RichTextEditor.vue';

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
