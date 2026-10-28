<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import type { Attachment, PgpMessageState } from '@/types/mail';
import { useImagePolicy } from '@/composables/useImagePolicy';
import { useSettingsStore } from '@/stores/settings';
import { recordFrontendDiagnostic } from '@/utils/diagnostics';
import { useLinkOpener } from '@/composables/useLinkOpener';
import {
  assessExternalLink,
  parseMailtoLink,
  type LinkAnchorRect,
  type LinkInteraction,
} from '@/utils/externalLinks';
import { countRemoteMarkdownImages, hasMeaningfulMarkdown } from '@/utils/safeMarkdown';
import type { QuotePart } from '@/utils/emailQuotes';
import { missingInlineImageSource } from '@/utils/emailInlineImages';
import { prepareEmailBody } from '@/utils/preparedEmailBody';
import { isPersonalMailboxDomain, senderDomain } from '@/utils/senderDomains';
import { splitRenderSegments, type RenderSegment } from '@/utils/emailSegments';
import { hasHoverCapablePointer } from '@/utils/pointerCapabilities';
import ExternalLinkDialog from './ExternalLinkDialog.vue';
import LinkDestinationPreview, { type LinkPreviewStatus } from './LinkDestinationPreview.vue';
import PlainTextLinks from './PlainTextLinks.vue';
import SafeMarkdownBody from './SafeMarkdownBody.vue';
import IsolatedEmailDocument from './IsolatedEmailDocument.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import { useUiStore } from '@/stores/ui';
import { formattingFreeText, isFormattingFreeHtml } from '@/utils/formattingFreeHtml';

const settings = useSettingsStore();
const ui = useUiStore();
const { t } = useI18n();

const props = withDefaults(defineProps<{
  html?: string;
  text?: string;
  pgpState?: PgpMessageState | null;
  /** Only a host with a connected decrypt action may offer the button. */
  canDecrypt?: boolean;
  decrypting?: boolean;
  senderEmail?: string | null;
  trustedDmarcPass?: boolean;
  attachments?: Attachment[];
  normalizeQuotes?: boolean;
  showViewModeToggle?: boolean;
  /** Shares the parsed body across remounts; the message id where there is one. */
  cacheKey?: string;
}>(), {
  attachments: () => [],
  normalizeQuotes: false,
  showViewModeToggle: true,
});
const emit = defineEmits<{
  (e: 'request-unlock'): void;
}>();

const imagePolicy = useImagePolicy();
const showBlockedImages = ref(false);
const quotesExpanded = ref(false);
/**
 * How the body of the current message is rendered. One flat dimension instead
 * of separate HTML/plain toggles: switching to a text mode is the common
 * escape hatch when HTML renders badly, and both reference clients offer it.
 * Resets to a per-message default derived from the global preferences.
 */
type ViewMode = 'html' | 'auto-dark' | 'markdown' | 'text';
const viewMode = ref<ViewMode>('text');

function setViewMode(value: string) {
  viewMode.value = value as ViewMode;
}
const linkOpener = useLinkOpener({
  onOpenFailed: (href, anchor) => showLinkError(href, anchor, t('mail.link.browserFailed')),
});
const pendingExternalLink = linkOpener.pending;
type LinkPreviewState = {
  href: string;
  status: LinkPreviewStatus;
  anchor: LinkAnchorRect;
  detail?: string;
};
const hoveredLink = ref<LinkPreviewState | null>(null);
const linkError = ref<LinkPreviewState | null>(null);
let linkErrorTimer: ReturnType<typeof setTimeout> | null = null;
const visibleLinkPreview = computed(() => linkError.value ?? hoveredLink.value);
/**
 * Touch taps synthesize a mouseover before the click but never a matching
 * mouseout, so a hover preview would flash on every tap and stick afterwards.
 * Devices without a real pointer get no hover previews at all.
 */
const hoverCapable = hasHoverCapablePointer();

const senderTrusted = computed(() => imagePolicy.isAllowed(props.senderEmail, props.trustedDmarcPass));
/**
 * In the `accepted` policy mode, the new-sender review's verdict also unlocks
 * images: whoever was accepted into the inbox is trusted enough to load from.
 * The policy module stays store-free, so the lookup happens here.
 */
