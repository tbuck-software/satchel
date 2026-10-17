<script lang="ts">
/** The frame a click last went into; quoted parts render frames of their own. */
let lastClickedFrame: HTMLIFrameElement | null = null;
</script>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import type { LinkInteraction } from '@/utils/externalLinks';
import { visibleLinkText } from '@/utils/linkLabel';
import { buildEmailCsp } from '@/utils/emailCsp';

const props = withDefaults(defineProps<{
  html: string;
  /** Resolved reader tokens for the adapted HTML view, across the iframe boundary. */
  canvasColor?: string;
  textColor?: string;
  /**
   * Mirrors the per-sender image policy. Drives the `img-src`/`media-src`/
   * `font-src` part of the CSP below — the actual enforcement of the
   * remote-content policy.
   */
  allowRemoteImages?: boolean;
  /** Reader text scale in percent; applied inside the isolated document. */
  zoom?: number;
  /** Mobile reading canvas: no nested card background or desktop document gutter. */
  flush?: boolean;
  /** Let a parent scroller own touch drags over embedded quoted history. */
  passThroughTouchScroll?: boolean;
}>(), {
  allowRemoteImages: false,
  zoom: 100,
  flush: false,
  passThroughTouchScroll: false,
});

const emit = defineEmits<{
  'open-link': [payload: LinkInteraction];
  'link-hover': [payload: LinkInteraction | null];
}>();

/**
 * How long a freshly navigated frame keeps being re-measured, in animation
 * frames (~10s at 60Hz). Long enough for a mail whose images trickle in,
 * bounded so a document that never settles cannot keep the loop alive.
 */
const SETTLE_MAX_FRAMES = 600;
/** Consecutive frames of unchanged height that count as settled. */
const SETTLE_STABLE_FRAMES = 3;

/**
 * A hover and click target for one line box of one mail link, expressed in the
 * frame's viewport coordinates.
 */
type LinkHotspot = {
  key: string;
  href: string;
  /** Read on interaction: the visible label needs computed styles, too costly per refresh. */
  anchor: HTMLAnchorElement;
  left: number;
  top: number;
  width: number;
  height: number;
};

const frameRef = ref<HTMLIFrameElement | null>(null);
const hostRef = ref<HTMLElement | null>(null);
const linkHotspots = ref<LinkHotspot[]>([]);
let hotspotFrameId: number | null = null;
let frameDocument: Document | null = null;
let contentResizeObserver: ResizeObserver | null = null;
let frameResizeObserver: ResizeObserver | null = null;
let resizeFrameId: number | null = null;
let settleFrameId: number | null = null;
let settleFramesLeft = 0;
let stableHeightFrames = 0;
let lastSettledHeight = '';
let observedFrameWidth = 0;

// Hex on purpose: this CSS is injected into the iframe srcdoc, a separate
// document that never sees the app's custom properties.
const shellCss = `
  html,
  body {
    box-sizing: border-box;
    width: 100%;
    max-width: 100%;
    margin: 0;
    padding: 0;
  }

  html {
    background: transparent;
  }

  body {
    overflow-x: auto;
    overflow-y: hidden;
  }

  .satchel-email-body {
    box-sizing: border-box;
    width: 100%;
    max-width: 100%;
    margin: 0;
    padding: 0;
    overflow-wrap: normal;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    color: #202124;
    background: #fff;
  }

  .satchel-email-body[data-satchel-simple="on"] {
    padding: 24px 26px 28px !important;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    line-height: 1.55;
    overflow-wrap: anywhere;
  }

  .satchel-email-body[data-satchel-simple="on"] * {
    max-width: 100%;
    overflow-wrap: anywhere;
  }

  img,
  picture,
  video,
  canvas,
  svg {
    max-width: 100% !important;
    height: auto !important;
  }

  .satchel-email-body[data-satchel-auto-dark="on"] {
    color-scheme: dark;
    color: var(--satchel-email-ink) !important;
    background: var(--satchel-email-canvas) !important;
  }

  .satchel-email-body[data-satchel-auto-dark="on"][data-satchel-simple="on"] [style*="border"] {
    border-color: #525862 !important;
  }

  a[data-satchel-link-risk="true"] {
    text-decoration-style: wavy !important;
    text-decoration-color: #d29922 !important;
  }

  a[data-satchel-link-risk="true"]::after {
    content: " ⚠";
    color: #d29922;
    font-size: 0.72em;
    vertical-align: 0.15em;
  }

  a:active {
    background: rgba(37, 112, 235, 0.2) !important;
    outline: 2px solid rgba(37, 112, 235, 0.32);
    outline-offset: 1px;
  }

  a[aria-disabled="true"] {
    color: inherit !important;
    cursor: not-allowed;
    text-decoration: line-through !important;
  }
`;

