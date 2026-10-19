<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import CardButton from '@/components/ui/CardButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Editor, EditorContent, Mark, mergeAttributes } from '@tiptap/vue-3';
import { BubbleMenu } from '@tiptap/vue-3/menus';
import StarterKit from '@tiptap/starter-kit';
import Link from '@tiptap/extension-link';
import Image from '@tiptap/extension-image';
import Placeholder from '@tiptap/extension-placeholder';
import {
  declarationValue,
  fontElementStyle,
  keepTextDeclarations,
  withDeclaration,
} from '@/utils/inlineTextStyle';
import type { InlineImagePayload } from '@/types/composer';
import { bytesToBase64 } from '@/utils/base64';
import { useDismissable } from '@/composables/useDismissable';
import ModalShell from '@/components/ui/ModalShell.vue';
import { isComposerHistoryIntro } from '@/utils/composerHistory';

type ImageAlignment = 'left' | 'center' | 'right' | 'wrap-left' | 'wrap-right';

const { t } = useI18n();

const ResizableImage = Image.extend({
  addAttributes() {
    return {
      ...this.parent?.(),
      alignment: {
        default: 'center',
        parseHTML: (element) => element.getAttribute('data-alignment') || 'center',
        renderHTML: (attributes) => {
          const alignment = attributes.alignment as ImageAlignment;
          const styles: Record<ImageAlignment, string> = {
            left: 'display:block;margin-left:0;margin-right:auto;',
            center: 'display:block;margin-left:auto;margin-right:auto;',
            right: 'display:block;margin-left:auto;margin-right:0;',
            'wrap-left': 'float:left;margin:0 18px 12px 0;',
            'wrap-right': 'float:right;margin:0 0 12px 18px;',
          };
          return {
            'data-alignment': alignment,
            style: styles[alignment] ?? styles.center,
          };
        },
      },
    };
  },
});

/** Inline elements that carry text styling in real mail signatures. */
const STYLED_INLINE_TAGS = ['span', 'strong', 'b', 'em', 'i', 'u', 's', 'a', 'font'];

/**
 * Carries colour, font and size through the editor.
 *
 * Without it the schema knows the bold in `<strong style="color:#48c725">` and
 * nothing else, so an adopted signature comes out in plain body text. The mark
 * renders as a span, which means the styling survives a round trip even when
 * the element it arrived on is not one the schema keeps.
 */
const InlineTextStyle = Mark.create({
  name: 'inlineTextStyle',

  addAttributes() {
    return {
      style: {
        default: null,
        parseHTML: (element: HTMLElement) => keepTextDeclarations(element.getAttribute('style')),
        renderHTML: (attributes: Record<string, unknown>) =>
          attributes.style ? { style: attributes.style as string } : {},
      },
    };
  },

  parseHTML() {
    return [
      ...STYLED_INLINE_TAGS.map((tag) => ({
        tag: `${tag}[style]`,
        getAttrs: (element: HTMLElement) => {
          const style = keepTextDeclarations(element.getAttribute('style'));
          return style ? { style } : false;
        },
      })),
      {
        // Pre-CSS mail still styles text with attributes.
        tag: 'font[color], font[face]',
        getAttrs: (element: HTMLElement) => {
          const style = fontElementStyle(
            element.getAttribute('color'),
            element.getAttribute('face'),
          );
          return style ? { style } : false;
        },
      },
    ];
  },

  renderHTML({ HTMLAttributes }) {
    return ['span', mergeAttributes(HTMLAttributes), 0];
  },
});

const props = withDefaults(
  defineProps<{
    /** Plain-text body — kept in sync with HTML for plain-text Compose pipelines. */
    modelValue: string;
    /** Initial HTML — used when restoring drafts that have body_html. */
    html?: string | null;
    placeholder?: string;
    minHeight?: string;
    disabled?: boolean;
    autofocus?: boolean;
    focusPosition?: 'start' | 'end';
  }>(),
  {
    html: null,
    minHeight: '240px',
    disabled: false,
    autofocus: false,
    focusPosition: 'end',
  },
);

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void;
  (e: 'update:html', value: string): void;
  (e: 'paste-files', files: File[]): void;
  (e: 'inline-image-added', image: InlineImagePayload): void;
  (e: 'image-to-attachment', image: InlineImagePayload): void;
  (e: 'submit'): void;
}>();