const senderAccepted = computed(() => {
  if (imagePolicy.state.mode !== 'accepted') return false;
  const email = props.senderEmail?.trim().toLowerCase();
  if (!email) return false;
  return (
    Boolean(settings.acceptedSenders[email]) ||
    (props.trustedDmarcPass && Boolean(settings.acceptedDomains[senderDomain(email)]))
  );
});
const imagesEffectivelyAllowed = computed(
  () => senderTrusted.value || senderAccepted.value || showBlockedImages.value,
);

watch(
  () => [props.html, props.senderEmail],
  () => {
    showBlockedImages.value = false;
    quotesExpanded.value = false;
  },
);

watch(
  () => props.text,
  () => {
    pendingExternalLink.value = null;
    hoveredLink.value = null;
    linkError.value = null;
  },
);

function showImagesOnce() {
  showBlockedImages.value = true;
}
function trustSender() {
  if (!props.senderEmail) return;
  imagePolicy.allow(props.senderEmail);
}

const senderDomainName = computed(() =>
  props.senderEmail ? senderDomain(props.senderEmail) : '',
);
/**
 * A domain-wide allow only makes sense where the domain identifies the
 * organisation. On shared mailbox providers (gmail.com, web.de, …) it would
 * wave through every stranger with an address there, so the action is not
 * offered at all.
 */
const canTrustDomain = computed(
  () => senderDomainName.value !== '' && !isPersonalMailboxDomain(senderDomainName.value),
);
function trustDomain() {
  if (!canTrustDomain.value) return;
  imagePolicy.allowDomain(senderDomainName.value);
}

const blockedImagesLabel = computed(() =>
  t('mail.imagePolicy.blocked', { count: blockedImagesCount.value }),
);

function onQuotesToggle(event: Event) {
  const expanded = (event.currentTarget as HTMLDetailsElement).open;
  quotesExpanded.value = expanded;
  recordFrontendDiagnostic(
    'quotes.toggle',
    `expanded=${expanded} quote_count=${extractedBody.value.quotes.length}`,
  );
}

const missingImageSource = computed(() => missingInlineImageSource(t('errors.inlineImageUnavailable')));

const prepared = computed(() => prepareEmailBody(props.cacheKey, {
  html: props.html,
  text: props.text,
  attachments: props.attachments,
  normalizeQuotes: props.normalizeQuotes,
  missingImageSource: missingImageSource.value,
}));
// Keeps the body's inline image URLs valid while this renderer shows it.
watch(prepared, (body, _previous, onCleanup) => onCleanup(body.retain()), { immediate: true });

const extractedBody = computed(() => prepared.value.extracted);
const renderHtml = computed(() => extractedBody.value.currentHtml || '');
const quoteParts = computed(() => extractedBody.value.quotes);

const shouldAutoDarkHtml = computed(() =>
  settings.theme === 'dark' && settings.autoDarkHtmlMails,
);

const documentOptions = computed(() => ({
  autoDark: viewMode.value === 'auto-dark',
  allowRemoteImages: imagesEffectivelyAllowed.value,
}));

/* One pass per message, view mode and image permission; see preparedEmailBody. */
const preparedDocument = computed(() => (
  renderHtml.value ? prepared.value.document(documentOptions.value) : null
));

const isolatedDocument = computed(() => {
  if (preferPlainText.value) {
    return '';
  }
  return preparedDocument.value?.html ?? '';
});

// Custom properties do not cross the iframe boundary. Resolve the reader's
// palette after the theme store applies it, for messages and quoted documents.
const documentPalette = computed(() => {
  if (viewMode.value !== 'auto-dark') return undefined;
  void settings.theme;
  const style = getComputedStyle(document.documentElement);
  return {
    canvas: style.getPropertyValue('--bg-surface').trim(),
    ink: style.getPropertyValue('--text-primary').trim(),
  };
});

function quoteIsolatedDocument(part: QuotePart): string {
  return part.html ? prepared.value.quoteDocument(part, documentOptions.value) : '';
}

const preferPlainText = computed(
  () => viewMode.value === 'markdown' || viewMode.value === 'text',
);

/**
 * Text to show when HTML is suppressed. Messages that only carry HTML have no
 * text/plain alternative, so derive a readable version rather than showing a
 * blank body.
 */
const plainTextCandidate = computed(() => prepared.value.plainText);

// HTML that only wraps its text in paragraphs reads as the text it is.
const formattingFree = computed(() => isFormattingFreeHtml(renderHtml.value));