// Inline importance keeps sender html/body rules from repainting the reader
// canvas, while coloured panels inside the message retain their own fills.
const canvasStyle = computed(() => props.canvasColor
  ? ' style="background: var(--satchel-email-canvas) !important"'
  : '');

const sourceDocument = computed(() => `<!doctype html>
<html${canvasStyle.value}>
  <head>
    <meta http-equiv="Content-Security-Policy" content="${buildEmailCsp(props.allowRemoteImages)}">
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>${shellCss}</style>
    ${props.canvasColor ? `<style>
    html {
      --satchel-email-canvas: ${props.canvasColor};
      --satchel-email-ink: ${props.textColor};
    }
  </style>` : ''}
    ${props.flush ? `<style>
      .satchel-email-body[data-satchel-auto-dark="on"] {
        background: transparent !important;
      }
      .satchel-email-body[data-satchel-simple="on"] {
        padding: 8px 2px 16px !important;
      }
    </style>` : ''}
    <style>
      /* Scale the root font size rather than using a CSS transform: a
         transform would leave the layout box at the old size and clip the
         message. Senders using px stay unaffected, which is why the body
         font-size is scaled too. */
      html { font-size: ${props.zoom}%; }
      .satchel-email-body { font-size: ${props.zoom}%; }
    </style>
  </head>
  <body${canvasStyle.value}>${props.html}</body>
</html>`);

function linkFromEvent(event: Event): HTMLAnchorElement | null {
  const anchor = event.composedPath().find((target) => (
    typeof (target as Element | undefined)?.matches === 'function'
      && (target as Element).matches('a')
  ));
  return (anchor as HTMLAnchorElement | undefined) ?? null;
}

function linkInteraction(anchor: HTMLAnchorElement): LinkInteraction {
  const frameRect = frameRef.value?.getBoundingClientRect();
  const rect = anchor.getBoundingClientRect();
  const offsetLeft = frameRect?.left ?? 0;
  const offsetTop = frameRect?.top ?? 0;
  return {
    href: anchor.getAttribute('href') ?? '',
    label: visibleLinkText(anchor),
    anchor: {
      left: offsetLeft + rect.left,
      right: offsetLeft + rect.right,
      top: offsetTop + rect.top,
      bottom: offsetTop + rect.bottom,
    },
  };
}

function onFrameClick(event: Event) {
  const anchor = linkFromEvent(event);
  if (!anchor) return;
  event.preventDefault();
  emit('open-link', linkInteraction(anchor));
}

/**
 * Mirror every mail link as a transparent target in *this* document.
 *
 * WebKit runs no event listener registered on the document of a frame that is
 * sandboxed without `allow-scripts` — observers fire, events do not. In
 * WKWebView, which is what the app ships on, that left clicking a mail link and
 * hovering it doing nothing at all, while the same code worked in Chromium and
 * so in every test. The hotspots live outside the frame, so they work in both.
 *
 * One rect per line box, because a link that wraps needs a target per line. The
 * container itself stays `pointer-events: none` so selecting mail text keeps
 * working everywhere except on the links themselves.
 *
 * Anchors whose href `buildIsolatedDocument` removed are skipped: a link the
 * assessment rejected has to stay inert.
 */
function collectLinkHotspots() {
  hotspotFrameId = null;
  const doc = frameRef.value?.contentDocument;
  if (!doc) {
    linkHotspots.value = [];
    return;
  }

  const next: LinkHotspot[] = [];
  for (const [index, anchor] of Array.from(doc.querySelectorAll('a')).entries()) {
    const href = anchor.getAttribute('href');
    if (!href) continue;
    for (const [rectIndex, rect] of Array.from(anchor.getClientRects()).entries()) {
      if (rect.width < 1 || rect.height < 1) continue;
      next.push({
        key: `${index}-${rectIndex}`,
        href,
        anchor,
        left: rect.left,
        top: rect.top,
        width: rect.width,
        height: rect.height,
      });
    }
  }
  linkHotspots.value = next;
}

