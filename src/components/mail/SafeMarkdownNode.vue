<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import { computed, inject, ref, watch, type Ref } from 'vue';
import type { SafeMarkdownBlock, SafeMarkdownInline } from '@/utils/safeMarkdown';
import { assessExternalLink, type LinkInteraction } from '@/utils/externalLinks';

const props = defineProps<{
  node: SafeMarkdownBlock | SafeMarkdownInline;
}>();

const openLink = inject<(payload: LinkInteraction) => void>(
  'satchel-open-markdown-link',
  () => undefined,
);
const hoverLink = inject<(payload: LinkInteraction | null) => void>(
  'satchel-hover-markdown-link',
  () => undefined,
);
const allowRemoteImages = inject<Readonly<Ref<boolean>>>(
  'satchel-allow-markdown-images',
  ref(false),
);
const openImage = inject<(payload: { src: string; alt: string }) => void>(
  'satchel-open-markdown-image',
  () => undefined,
);
const imageFailed = ref(false);
const imageDescription = computed(() => {
  if (props.node.kind !== 'blocked_image') return '';
  const alt = props.node.alt.trim();
  return /^(?:image|img|bild|picture|photo|foto)$/i.test(alt) ? '' : alt;
});

watch([() => props.node, allowRemoteImages], () => {
  imageFailed.value = false;
});

const linkAssessment = computed(() => {
  if (props.node.kind !== 'link') return null;
  return assessExternalLink(props.node.href, props.node.label);
});

function interaction(event: MouseEvent): LinkInteraction | null {
  if (props.node.kind !== 'link') return null;
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  return {
    href: props.node.href,
    label: props.node.label,
    anchor: { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom },
  };
}

function requestLink(event: MouseEvent) {
  const payload = interaction(event);
  if (payload) openLink(payload);
}

function previewLink(event: MouseEvent) {
  const payload = interaction(event);
  if (payload) hoverLink(payload);
}

const markdownImageStyle = computed(() => {
  if (props.node.kind !== 'blocked_image' || !props.node.width || !props.node.height) return undefined;
  return {
    width: `min(100%, ${props.node.width}px)`,
    maxWidth: '100%',
  };
});

const blockedImageStyle = computed(() => {
  if (!markdownImageStyle.value || props.node.kind !== 'blocked_image') return undefined;
  return {
    ...markdownImageStyle.value,
    aspectRatio: `${props.node.width} / ${props.node.height}`,
  };
});

function requestImagePreview() {
  if (props.node.kind !== 'blocked_image' || !props.node.src) return;
  openImage({ src: props.node.src, alt: props.node.alt });
}
</script>

<template>
  <p v-if="node.kind === 'paragraph'">
    <SafeMarkdownNode v-for="(child, index) in node.children" :key="index" :node="child" />
  </p>
  <component :is="`h${node.depth}`" v-else-if="node.kind === 'heading'">
    <SafeMarkdownNode v-for="(child, index) in node.children" :key="index" :node="child" />
  </component>
  <pre v-else-if="node.kind === 'code_block'" class="md-code-block"><code>{{ node.text }}</code></pre>
  <blockquote v-else-if="node.kind === 'blockquote'">
    <SafeMarkdownNode v-for="(child, index) in node.children" :key="index" :node="child" />
  </blockquote>
  <component
    :is="node.ordered ? 'ol' : 'ul'"
    v-else-if="node.kind === 'list'"
    :start="node.ordered ? node.start : undefined"
  >
    <li v-for="(item, itemIndex) in node.items" :key="itemIndex">
      <SafeMarkdownNode v-for="(child, index) in item" :key="index" :node="child" />
    </li>
  </component>
  <hr v-else-if="node.kind === 'rule'">
  <div v-else-if="node.kind === 'table'" class="md-table-scroll">
    <table>
      <thead>
        <tr>
          <th
            v-for="(cell, cellIndex) in node.header"
            :key="cellIndex"
            :style="{ textAlign: node.align[cellIndex] ?? undefined }"
          >
            <SafeMarkdownNode v-for="(child, index) in cell" :key="index" :node="child" />
          </th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(row, rowIndex) in node.rows" :key="rowIndex">
          <td
            v-for="(cell, cellIndex) in row"
            :key="cellIndex"
            :style="{ textAlign: node.align[cellIndex] ?? undefined }"
          >
            <SafeMarkdownNode v-for="(child, index) in cell" :key="index" :node="child" />
          </td>
        </tr>
      </tbody>
    </table>
  </div>
  <p v-else-if="node.kind === 'raw_text'" class="md-raw-text">{{ node.text }}</p>
  <template v-else-if="node.kind === 'text'">{{ node.text }}</template>
  <strong v-else-if="node.kind === 'strong'">
    <SafeMarkdownNode v-for="(child, index) in node.children" :key="index" :node="child" />
  </strong>
  <em v-else-if="node.kind === 'emphasis'">
    <SafeMarkdownNode v-for="(child, index) in node.children" :key="index" :node="child" />
  </em>
  <s v-else-if="node.kind === 'delete'">
    <SafeMarkdownNode v-for="(child, index) in node.children" :key="index" :node="child" />
  </s>
  <code v-else-if="node.kind === 'code'" class="md-inline-code">{{ node.text }}</code>
  <a
    v-else-if="node.kind === 'link'"
    :href="node.href"
    :class="['md-link', { 'md-link--risky': linkAssessment?.requiresConfirmation }]"
    rel="noopener noreferrer"
    target="_blank"
    @click.prevent="requestLink"
    @mouseenter="previewLink"
    @mouseleave="hoverLink(null)"
  ><SafeMarkdownNode v-for="(child, index) in node.children" :key="index" :node="child" /><span
    v-if="linkAssessment?.requiresConfirmation"
    class="md-link-warning"
    :aria-label="$t('mail.link.warningLabel')"
  >⚠</span></a>
  <br v-else-if="node.kind === 'break'">
  <CardButton
    v-else-if="node.kind === 'blocked_image' && node.src && allowRemoteImages && !imageFailed"
    class="md-image-button"
    :style="markdownImageStyle"
    :aria-label="imageDescription ? $t('mail.markdown.enlargeImageNamed', { description: imageDescription }) : $t('mail.markdown.enlargeImage')"
    @click="requestImagePreview"
  >
    <img
      class="md-image"
      :src="node.src"
      :alt="node.alt"
      :width="node.width"
      :height="node.height"
      loading="lazy"
      decoding="async"
      referrerpolicy="no-referrer"
      @error="imageFailed = true"
    >
  </CardButton>
  <span
    v-else-if="node.kind === 'blocked_image' && node.src && allowRemoteImages && imageFailed"
    class="md-image-error"
    role="status"
  >
    <span>{{ $t('mail.markdown.imageLoadFailed') }}<span v-if="imageDescription">: {{ imageDescription }}</span></span>
  </span>
  <span
    v-else-if="node.kind === 'blocked_image'"
    :class="['md-blocked-image', { 'md-blocked-image--sized': node.width && node.height }]"
    :style="blockedImageStyle"
    role="img"
    :aria-label="$t('mail.markdown.imageBlocked', { alt: node.alt })"
    :title="$t('mail.markdown.imagesBlockedTitle')"
  ><span>{{ $t('mail.markdown.imageBlocked', { alt: node.alt }) }}</span></span>
</template>