const displayText = computed(() => {
  if (!preferPlainText.value) return extractedBody.value.currentText;
  return formattingFree.value ? formattingFreeText(renderHtml.value) : plainTextCandidate.value;
});

/** Only offer the toggle when there is actually HTML to switch away from. */
const htmlAvailable = computed(() => Boolean(renderHtml.value.trim()) && !formattingFree.value);
/**
 * Markdown is detected in the text view. An HTML message only has one once the
 * reader switches to a text mode, so the option joins the toggle then: asking
 * up front cost a further parse of the HTML and a Markdown lex of up to 250 KB
 * on every open, for an option that is rarely there.
 */
const markdownAvailable = computed(() => (
  (!htmlAvailable.value || preferPlainText.value) && prepared.value.markdownAvailable
));
const blockedImagesCount = computed(() => {
  if (imagesEffectivelyAllowed.value) return 0;
  if (viewMode.value === 'markdown' && markdownAvailable.value) {
    return countRemoteMarkdownImages(displayText.value);
  }
  if (viewMode.value === 'html' || viewMode.value === 'auto-dark') {
    return preparedDocument.value?.blockedCount ?? 0;
  }
  return 0;
});

const viewModeOptions = computed(() => {
  const options: Array<{ value: ViewMode; label: string; title: string }> = [];
  if (htmlAvailable.value) {
    options.push({
      value: 'html',
      label: t('mail.viewMode.original'),
      title: t('mail.viewMode.originalTitle'),
    });
    if (settings.theme === 'dark') {
      options.push({
        value: 'auto-dark',
        label: t('mail.viewMode.autoDark'),
        title: t('mail.viewMode.autoDarkTitle'),
      });
    }
  }
  if (markdownAvailable.value) {
    options.push({
      value: 'markdown',
      label: t('mail.viewMode.markdown'),
      title: t('mail.viewMode.markdownTitle'),
    });
  }
  options.push({
    value: 'text',
    label: t('mail.viewMode.text'),
    title: t('mail.viewMode.textTitle'),
  });
  return options;
});

watch(
  () => [
    props.html,
    props.text,
    settings.theme,
    settings.autoDarkHtmlMails,
    settings.preferPlainTextReading,
  ],
  () => {
    if (!htmlAvailable.value || settings.preferPlainTextReading) {
      // Text is what will show, so its Markdown check is due either way.
      viewMode.value = prepared.value.markdownAvailable ? 'markdown' : 'text';
    } else {
      viewMode.value = shouldAutoDarkHtml.value ? 'auto-dark' : 'html';
    }
  },
  { immediate: true },
);

/**
 * Opening feedback beyond the click itself is noise: the browser takes over
 * immediately, and a checkmark restating that adds nothing. Only failure is
 * worth a card, since nothing else will tell the user.
 */
function showLinkError(href: string, anchor: LinkAnchorRect, detail: string) {
  if (linkErrorTimer) clearTimeout(linkErrorTimer);
  linkError.value = { href, status: 'error', anchor, detail };
  linkErrorTimer = setTimeout(() => {
    linkError.value = null;
    linkErrorTimer = null;
  }, 3_200);
}

function handleLinkHover(payload: LinkInteraction | null) {
  if (!payload) {
    hoveredLink.value = null;
    return;
  }
  if (!hoverCapable) return;
  const mailto = parseMailtoLink(payload.href);
  if (mailto) {
    hoveredLink.value = {
      href: mailto.href,
      status: 'hover',
      anchor: payload.anchor,
      detail: t('mail.link.newEmail'),
    };
    return;
  }
  const assessment = assessExternalLink(payload.href, payload.label);
  hoveredLink.value = {
    href: assessment.allowed ? assessment.href : payload.href,
    status: 'hover',
    anchor: payload.anchor,
    detail: assessment.requiresConfirmation ? t('mail.link.checkTarget') : undefined,
  };
}

function requestExternalLink(payload: LinkInteraction) {
  // A tap never produces the mouseout that clears the hover preview, so the
  // click itself has to retire it — on desktop it also reads as acknowledgement.
  hoveredLink.value = null;
  linkOpener.open(payload);
}

onBeforeUnmount(() => {
  if (linkErrorTimer) clearTimeout(linkErrorTimer);
});

const segments = computed<RenderSegment[]>(() => splitRenderSegments(displayText.value));