function scheduleHotspotRefresh() {
  if (hotspotFrameId !== null) return;
  hotspotFrameId = requestAnimationFrame(collectLinkHotspots);
}

function hotspotInteraction(hotspot: LinkHotspot): LinkInteraction {
  const frameRect = frameRef.value?.getBoundingClientRect();
  const offsetLeft = frameRect?.left ?? 0;
  const offsetTop = frameRect?.top ?? 0;
  return {
    href: hotspot.href,
    label: visibleLinkText(hotspot.anchor),
    anchor: {
      left: offsetLeft + hotspot.left,
      right: offsetLeft + hotspot.left + hotspot.width,
      top: offsetTop + hotspot.top,
      bottom: offsetTop + hotspot.top + hotspot.height,
    },
  };
}

function onFrameMouseOver(event: Event) {
  const anchor = linkFromEvent(event);
  if (!anchor) return;
  emit('link-hover', linkInteraction(anchor));
}

function onFrameMouseOut(event: MouseEvent) {
  const anchor = linkFromEvent(event);
  if (!anchor) return;
  const relatedTarget = event.relatedTarget as Node | null;
  if (relatedTarget && typeof relatedTarget.nodeType === 'number' && anchor.contains(relatedTarget)) return;
  emit('link-hover', null);
}

function onFrameKeyDown(event: KeyboardEvent) {
  const frame = frameRef.value;
  if (!frame || event.defaultPrevented) return;
  const forwarded = new KeyboardEvent(event.type, {
    key: event.key,
    code: event.code,
    location: event.location,
    ctrlKey: event.ctrlKey,
    shiftKey: event.shiftKey,
    altKey: event.altKey,
    metaKey: event.metaKey,
    repeat: event.repeat,
    bubbles: true,
    cancelable: true,
  });
  if (!frame.dispatchEvent(forwarded)) {
    event.preventDefault();
  }
}

/**
 * Take the keyboard back after a click into the mail.
 *
 * The frame forwards keys through `onFrameKeyDown`, but WebKit never runs that
 * listener, so in WKWebView a click into the text left every shortcut dead
 * until the next click outside. The selection stays in the frame; `onCopy`
 * keeps Cmd+C working on it. Tab is left alone: whoever tabs into the mail's
 * links meant to go there.
 */
let lastTabAt = -Infinity;

function onWindowKeyDown(event: KeyboardEvent) {
  if (event.key === 'Tab') lastTabAt = performance.now();
}

function onWindowBlur() {
  // The parent's activeElement moves to the frame once the blur is through.
  window.setTimeout(() => {
    const frame = frameRef.value;
    if (!frame || document.activeElement !== frame) return;
    if (performance.now() - lastTabAt < 500) return;
    lastClickedFrame = frame;
    hostRef.value?.focus({ preventScroll: true });
  }, 0);
}

function frameSelectionText(): string {
  const frame = frameRef.value;
  if (!frame || frame !== lastClickedFrame) return '';
  const parentSelection = document.getSelection();
  if (parentSelection && !parentSelection.isCollapsed && parentSelection.toString()) return '';
  return frame.contentDocument?.getSelection()?.toString() ?? '';
}

/** WebKit only enables Copy for a document without a selection when this is cancelled. */
function onBeforeCopy(event: Event) {
  if (frameSelectionText()) event.preventDefault();
}

function onCopy(event: ClipboardEvent) {
  const text = frameSelectionText();
  if (!text || !event.clipboardData) return;
  const range = frameRef.value?.contentDocument?.getSelection()?.getRangeAt(0);
  const holder = document.createElement('div');
  if (range) holder.appendChild(range.cloneContents());
  event.clipboardData.setData('text/plain', text);
  if (holder.innerHTML) event.clipboardData.setData('text/html', holder.innerHTML);
  event.preventDefault();
}

onMounted(() => {
  window.addEventListener('keydown', onWindowKeyDown, true);
  window.addEventListener('blur', onWindowBlur);
  document.addEventListener('beforecopy', onBeforeCopy);
  document.addEventListener('copy', onCopy);
});

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onWindowKeyDown, true);
  window.removeEventListener('blur', onWindowBlur);
  document.removeEventListener('beforecopy', onBeforeCopy);
  document.removeEventListener('copy', onCopy);
  if (lastClickedFrame === frameRef.value) lastClickedFrame = null;
});