const editor = shallowRef<Editor | null>(null);
const showLinkDialog = ref(false);
const linkUrl = ref('');
const linkText = ref('');
const colorPickerOpen = ref(false);

// Back and Escape close the link dialog before they close the composer.
useDismissable(() => showLinkDialog.value, () => { showLinkDialog.value = false; });

editor.value = new Editor({
  content: props.html || htmlFromText(props.modelValue),
  extensions: [
    StarterKit.configure({
      heading: { levels: [1, 2, 3] },
      link: false,
    }),
    InlineTextStyle,
    Link.configure({
      openOnClick: false,
      autolink: true,
      HTMLAttributes: {
        rel: 'noopener noreferrer',
        target: '_blank',
      },
    }),
    ResizableImage.configure({
      inline: false,
      allowBase64: true,
      resize: {
        enabled: true,
        directions: ['left', 'right'],
        minWidth: 96,
        minHeight: 48,
        alwaysPreserveAspectRatio: true,
      },
    }),
    Placeholder.configure({
      placeholder: () => props.placeholder || t('compose.editor.placeholder'),
    }),
  ],
  editorProps: {
    handleKeyDown: (_view, event) => {
      if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
        event.preventDefault();
        emit('submit');
        return true;
      }
      return false;
    },
    handlePaste: (_view, event) => {
      const items = Array.from(event.clipboardData?.items ?? []);
      const files = items
        .filter((item) => item.kind === 'file')
        .map((item) => item.getAsFile())
        .filter((file): file is File => file !== null);
      if (files.length > 0) {
        const images = files.filter((file) => file.type.startsWith('image/'));
        const attachments = files.filter((file) => !file.type.startsWith('image/'));
        images.forEach((file) => void insertImageFromFile(file));
        if (attachments.length) emit('paste-files', attachments);
        return true;
      }
      return false;
    },
    handleDrop: (view, event, _slice, moved) => {
      if (moved) return false;
      const files = Array.from(event.dataTransfer?.files ?? []);
      if (!files.length) return false;
      const images = files.filter((file) => file.type.startsWith('image/'));
      const attachments = files.filter((file) => !file.type.startsWith('image/'));
      const position = view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos;
      images.forEach((file, index) => void insertImageFromFile(file, position === undefined ? undefined : position + index));
      if (attachments.length) emit('paste-files', attachments);
      return true;
    },
  },
  editable: !props.disabled,
  autofocus: props.autofocus ? props.focusPosition : false,
  onUpdate: ({ editor: ed }) => {
    const html = ed.getHTML();
    const text = ed.getText();
    emit('update:modelValue', text);
    emit('update:html', html);
  },
  // Moving the caret ends the colour decision it was opened for.
  onSelectionUpdate: () => {
    colorPickerOpen.value = false;
  },
});

watch(
  () => props.disabled,
  (disabled) => editor.value?.setEditable(!disabled),
);

watch(
  () => props.html,
  (newHtml, oldHtml) => {
    // Only mirror external resets (e.g. template insertion or draft restore) —
    // skip when the change came from us.
    if (!editor.value) return;
    if (newHtml === oldHtml) return;
    const current = editor.value.getHTML();
    const candidate = newHtml || htmlFromText(props.modelValue);
    if (candidate !== current) {
      editor.value.commands.setContent(candidate, { emitUpdate: false });
      if (props.focusPosition === 'start') editor.value.commands.setTextSelection(1);
    }
  },
);

watch(
  () => props.modelValue,
  (next) => {
    if (!editor.value) return;
    // If only plain-text changed externally and editor is empty, seed it.
    if (!props.html && !editor.value.getText() && next) {
      editor.value.commands.setContent(htmlFromText(next), { emitUpdate: false });
    }
  },
);

onBeforeUnmount(() => {
  editor.value?.destroy();
  editor.value = null;
});

function htmlFromText(text: string): string {
  if (!text) return '';
  return text
    .split(/\r?\n/)
    .map((line) => `<p>${escapeHtml(line)}</p>`)
    .join('');
}