const hasPgpSegments = computed(() =>
  segments.value.some((segment) => segment.kind !== 'text'),
);

</script>

<template>
  <div class="renderer">
    <div
      v-if="blockedImagesCount > 0 && !imagesEffectivelyAllowed"
      class="image-guard"
      role="status"
    >
      <svg
        class="image-guard__icon"
        aria-hidden="true"
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <path d="M8 1.9 13 3.7v4c0 3.2-2.1 5.3-5 6.4-2.9-1.1-5-3.2-5-6.4v-4L8 1.9Z" />
      </svg>
      <span
        class="image-guard__text"
        :title="$t('mail.imagePolicy.shieldTitle')"
      >
        {{ blockedImagesLabel }}
      </span>
      <AppButton
        class="image-guard__action image-guard__action--primary"
        :title="$t('mail.imagePolicy.showOnceTitle')"
        @click="showImagesOnce"
        variant="link"
        size="sm"
      >
        {{ $t('mail.imagePolicy.showOnce') }}
      </AppButton>
      <template v-if="props.senderEmail">
        <span class="image-guard__sep" aria-hidden="true">·</span>
        <AppButton
          class="image-guard__action"
          :title="$t('mail.imagePolicy.allowSenderTitle', { email: props.senderEmail })"
          @click="trustSender"
          variant="link"
          size="sm"
        >
          {{ $t('mail.imagePolicy.allowSender') }}
        </AppButton>
      </template>
      <template v-if="canTrustDomain">
        <span class="image-guard__sep" aria-hidden="true">·</span>
        <AppButton
          class="image-guard__action"
          :title="$t('mail.imagePolicy.allowDomainTitle', { domain: senderDomainName })"
          @click="trustDomain"
          variant="link"
          size="sm"
        >
          {{ $t('mail.imagePolicy.allowDomain', { domain: senderDomainName }) }}
        </AppButton>
      </template>
    </div>
    <div v-if="isolatedDocument" class="html-document-group">
      <SegmentedControl
        v-if="props.showViewModeToggle && viewModeOptions.length > 1"
        class="view-mode-toggle"
        :options="viewModeOptions"
        :model-value="viewMode"
        :aria-label="$t('mail.viewMode.label')"
        @update:model-value="setViewMode"
      />
      <div :class="['html-document-shell', { 'html-document-shell--auto-dark': viewMode === 'auto-dark' }]">
        <IsolatedEmailDocument
          :html="isolatedDocument"
          :canvas-color="documentPalette?.canvas"
          :text-color="documentPalette?.ink"
          :allow-remote-images="imagesEffectivelyAllowed"
          :zoom="settings.readerZoom"
          :flush="ui.isMobile"
          @open-link="requestExternalLink"
          @link-hover="handleLinkHover"
        />
      </div>
    </div>
    <div v-else-if="!hasPgpSegments" class="plain-document-group">
      <SegmentedControl
        v-if="props.showViewModeToggle && viewModeOptions.length > 1"
        class="view-mode-toggle"
        :options="viewModeOptions"
        :model-value="viewMode"
        :aria-label="$t('mail.viewMode.label')"
        @update:model-value="setViewMode"
      />
      <SafeMarkdownBody
        v-if="markdownAvailable && viewMode === 'markdown'"
        class="text"
        :text="displayText"
        :allow-remote-images="imagesEffectivelyAllowed"
        @open-link="requestExternalLink"
        @link-hover="handleLinkHover"
      />
      <PlainTextLinks
        v-else
        class="text"
        :text="displayText"
        @open-link="requestExternalLink"
        @link-hover="handleLinkHover"
      />
    </div>
    <div v-else class="text-layout">
      <template v-for="(segment, index) in segments" :key="index">
        <PlainTextLinks
          v-if="segment.kind === 'text' && segment.content.trim()"
          class="text text-segment"
          :text="segment.content"
          @open-link="requestExternalLink"
          @link-hover="handleLinkHover"
        />
        <section v-else-if="segment.kind === 'pgp_decrypted'" class="pgp-segment">
          <header class="pgp-segment-header">{{ $t('mail.pgpBlock.decryptedPgp') }}</header>
          <pre class="pgp-segment-body">{{ segment.content }}</pre>
        </section>
        <section v-else-if="segment.kind === 'smime_decrypted'" class="pgp-segment">
          <header class="pgp-segment-header">{{ $t('mail.pgpBlock.decryptedSmime') }}</header>
          <pre class="pgp-segment-body">{{ segment.content }}</pre>
        </section>
        <section v-else-if="segment.kind === 'pgp_encrypted'" class="pgp-segment encrypted">
          <header class="pgp-segment-header">{{ $t('mail.pgpBlock.encryptedPgp') }}</header>
          <p class="pgp-segment-hint">
            {{ props.canDecrypt ? $t('mail.pgpBlock.decryptHint') : $t('mail.pgpBlock.encryptedHint') }}
          </p>
          <AppButton
            v-if="props.canDecrypt"
            class="pgp-segment-action"
            :disabled="props.decrypting"
            @click="emit('request-unlock')"
            variant="primary"
            size="sm"
          >
            {{ props.decrypting ? $t('mail.pgp.decrypting') : $t('mail.pgpBlock.decryptAction') }}
          </AppButton>
          <pre class="pgp-segment-raw">{{ segment.content }}</pre>
        </section>
      </template>
    </div>
    <section v-if="quoteParts.length > 0" class="quote-timeline" :aria-label="$t('mail.quotes.timelineLabel')">
      <details
        class="quote-part"
        @toggle="onQuotesToggle"
      >
        <summary
          class="quote-summary"
          :title="$t('mail.quotes.show', { count: quoteParts.length })"
          :aria-label="$t('mail.quotes.show', { count: quoteParts.length })"
        >
          <span class="quote-ellipsis" aria-hidden="true">•••</span>
          <span class="quote-title">{{ $t('mail.quotes.count', { count: quoteParts.length }) }}</span>
        </summary>
        <div v-if="quotesExpanded" class="quote-body">
          <article v-for="part in quoteParts" :key="part.id" class="quote-message">
            <header class="quote-message-title">
              <span>{{ part.label }}</span>
              <span v-if="part.duplicateCount > 1">{{ $t('mail.quotes.duplicates', { count: part.duplicateCount }) }}</span>
            </header>
            <IsolatedEmailDocument
              v-if="part.html"
              :html="quoteIsolatedDocument(part)"
              :canvas-color="documentPalette?.canvas"
              :text-color="documentPalette?.ink"
              :allow-remote-images="imagesEffectivelyAllowed"
              :zoom="settings.readerZoom"
              pass-through-touch-scroll
              @open-link="requestExternalLink"
              @link-hover="handleLinkHover"
            />
            <SafeMarkdownBody
              v-else-if="hasMeaningfulMarkdown(part.text)"
              class="quote-message-body"
              :text="part.text"
              :allow-remote-images="imagesEffectivelyAllowed"
              @open-link="requestExternalLink"
              @link-hover="handleLinkHover"
            />
            <PlainTextLinks
              v-else
              class="quote-message-body"
              :text="part.text"
              @open-link="requestExternalLink"
              @link-hover="handleLinkHover"
            />
          </article>
        </div>
      </details>
    </section>
    <ExternalLinkDialog
      :assessment="pendingExternalLink"
      @cancel="linkOpener.cancel"
      @confirm="linkOpener.confirm"
    />
    <LinkDestinationPreview
      v-if="visibleLinkPreview"
      :href="visibleLinkPreview.href"
      :status="visibleLinkPreview.status"
      :anchor="visibleLinkPreview.anchor"
      :detail="visibleLinkPreview.detail"
    />
  </div>