function resizeFrame() {
  if (resizeFrameId !== null) {
    cancelAnimationFrame(resizeFrameId);
    resizeFrameId = null;
  }
  const frame = frameRef.value;
  const doc = frame?.contentDocument;
  if (!frame || !doc?.body) return;
  const bodyHeight = Math.max(
    doc.body.scrollHeight,
    Math.ceil(doc.body.getBoundingClientRect().height),
  );
  const hasHorizontalOverflow = doc.documentElement.scrollWidth > doc.documentElement.clientWidth + 1;
  const nextHeight = Math.max(1, bodyHeight + (hasHorizontalOverflow ? 16 : 0));
  if (frame.style.height !== `${nextHeight}px`) {
    frame.style.height = `${nextHeight}px`;
    // Reflowing the mail moves its links, and the hotspots have to follow.
    scheduleHotspotRefresh();
  }
}

function scheduleFrameResize() {
  if (resizeFrameId !== null) return;
  resizeFrameId = requestAnimationFrame(resizeFrame);
}

function disconnectFrameDocument() {
  if (resizeFrameId !== null) {
    cancelAnimationFrame(resizeFrameId);
    resizeFrameId = null;
  }
  contentResizeObserver?.disconnect();
  contentResizeObserver = null;
  frameResizeObserver?.disconnect();
  frameResizeObserver = null;
  frameDocument?.removeEventListener('click', onFrameClick);
  frameDocument?.removeEventListener('mouseover', onFrameMouseOver);
  frameDocument?.removeEventListener('mouseout', onFrameMouseOut as EventListener);
  frameDocument?.removeEventListener('keydown', onFrameKeyDown);
  frameDocument?.removeEventListener('load', scheduleFrameResize, true);
  frameDocument?.removeEventListener('error', scheduleFrameResize, true);
  frameDocument = null;
}

function bindFrameDocument(doc: Document) {
  disconnectFrameDocument();
  const frame = frameRef.value;
  if (!frame) return;
  frameDocument = doc;
  doc.addEventListener('click', onFrameClick);
  doc.addEventListener('mouseover', onFrameMouseOver);
  doc.addEventListener('mouseout', onFrameMouseOut as EventListener);
  doc.addEventListener('keydown', onFrameKeyDown);
  // Images and webfonts arrive after parsing and change the layout when they
  // do. Neither event bubbles, hence the capture phase.
  doc.addEventListener('load', scheduleFrameResize, true);
  doc.addEventListener('error', scheduleFrameResize, true);

  contentResizeObserver = new ResizeObserver(() => {
    scheduleFrameResize();
    scheduleHotspotRefresh();
  });
  contentResizeObserver.observe(doc.body);
  contentResizeObserver.observe(doc.documentElement);

  observedFrameWidth = frame.getBoundingClientRect().width;
  frameResizeObserver = new ResizeObserver(([entry]) => {
    if (!entry) return;
    const nextWidth = entry.contentRect.width;
    if (Math.abs(nextWidth - observedFrameWidth) < 0.5) return;
    observedFrameWidth = nextWidth;
    scheduleFrameResize();
    scheduleHotspotRefresh();
  });
  frameResizeObserver.observe(frame);
  scheduleFrameResize();
  scheduleHotspotRefresh();
}

/**
 * The document carries mail content, rather than being the blank document an
 * iframe holds before its `srcdoc` navigation commits. `buildIsolatedDocument`
 * always wraps the body in an element, so a first child means the parser has
 * reached our content.
 */
function carriesMailContent(doc: Document | null | undefined): doc is Document {
  return Boolean(doc?.body?.firstElementChild);
}

/**
 * Take over the freshly navigated document and measure it until the height
 * holds still.
 *
 * Deliberately not driven by the frame's `load` event alone: that event waits
 * for *every* subresource of the mail, and a newsletter ships dozens of remote
 * images and webfonts. One request that never settles — a blocked tracker, a
 * CDN that stalls — means no `load`, so binding the observers there left the
 * frame at the 150px default the UA gives an unsized iframe, with the shell's
 * `overflow-y: hidden` cutting off everything below. The message looked empty
 * until some unrelated reflow happened to fire the observers minutes later.
 */