function escapeHtml(s: string) {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function toggle(action: () => void) {
  action();
  editor.value?.commands.focus();
}

function openLinkDialog() {
  if (!editor.value) return;
  const previous = editor.value.getAttributes('link').href;
  linkUrl.value = previous || 'https://';
  const { from, to } = editor.value.state.selection;
  linkText.value = editor.value.state.doc.textBetween(from, to);
  showLinkDialog.value = true;
}

function applyLink() {
  if (!editor.value) return;
  const url = linkUrl.value.trim();
  if (!url) {
    editor.value.chain().focus().unsetLink().run();
  } else if (linkText.value && editor.value.state.selection.empty) {
    editor.value
      .chain()
      .focus()
      .insertContent(`<a href="${escapeHtml(url)}">${escapeHtml(linkText.value)}</a>`)
      .run();
  } else {
    editor.value.chain().focus().setLink({ href: url }).run();
  }
  showLinkDialog.value = false;
}

async function insertImageFromFile(file: File, position?: number) {
  const contentBase64 = bytesToBase64(new Uint8Array(await file.arrayBuffer()));
  const mimeType = file.type || 'image/png';
  const dataUrl = `data:${mimeType};base64,${contentBase64}`;
  const payload = {
    filename: file.name || t('compose.editor.image.fallbackName'),
    mimeType,
    contentBase64,
    dataUrl,
  };
  if (position === undefined) {
    editor.value?.chain().focus().setImage({ src: dataUrl, alt: payload.filename, title: payload.filename, width: 480 }).run();
  } else {
    editor.value?.chain().focus().insertContentAt(position, {
      type: 'image',
      attrs: { src: dataUrl, alt: payload.filename, title: payload.filename, width: 480 },
    }).run();
  }
  emit('inline-image-added', payload);
}

const ed = computed(() => editor.value ?? undefined);

function setParagraphStyle(event: Event) {
  const value = (event.target as HTMLSelectElement).value;
  const chain = editor.value?.chain().focus();
  if (!chain) return;
  if (value === 'heading-1') chain.toggleHeading({ level: 1 }).run();
  else if (value === 'heading-2') chain.toggleHeading({ level: 2 }).run();
  else if (value === 'heading-3') chain.toggleHeading({ level: 3 }).run();
  else chain.setParagraph().run();
}

const currentBlockStyle = computed(() => {
  if (ed.value?.isActive('heading', { level: 1 })) return 'heading-1';
  if (ed.value?.isActive('heading', { level: 2 })) return 'heading-2';
  if (ed.value?.isActive('heading', { level: 3 })) return 'heading-3';
  return 'paragraph';
});

/**
 * Text colours, not UI colours: these values travel into the mail and are read
 * on the recipient's background, so they are deliberately literal and do not
 * follow the app theme.
 */
const TEXT_COLORS = computed(() => [
  { value: '#202124', label: t('compose.editor.colors.black') },
  { value: '#5f6368', label: t('compose.editor.colors.gray') },
  { value: '#d93025', label: t('compose.editor.colors.red') },
  { value: '#e8710a', label: t('compose.editor.colors.orange') },
  { value: '#188038', label: t('compose.editor.colors.green') },
  { value: '#1a73e8', label: t('compose.editor.colors.blue') },
  { value: '#8430ce', label: t('compose.editor.colors.purple') },
  { value: '#c2185b', label: t('compose.editor.colors.magenta') },
]);

/** The colour under the caret, or null while the text runs in the default. */
const currentTextColor = computed(() => {
  const style = ed.value?.getAttributes('inlineTextStyle').style as string | undefined;
  return declarationValue(style ?? null, 'color');
});

function applyTextColor(value: string | null) {
  const editorInstance = editor.value;
  if (!editorInstance) return;

  const current = editorInstance.getAttributes('inlineTextStyle').style as string | undefined;
  const style = withDeclaration(current ?? null, 'color', value);
  const chain = editorInstance.chain().focus();

  // Nothing left to carry — drop the mark rather than leave an empty span.
  if (style) chain.setMark('inlineTextStyle', { style });
  else chain.unsetMark('inlineTextStyle');

  chain.run();
  colorPickerOpen.value = false;
}

function shouldShowImageMenu({ editor: menuEditor }: { editor: { isActive: (name: string) => boolean } }) {
  return menuEditor.isActive('image');
}

function shouldShowTextMenu({
  editor: menuEditor,
  from,
  to,
}: {
  editor: { isActive: (name: string) => boolean };
  from: number;
  to: number;
}) {
  return from !== to && !menuEditor.isActive('image');
}

function setSelectedImageWidth(width: number | null) {
  editor.value?.chain().focus().updateAttributes('image', { width, height: null }).run();
}

function setSelectedImageAlignment(alignment: ImageAlignment) {
  editor.value?.chain().focus().updateAttributes('image', { alignment }).run();
}

function dataUrlPayload(src: string, filename: string): InlineImagePayload | null {
  const match = /^data:([^;,]+);base64,(.+)$/s.exec(src);
  if (!match?.[1] || !match[2]) return null;
  return {
    filename: filename || t('compose.editor.image.fallbackName'),
    mimeType: match[1],
    contentBase64: match[2],
    dataUrl: src,
  };
}

function moveSelectedImageToAttachments() {
  if (!editor.value) return;
  const { selection } = editor.value.state;
  const node = (selection as typeof selection & {
    node?: { type: { name: string }; attrs: Record<string, unknown> };
  }).node ?? null;
  if (!node || node.type.name !== 'image') return;
  const payload = dataUrlPayload(
    String(node.attrs.src ?? ''),
    String(node.attrs.title || node.attrs.alt || t('compose.editor.image.fallbackName')),
  );
  if (!payload) return;
  emit('image-to-attachment', payload);
  editor.value.chain().focus().deleteSelection().run();
}

function insertAttachmentImage(image: {
  filename: string;
  mime_type: string;
  content_base64: string;
}) {
  const dataUrl = `data:${image.mime_type};base64,${image.content_base64}`;
  if (!editor.value) return;
  const insertionPosition = editor.value.state.selection.to;
  editor.value
    .chain()
    .focus()
    .setTextSelection(insertionPosition)
    .setImage({ src: dataUrl, alt: image.filename, title: image.filename, width: 480 })
    .run();
}

function removeAttachmentImage(image: {
  mime_type: string;
  content_base64: string;
}) {
  if (!editor.value) return;
  const dataUrl = `data:${image.mime_type};base64,${image.content_base64}`;
  const imageRanges: Array<{ from: number; to: number }> = [];
  editor.value.state.doc.descendants((node, position) => {
    if (node.type.name === 'image' && node.attrs.src === dataUrl) {
      imageRanges.push({ from: position, to: position + node.nodeSize });
    }
    return true;
  });
  if (!imageRanges.length) return;
  const chain = editor.value.chain().focus();
  imageRanges.reverse().forEach((range) => chain.deleteRange(range));
  chain.run();
}

function focusEditor() {
  editor.value?.commands.focus();
}

function removeQuotedHistory() {
  const ed = editor.value;
  if (!ed) return;
  const ranges: Array<{ from: number; to: number }> = [];
  ed.state.doc.descendants((node, position) => {
    if (node.type.name !== 'blockquote' || !isComposerHistoryIntro(node.firstChild?.textContent ?? '')) return;
    ranges.push({ from: position, to: position + node.nodeSize });
    return false;
  });
  const transaction = ed.chain();
  ranges.reverse().forEach((range) => transaction.deleteRange(range));
  transaction.run();
}

defineExpose({ focusEditor, insertAttachmentImage, removeAttachmentImage, removeQuotedHistory });
</script>

<template>
  <div class="rt-editor">
    <BubbleMenu
      v-if="ed"
      :editor="ed"
      :options="{ placement: 'top', offset: 8 }"
      :should-show="shouldShowTextMenu"
      class="selection-toolbar"
    >
      <select
        :value="currentBlockStyle"
        :title="$t('compose.editor.blockStyle')"
        @change="setParagraphStyle"
      >
        <option value="paragraph">{{ $t('compose.editor.paragraph') }}</option>
        <option value="heading-1">{{ $t('compose.editor.heading1') }}</option>
        <option value="heading-2">{{ $t('compose.editor.heading2') }}</option>
        <option value="heading-3">{{ $t('compose.editor.heading3') }}</option>
      </select>
      <span class="sep" />
      <IconButton :active="ed.isActive('bold')" :title="$t('compose.editor.bold')" @click="toggle(() => ed?.chain().focus().toggleBold().run())"><b>B</b></IconButton>
      <IconButton :active="ed.isActive('italic')" :title="$t('compose.editor.italic')" @click="toggle(() => ed?.chain().focus().toggleItalic().run())"><i>I</i></IconButton>
      <IconButton :active="ed.isActive('strike')" :title="$t('compose.editor.strike')" @click="toggle(() => ed?.chain().focus().toggleStrike().run())"><s>S</s></IconButton>
      <span class="sep" />
      <IconButton :active="ed.isActive('bulletList')" :title="$t('compose.editor.bulletList')" @click="toggle(() => ed?.chain().focus().toggleBulletList().run())">
        <svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M2.6 4.5h.7M2.6 8h.7M2.6 11.5h.7M6.5 4.5h7M6.5 8h7M6.5 11.5h7" /></svg>
      </IconButton>
      <IconButton :active="ed.isActive('orderedList')" :title="$t('compose.editor.orderedList')" @click="toggle(() => ed?.chain().focus().toggleOrderedList().run())">
        <svg class="icon" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M6.5 5h7M6.5 11h7" />
          <text class="icon-numeral" x="1" y="6.9">1</text>
          <text class="icon-numeral" x="1" y="12.9">2</text>
        </svg>
      </IconButton>
      <IconButton :active="ed.isActive('link')" :title="$t('compose.editor.link')" :aria-label="$t('compose.editor.link')" @click="openLinkDialog">
        <svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M6.9 9.1a2.7 2.7 0 004 .3l1.8-1.8a2.7 2.7 0 00-3.8-3.8l-1 1" /><path d="M9.1 6.9a2.7 2.7 0 00-4-.3L3.3 8.4a2.7 2.7 0 003.8 3.8l1-1" /></svg>
      </IconButton>
      <span class="sep" />
      <div class="color-control">
        <IconButton
          :active="colorPickerOpen || currentTextColor !== null"
          :title="$t('compose.editor.textColor')"
          aria-haspopup="true"
          :aria-expanded="colorPickerOpen"
          @click="colorPickerOpen = !colorPickerOpen"
        >
          <svg class="icon" viewBox="0 0 16 16" aria-hidden="true">
            <path d="M3.6 11.2L8 3.4l4.4 7.8M5.4 8.6h5.2" />
          </svg>
          <span
            class="color-bar"
            :style="{ background: currentTextColor ?? 'currentColor' }"
            aria-hidden="true"
          />
        </IconButton>

        <div v-if="colorPickerOpen" class="color-panel" role="group" :aria-label="$t('compose.editor.textColor')">
          <div class="color-swatches">
            <CardButton
              v-for="color in TEXT_COLORS"
              :key="color.value"
              class="color-swatch"
              :class="{ chosen: currentTextColor === color.value }"
              :style="{ background: color.value }"
              :title="color.label"
              :aria-label="color.label"
              @click="applyTextColor(color.value)"
            />
          </div>
          <label class="color-custom" :title="$t('compose.editor.customColor')">
            <!-- change, not input: the OS picker streams every drag step, and
                 applying one closes the panel the picker was opened from. -->
            <input
              type="color"
              :value="currentTextColor ?? '#202124'"
              @change="applyTextColor(($event.target as HTMLInputElement).value)"
            />
            {{ $t('compose.editor.customColorLabel') }}
          </label>
          <AppButton variant="ghost" size="sm" class="color-reset" @click="applyTextColor(null)">
            {{ $t('compose.editor.defaultColor') }}
          </AppButton>
        </div>
      </div>
    </BubbleMenu>

    <BubbleMenu
      v-if="ed"
      plugin-key="imageControls"
      :editor="ed"
      :options="{ placement: 'bottom' }"
      :should-show="shouldShowImageMenu"
      class="image-toolbar"
    >
      <div class="image-control-group image-width-group">
        <IconButton :title="$t('compose.editor.image.small')" @click="setSelectedImageWidth(280)">S</IconButton>
        <IconButton :title="$t('compose.editor.image.medium')" @click="setSelectedImageWidth(480)">M</IconButton>
        <IconButton :title="$t('compose.editor.image.large')" @click="setSelectedImageWidth(720)">L</IconButton>
        <IconButton :title="$t('compose.editor.image.original')" :aria-label="$t('compose.editor.image.original')" @click="setSelectedImageWidth(null)">
          <svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M3.3 8a4.7 4.7 0 104.7-4.7 4.7 4.7 0 00-3.7 1.8" /><path d="M3.6 2.7v2.7h2.7" /></svg>
        </IconButton>
      </div>
      <span class="image-toolbar-separator" />
      <div class="image-control-group">
        <IconButton :title="$t('compose.editor.image.alignLeft')" :aria-label="$t('compose.editor.image.alignLeft')" @click="setSelectedImageAlignment('left')">
          <svg width="22" height="18" viewBox="0 0 22 18" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 2h18M2 16h12"/><rect x="2" y="5" width="11" height="8" rx="1.5"/></svg>
        </IconButton>
        <IconButton :title="$t('compose.editor.image.alignCenter')" :aria-label="$t('compose.editor.image.alignCenter')" @click="setSelectedImageAlignment('center')">
          <svg width="22" height="18" viewBox="0 0 22 18" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 2h18M4 16h14"/><rect x="5.5" y="5" width="11" height="8" rx="1.5"/></svg>
        </IconButton>
        <IconButton :title="$t('compose.editor.image.alignRight')" :aria-label="$t('compose.editor.image.alignRight')" @click="setSelectedImageAlignment('right')">
          <svg width="22" height="18" viewBox="0 0 22 18" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 2h18M8 16h12"/><rect x="9" y="5" width="11" height="8" rx="1.5"/></svg>
        </IconButton>
      </div>
      <span class="image-toolbar-separator" />
      <div class="image-control-group">
        <IconButton :title="$t('compose.editor.image.wrapLeft')" :aria-label="$t('compose.editor.image.wrapLeft')" @click="setSelectedImageAlignment('wrap-left')">
          <svg width="24" height="18" viewBox="0 0 24 18" fill="none" stroke="currentColor" stroke-width="1.35"><rect x="2" y="4" width="9" height="10" rx="1.5"/><path d="M14 4h8M14 8h8M14 12h8M2 17h20"/></svg>
        </IconButton>
        <IconButton :title="$t('compose.editor.image.wrapRight')" :aria-label="$t('compose.editor.image.wrapRight')" @click="setSelectedImageAlignment('wrap-right')">
          <svg width="24" height="18" viewBox="0 0 24 18" fill="none" stroke="currentColor" stroke-width="1.35"><rect x="13" y="4" width="9" height="10" rx="1.5"/><path d="M2 4h8M2 8h8M2 12h8M2 17h20"/></svg>
        </IconButton>
      </div>
      <span class="image-toolbar-separator" />
      <IconButton class="image-as-attachment" :title="$t('compose.editor.image.asAttachment')" :aria-label="$t('compose.editor.image.asAttachment')" @click="moveSelectedImageToAttachments">
        <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.35" stroke-linecap="round"><path d="M13.5 7.5l-6.2 6.2a3.5 3.5 0 01-5-5l6.5-6.5a2.5 2.5 0 013.5 3.5L5.8 12.2a1.5 1.5 0 01-2.1-2.1l6.1-6.1"/></svg>
      </IconButton>
    </BubbleMenu>

    <EditorContent
      :editor="ed"
      class="content"
      :style="{ minHeight: props.minHeight }"
    />
    <ModalShell
      v-if="showLinkDialog"
      :label="$t('compose.editor.linkDialog.title')"
      width="420px"
      height="auto"
      @dismiss="showLinkDialog = false"
    >
      <div class="link-panel">
        <h4>{{ $t('compose.editor.linkDialog.title') }}</h4>
        <label>
          <span>URL</span>
          <input v-model="linkUrl" type="url" autofocus />
        </label>
        <label>
          <span>{{ $t('compose.editor.linkDialog.text') }}</span>
          <input v-model="linkText" type="text" :placeholder="$t('compose.editor.linkDialog.textPlaceholder')" />
        </label>
        <div class="link-actions">
          <AppButton class="ghost" @click="showLinkDialog = false" variant="ghost" size="sm">{{ $t('common.cancel') }}</AppButton>
          <AppButton class="primary" @click="applyLink" variant="primary" size="sm">{{ $t('compose.editor.linkDialog.insert') }}</AppButton>
        </div>
      </div>
    </ModalShell>
  </div>
</template>

<style scoped>
.rt-editor {
  display: grid;
  gap: 6px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  padding: 6px;
}
.selection-toolbar {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 5px;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, white 8%);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-overlay) 94%, transparent);
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.38), 0 1px 0 rgba(255, 255, 255, 0.04) inset;
  backdrop-filter: blur(18px) saturate(145%);
  -webkit-backdrop-filter: blur(18px) saturate(145%);
  z-index: 410;
}
.selection-toolbar button,
.selection-toolbar select {
  height: 30px;
  border: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  cursor: pointer;
}
.selection-toolbar button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 30px;
  padding: 0 7px;
}
.selection-toolbar select {
  width: 112px;
  padding: 0 7px;
}
.selection-toolbar button:hover,
.selection-toolbar select:hover,
.selection-toolbar .icon-button--active {
  background: var(--bg-hover);
  color: var(--text-primary);
}
.selection-toolbar .icon-button--active {
  color: var(--accent-primary);
}
.selection-toolbar .sep {
  width: 1px;
  height: 18px;
  margin: 0 3px;
  background: var(--border-color);
}
.color-control {
  position: relative;
}
.color-control > button {
  flex-direction: column;
  gap: 1px;
}
/* The swatch under the glyph is the only colour here that is content, not UI. */
.color-bar {
  width: 14px;
  height: 3px;
  border-radius: 2px;
}
.color-panel {
  position: absolute;
  top: calc(100% + 7px);
  right: 0;
  z-index: 1;
  display: grid;
  gap: 6px;
  width: 168px;
  padding: 8px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--bg-overlay);
  box-shadow: var(--shadow-lg);
}
.color-swatches {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 6px;
}
.selection-toolbar .color-swatch {
  min-width: 0;
  width: 28px;
  height: 28px;
  padding: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 70%, transparent);
  border-radius: 50%;
}
.selection-toolbar .color-swatch.chosen {
  box-shadow: 0 0 0 2px var(--bg-overlay), 0 0 0 4px var(--accent-primary);
}
.color-custom {
  display: flex;
  align-items: center;
  gap: 7px;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--font-size-xs);
}
.color-custom input {
  width: 26px;
  height: 22px;
  padding: 0;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: transparent;
  cursor: pointer;
}
.image-toolbar {
  display: flex;
  align-items: center;
  gap: 3px;
  padding: 5px;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, white 8%);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-overlay) 95%, transparent);
  box-shadow: 0 14px 34px rgba(0, 0, 0, 0.4);
  backdrop-filter: blur(18px) saturate(145%);
  -webkit-backdrop-filter: blur(18px) saturate(145%);
  z-index: 411;
}
.image-control-group {
  display: flex;
  align-items: center;
  gap: 2px;
}
.image-toolbar button {
  min-width: 28px;
  height: 29px;
  display: grid;
  place-items: center;
  padding: 0 5px;
  border: 0;
  border-radius: var(--radius-xs);
  background: var(--bg-hover);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--font-size-xs);
}
.image-toolbar button:hover {
  background: var(--bg-raised);
  color: var(--text-primary);
}
.image-toolbar .image-as-attachment {
  color: var(--accent-primary);
}
.image-toolbar-separator {
  width: 1px;
  height: 21px;
  margin: 0 1px;
  background: var(--border-color);
}
/* Shared look for every drawn toolbar glyph. */
.icon {
  width: 16px;
  height: 16px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.icon-numeral {
  fill: currentColor;
  stroke: none;
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-bold);
}
.content {
  padding: 8px 12px;
  color: var(--text-primary);
  font-size: var(--font-size-md);
  line-height: 1.6;
}
.content :deep(.ProseMirror) {
  outline: none;
  min-height: 100%;
  font-family: inherit;
}
.content :deep(.ProseMirror > *:first-child) {
  margin-top: 0;
}
.content :deep(.ProseMirror h1),
.content :deep(.ProseMirror h2),
.content :deep(.ProseMirror h3) {
  margin: 0.85em 0 0.35em;
  color: var(--text-primary);
  line-height: 1.2;
  letter-spacing: -0.02em;
}
.content :deep(.ProseMirror h1) { font-size: 1.85em; }
.content :deep(.ProseMirror h2) { font-size: 1.45em; }
.content :deep(.ProseMirror h3) { font-size: 1.2em; }
.content :deep(.ProseMirror ul),
.content :deep(.ProseMirror ol) {
  padding-left: 1.45em;
  margin: 0.5em 0;
}
.content :deep(.ProseMirror p.is-editor-empty:first-child::before) {
  color: var(--text-tertiary);
  content: attr(data-placeholder);
  float: left;
  height: 0;
  pointer-events: none;
}
.content :deep(blockquote) {
  border-left: 3px solid var(--border-color);
  margin: 8px 0;
  padding-left: 12px;
  color: var(--text-secondary);
}
.content :deep(pre) {
  background: var(--bg-raised);
  padding: 12px;
  border-radius: var(--radius-sm);
  font-family: var(--font-mono);
  font-size: var(--font-size-sm);
  overflow-x: auto;
}
.content :deep(a) {
  color: var(--accent-primary);
  text-decoration: underline;
}
.content :deep(img) {
  max-width: 100%;
  border-radius: var(--radius-sm);
}
.content :deep([data-resize-container]) {
  max-width: 100%;
  margin: 10px 0;
}
.content :deep([data-resize-wrapper]) {
  max-width: 100%;
  border-radius: var(--radius-sm);
  outline: 1px solid transparent;
  transition: outline-color var(--duration-fast) ease;
}
.content :deep([data-resize-container]:has(img[data-alignment='left'])) { justify-content: flex-start; }
.content :deep([data-resize-container]:has(img[data-alignment='center'])) { justify-content: center; }
.content :deep([data-resize-container]:has(img[data-alignment='right'])) { justify-content: flex-end; }
.content :deep([data-resize-container]:has(img[data-alignment='wrap-left'])) {
  float: left;
  width: fit-content;
  max-width: 64%;
  margin: 0 18px 12px 0;
}
.content :deep([data-resize-container]:has(img[data-alignment='wrap-right'])) {
  float: right;
  width: fit-content;
  max-width: 64%;
  margin: 0 0 12px 18px;
}
.content :deep([data-resize-container].ProseMirror-selectednode [data-resize-wrapper]),
.content :deep([data-resize-wrapper]:hover) {
  outline: 2px solid var(--accent-primary);
  outline-offset: 1px;
}
.content :deep([data-resize-handle]) {
  z-index: 2;
  opacity: 0;
  pointer-events: none;
  transition: opacity var(--duration-fast) ease, transform var(--duration-fast) ease;
}
.content :deep([data-resize-container].ProseMirror-selectednode [data-resize-handle]),
.content :deep([data-resize-container]:hover [data-resize-handle]) {
  opacity: 1;
  pointer-events: auto;
}
.content :deep([data-resize-handle='left']),
.content :deep([data-resize-handle='right']) {
  top: 50% !important;
  bottom: auto !important;
  width: 12px;
  height: 44px;
  margin: 0;
  transform: translateY(-50%);
  border: 2px solid var(--bg-surface);
  border-radius: var(--radius-full);
  background: var(--text-primary);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.36);
  cursor: ew-resize;
}
.content :deep([data-resize-handle='left']) { left: 10px !important; }
.content :deep([data-resize-handle='right']) { right: 10px !important; }
.link-panel {
  min-height: 0;
  padding: var(--space-lg);
  display: grid;
  align-content: start;
  gap: var(--space-sm);
  overflow: auto;
}
.link-panel h4 {
  margin: 0;
  color: var(--text-primary);
}
.link-panel label {
  display: grid;
  gap: 3px;
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
}
.link-panel input {
  border: 1px solid var(--border-color);
  background: var(--bg-raised);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  color: var(--text-primary);
  font-family: inherit;
}
.link-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