</template>

<style scoped>
.plain-document-group,
.quote-message-body {
  font-size: calc(1em * var(--reader-zoom, 1));
}

.view-mode-toggle {
  justify-self: start;
}

.renderer {
  min-width: 0;
  max-width: 100%;
  width: 100%;
}

.text-layout {
  display: grid;
  gap: 12px;
}

.plain-document-group {
  display: grid;
  min-width: 0;
  max-width: 100%;
  gap: 10px;
}

/*
  A text/plain body is a letter, not a terminal. The element is a <pre> for its
  whitespace, and a <pre> brings the monospace face along; nothing in the
  message asks for that (format=fixed is the RFC 3676 default, not a choice),
  so it reads in the app's face, at the HTML body's line height, and wraps at a
  line length the eye can follow back. `ch` is the width of a zero, wider than
  the average letter of the sans face: 60ch comes to roughly 85 characters.
*/
.text {
  max-width: 60ch;
  margin: 0;
  font-family: var(--font-sans);
  font-size: var(--font-size-lg);
  line-height: 1.55;
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.html-document-group {
  display: grid;
  gap: 8px;
}

.html-document-shell {
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  border-radius: var(--radius-md);
  background: var(--email-canvas);
  padding: 0;
}

/*
  The frame goes on a phone, the canvas stays.
  This is the second card around a message: the first sat around the whole
  bubble and went with the same reasoning, this one wraps the HTML document
  inside it. A border and a radius separate a document from things beside it,
  and on a phone there is nothing beside it — the message is the screen, and
  the frame only narrows it.
  The background is not decoration and must not go with the rest: mail is
  written against a light canvas and assumes one, so removing it would leave
  dark text on a dark surface for any sender who set a colour and trusted the
  default for the rest.
*/
@media (max-width: 767px) {
  .html-document-shell {
    border: 0;
    border-radius: 0;
    background: transparent;
  }
}

.html-document-shell--auto-dark {
  border-color: transparent;
  background: var(--bg-surface);
}

.text {
  white-space: pre-wrap;
  background: transparent !important;
}

.text-segment {
  margin: 0;
}

.quote-timeline {
  display: grid;
  gap: 8px;
  margin-top: 14px;
}

.quote-part {
  width: fit-content;
  max-width: 100%;
  border: 0;
  background: transparent;
}

.quote-summary {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  padding: 3px 8px 4px;
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
  color: var(--text-secondary);
  cursor: pointer;
  list-style: none;
}

.quote-summary::-webkit-details-marker {
  display: none;
}

.quote-ellipsis {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-bold);
  letter-spacing: 1px;
  line-height: 1;
}