function settleFrame() {
  settleFrameId = null;
  settleFramesLeft -= 1;
  const frame = frameRef.value;
  const doc = frame?.contentDocument;
  // A `srcdoc` change replaces the document; rebind whenever that happens.
  if (carriesMailContent(doc) && doc !== frameDocument) {
    bindFrameDocument(doc);
  }
  resizeFrame();

  const height = frame?.style.height ?? '';
  const parsed = frameDocument !== null && frameDocument.readyState !== 'loading';
  if (parsed && height && height === lastSettledHeight) {
    stableHeightFrames += 1;
  } else {
    stableHeightFrames = 0;
  }
  lastSettledHeight = height;

  if (stableHeightFrames >= SETTLE_STABLE_FRAMES || settleFramesLeft <= 0) {
    // Final pass: the layout is done, so these rects are the ones that hold.
    scheduleHotspotRefresh();
    return;
  }
  settleFrameId = requestAnimationFrame(settleFrame);
}

function startSettleLoop() {
  stableHeightFrames = 0;
  lastSettledHeight = '';
  settleFramesLeft = SETTLE_MAX_FRAMES;
  if (settleFrameId !== null) return;
  settleFrameId = requestAnimationFrame(settleFrame);
}

onMounted(startSettleLoop);

// A new document means a new navigation to wait out; `post` so the iframe has
// the new `srcdoc` by the time the loop looks at it.
watch(sourceDocument, startSettleLoop, { flush: 'post' });

onBeforeUnmount(() => {
  if (settleFrameId !== null) {
    cancelAnimationFrame(settleFrameId);
    settleFrameId = null;
  }
  if (hotspotFrameId !== null) {
    cancelAnimationFrame(hotspotFrameId);
    hotspotFrameId = null;
  }
  disconnectFrameDocument();
});
</script>

<template>
  <div
    ref="hostRef"
    class="satchel-email-host"
    :class="{ 'satchel-email-host--pass-through-touch-scroll': props.passThroughTouchScroll }"
    tabindex="-1"
  >
    <iframe
      ref="frameRef"
      class="satchel-email-document"
      :title="$t('mail.document.frameTitle')"
      sandbox="allow-same-origin"
      referrerpolicy="no-referrer"
      :srcdoc="sourceDocument"
      @load="startSettleLoop"
    />
    <!-- The links are announced by the anchors inside the frame; these targets
         only carry the pointer, hence aria-hidden. -->
    <div class="satchel-email-hotspots" aria-hidden="true">
      <div
        v-for="hotspot in linkHotspots"
        :key="hotspot.key"
        class="satchel-email-hotspot"
        :style="{
          left: `${hotspot.left}px`,
          top: `${hotspot.top}px`,
          width: `${hotspot.width}px`,
          height: `${hotspot.height}px`,
        }"
        @mouseenter="emit('link-hover', hotspotInteraction(hotspot))"
        @mouseleave="emit('link-hover', null)"
        @click="emit('open-link', hotspotInteraction(hotspot))"
      />
    </div>
  </div>
</template>

<style scoped>
.satchel-email-host {
  position: relative;
  width: 100%;
  max-width: 100%;
}

.satchel-email-document {
  display: block;
  width: 100%;
  max-width: 100%;
  min-height: 1px;
  border: 0;
  background: transparent;
}

.satchel-email-hotspots {
  position: absolute;
  inset: 0;
  /* Only the hotspots themselves take the pointer, so text selection inside
     the mail keeps working. */
  pointer-events: none;
}

.satchel-email-hotspot {
  position: absolute;
  pointer-events: auto;
  cursor: pointer;
}

/*
  A vertically non-scrollable iframe still captures the whole swipe in Android
  WebView. Quoted history lives inside the thread's own scroller, so let that
  scroller receive coarse-pointer drags. Link hotspots remain interactive above
  the frame; only text selection inside quoted HTML is traded for stable phone
  scrolling. The primary message document keeps its normal pointer behaviour.
*/
@media (pointer: coarse) {
  .satchel-email-host--pass-through-touch-scroll .satchel-email-document {
    pointer-events: none;
  }
}
</style>