.quote-title {
  display: none;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-size-sm);
}

.quote-part[open] {
  width: 100%;
}

.quote-part[open] .quote-title {
  display: block;
}

.quote-body {
  margin-top: 8px;
  display: grid;
  gap: 12px;
  padding: 12px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-base) 75%, var(--bg-surface));
}

.quote-message {
  min-width: 0;
}

.quote-message + .quote-message {
  padding-top: 12px;
  border-top: 1px solid var(--border-color-muted);
}

.quote-message-title {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 8px;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.quote-message-body {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-md);
  line-height: 1.55;
}

.pgp-segment {
  border: 1px solid color-mix(in srgb, var(--accent-green) 45%, var(--border-color));
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-green) 10%, transparent);
  overflow: hidden;
}

.pgp-segment-header {
  margin: 0;
  padding: 6px 10px;
  font-size: var(--font-size-xs);
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: color-mix(in srgb, var(--accent-green) 75%, white);
  border-bottom: 1px solid color-mix(in srgb, var(--accent-green) 25%, var(--border-color));
}

.pgp-segment-body {
  margin: 0;
  padding: 10px;
  white-space: pre-wrap;
  font-size: var(--font-size-md);
  line-height: 1.55;
  color: var(--text-primary);
}

.pgp-segment.encrypted {
  border-color: color-mix(in srgb, var(--accent-primary) 35%, var(--border-color));
  background: color-mix(in srgb, var(--accent-primary) 8%, transparent);
}

.pgp-segment.encrypted .pgp-segment-header {
  color: color-mix(in srgb, var(--accent-primary) 75%, white);
  border-bottom-color: color-mix(in srgb, var(--accent-primary) 25%, var(--border-color));
}

.pgp-segment-hint {
  margin: 0;
  padding: 10px 10px 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.pgp-segment-action { margin:10px 10px 0; width:fit-content; }

.pgp-segment-raw {
  margin: 10px;
  padding: 10px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-overlay) 55%, transparent);
  color: var(--text-tertiary);
  white-space: pre-wrap;
  font-size: var(--font-size-xs);
}

/* One quiet line above the document, in the invitation card's language: no
   box, no background, underlined actions. It states a fact and offers a way
   out — it does not compete with the message for attention or height. */
.image-guard {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-2xs) var(--space-sm);
  margin-bottom: var(--space-sm);
  font-size: var(--font-size-sm);
}

.image-guard__icon {
  flex-shrink: 0;
  width: 13px;
  height: 13px;
  align-self: center;
  color: var(--text-tertiary);
}

.image-guard__text {
  color: var(--text-secondary);
}

.image-guard__sep {
  color: var(--text-tertiary);
}
</style>
