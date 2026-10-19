<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import type { Thread } from '@/types/mail';
import type { BackendTopic } from '@/types/topics';
import type {
  NewSenderDomainSuggestion,
  NewSenderGroup,
  NewSenderStack,
  SmartInboxBucketPreviewItem,
  ThreadListEntry,
} from '@/stores/threads';
import Avatar from '@/components/common/Avatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import CountBadge from '@/components/ui/CountBadge.vue';
import FolderPreviewCard from '@/components/mail/FolderPreviewCard.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import SmartBucketIcon from '@/components/mail/SmartBucketIcon.vue';
import ThreadGroupCard from '@/components/mail/ThreadGroupCard.vue';
import ThreadListItem from '@/components/mail/ThreadListItem.vue';
import TopicsFolderCard from '@/components/topics/TopicsFolderCard.vue';
import ErrorState from '@/components/ui/ErrorState.vue';
import SkeletonRow from '@/components/ui/SkeletonRow.vue';
import { backendErrorLabel } from '@/utils/backendErrorLabels';

const props = defineProps<{
  items: ThreadListEntry[];
  activeThreadId?: string;
  selectedThreadIds: string[];
  activeGroupId?: string;
  focusedTargetKey?: string;
  keyboardNavigationActive?: boolean;
  emptyLabel?: string;
  loading?: boolean;
  /** The next page failed to load: the list ends here for a reason. */
  loadMoreFailed?: boolean;
  loadPreviousFailed?: boolean;
  hasPreviousThreads?: boolean;
  paginationBusy?: boolean;
  /** Raw backend text of a failed load; translated here before it is shown. */
  error?: string | null;
  /** First-sync progress. While it runs, an empty list is not an empty mailbox. */
  backfill?: { remaining: number; total: number } | null;
  senderReviewSetupComplete: boolean;
  bulkBusy: boolean;
  /** When set, the topics folder joins the smart folder section as its first card. */
  topics?: BackendTopic[];
}>();

const emit = defineEmits<{
  activate: [payload: { threadId: string; shiftKey: boolean; metaKey: boolean; ctrlKey: boolean; keyboard?: boolean }];
  check: [payload: { threadId: string; shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }];
  openGroup: [groupId: string];
  openBucket: [bucketId: string];
  openTopics: [];
  createTopic: [];
  openNewSenders: [];
  acceptExistingSenders: [];
  bucketAction: [payload: { threadIds: string[]; action: 'done' | 'read' }];
  groupDone: [threadIds: string[]];
  dateGroupDone: [payload: { groupId: string; threadIds: string[] }];
  newSenderAction: [payload: { sender: NewSenderGroup; action: 'accept' | 'block' }];
  acceptDomain: [suggestion: NewSenderDomainSuggestion];
  loadMore: [];
  loadPrevious: [];
  retryLoadMore: [];
  /** First row the virtualizer renders, so the store can trim what is far behind it. */
  viewportStart: [threadId: string];
  retry: [];
}>();
const { t } = useI18n();

const backfillRunning = computed(() => (props.backfill?.remaining ?? 0) > 0);
const backfillLabel = computed(() => {
  const remaining = props.backfill?.remaining ?? 0;
  const total = props.backfill?.total ?? 0;
  // Without a total the percentage would be invented; say what is known.
  if (total <= 0) return t('thread.list.backfillPlain', { remaining });
  return t('thread.list.backfill', { done: Math.max(total - remaining, 0), total });
});

const OVERSCAN_ROWS = 8;

/*
 * Row heights: the virtualizer adds these up, the markup has to render exactly
 * them. Carrying a number twice — here and again as a pixel class in a wrapper
 * — is what AGENTS.md calls out as silently breaking: change one and the
 * scrolling drifts a few pixels per row until clicks land on the wrong thread.
 * Every height below is bound as a custom property on the scroller root and
 * the wrappers reference that; `ThreadGroupList.heights.test.ts` holds the line.
 */
interface RowMetrics {
  thread: number;
  dateHeader: number;
  folderCard: number;
}

const DESK_ROWS: RowMetrics = { thread: 72, dateHeader: 36, folderCard: 80 };
/*
 * tokens.css lifts every font token by 2px at 1020px and below. Three-line
 * rows at 72px then had no room left: snippet and next sender touched.
 */
const PHONE_ROWS: RowMetrics = { thread: 88, dateHeader: 44, folderCard: 88 };
const PHONE_TYPE_QUERY = '(max-width: 1020px)';

const BUCKET_HEIGHT = 76;
/** Card header, and the "+N more" / domain-allow footer that matches it. */
const CARD_HEADER_HEIGHT = 40;
/** Section `py-3`, so twelve above and twelve below. */
const SECTION_PADDING = 24;
/** Card `pb-2`. */
const CARD_BOTTOM_PADDING = 8;
// The folder section carries no own title — it sits under the "Heute" header,
// whose bottom padding already separates the two.
const FOLDER_SECTION_TOP_INSET = 4;
const FOLDER_CARD_GAP = 6;
const FOLDER_SECTION_FOOTER_HEIGHT = 12;

const phoneTypeScale = ref(
  typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    && window.matchMedia(PHONE_TYPE_QUERY).matches,
);
let phoneTypeQuery: MediaQueryList | null = null;
function onPhoneTypeChange(event: MediaQueryListEvent) {
  phoneTypeScale.value = event.matches;
}

const rows = computed(() => (phoneTypeScale.value ? PHONE_ROWS : DESK_ROWS));
/** The review row sits inside a thread-height wrapper, inset two pixels a side. */
const reviewRowHeight = computed(() => rows.value.thread - 4);

/** The heights the markup needs, as custom properties on the scroller root. */
const rowHeights = computed(() => ({
  '--thread-row-h': `${rows.value.thread}px`,
  '--date-header-h': `${rows.value.dateHeader}px`,
  '--bucket-h': `${BUCKET_HEIGHT - 4}px`,
  '--folder-card-h': `${rows.value.folderCard}px`,
  '--folder-card-gap': `${FOLDER_CARD_GAP}px`,
  '--folder-section-inset': `${FOLDER_SECTION_TOP_INSET}px`,
  '--card-header-h': `${CARD_HEADER_HEIGHT}px`,
  '--review-row-h': `${reviewRowHeight.value}px`,
}));

const scrollerRef = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewportHeight = ref(0);
const showLoadingFeedback = ref(false);
let resizeObserver: ResizeObserver | null = null;
let scrollFrame: number | null = null;
let pendingScrollTop = 0;
let loadingFeedbackTimer: number | null = null;

watch(
  () => props.loading,
  (loading) => {
    if (loadingFeedbackTimer !== null) {
      window.clearTimeout(loadingFeedbackTimer);
      loadingFeedbackTimer = null;
    }
    if (!loading) {
      showLoadingFeedback.value = false;
      return;
    }
    // Match the documented loading grace period: quick cache hits should feel
    // immediate, while a real cold load still gets stable reserved geometry.
    loadingFeedbackTimer = window.setTimeout(() => {
      loadingFeedbackTimer = null;
      showLoadingFeedback.value = true;
    }, 160);
  },
  { immediate: true },
);

const emptyLabel = computed(() => {
  if (props.emptyLabel) return props.emptyLabel;
  return props.activeGroupId ? t('thread.list.emptyGroup') : t('thread.list.emptyView');
});

// An empty topics folder stays out of the smart folder section entirely.
const topicsCardVisible = computed(() => Boolean(props.topics?.length));

function itemHeight(item: ThreadListEntry): number {
  const { thread, dateHeader, folderCard } = rows.value;
  if (item.kind === 'folder-row') {
    const cardCount = item.folders.length + (topicsCardVisible.value ? 1 : 0);
    return FOLDER_SECTION_TOP_INSET
      + cardCount * folderCard
      + Math.max(0, cardCount - 1) * FOLDER_CARD_GAP
      + FOLDER_SECTION_FOOTER_HEIGHT;
  }
  if (item.kind === 'priority-card') {
    // Rows add a thread height each, the optional "+N weitere" footer a card header.
    return SECTION_PADDING + CARD_HEADER_HEIGHT + CARD_BOTTOM_PADDING
      + item.threads.length * thread
      + (item.hiddenCount > 0 ? CARD_HEADER_HEIGHT : 0);
  }
  if (item.kind === 'date-header') return dateHeader;
  if (item.kind === 'bucket') return BUCKET_HEIGHT;
  if (item.kind === 'new-sender-stack') {
    return SECTION_PADDING + CARD_HEADER_HEIGHT + thread + CARD_BOTTOM_PADDING
      + (hasNewSenderFooter(item.stack) ? CARD_HEADER_HEIGHT : 0);
  }
  return thread;
}

function hasNewSenderFooter(stack: NewSenderStack): boolean {
  return Boolean(stack.domainSuggestion) || stack.count > 1;
}

const itemOffsets = computed(() => {
  let offset = 0;
  return props.items.map((item) => {
    const current = offset;
    offset += itemHeight(item);
    return current;
  });
});

const totalHeight = computed(() =>
  props.items.reduce((sum, item) => sum + itemHeight(item), 0),
);

function findStartIndex(top: number): number {
  const offsets = itemOffsets.value;
  let low = 0;
  let high = offsets.length - 1;
  let result = 0;

  while (low <= high) {
    const mid = Math.floor((low + high) / 2);
    const offset = offsets[mid];
    const entry = props.items[mid];
    if (offset === undefined || !entry) break;
    const itemBottom = offset + itemHeight(entry);
    if (itemBottom < top) {
      low = mid + 1;
    } else {
      result = mid;
      high = mid - 1;
    }
  }

  return Math.max(0, result - OVERSCAN_ROWS);
}

const startIndex = computed(() => findStartIndex(scrollTop.value));
const endIndex = computed(() => {
  const bottom = scrollTop.value + viewportHeight.value;
  let index = startIndex.value;
  while (index < props.items.length && (itemOffsets.value[index] ?? Infinity) < bottom) {
    index += 1;
  }
  return Math.min(props.items.length, index + OVERSCAN_ROWS);
});
type ThreadEntry = Extract<ThreadListEntry, { kind: 'thread' }>;

/*
 * A listbox may only own options. Each run of consecutive thread rows is its
 * own listbox, named after its date section; headers and cards stay outside.
 */
interface ThreadRun {
  key: string;
  label: string;
  size: number;
}

const threadRuns = computed(() => {
  const runs = new Map<string, { run: ThreadRun; position: number }>();
  let label = t('thread.list.ariaLabel');
  let run: ThreadRun | null = null;
  for (const item of props.items) {
    if (item.kind === 'thread') {
      run ??= { key: `run-${item.id}`, label, size: 0 };
      run.size += 1;
      runs.set(item.id, { run, position: run.size });
      continue;
    }
    run = null;
    if (item.kind === 'date-header') label = item.label;
  }
  return runs;
});

type VisibleSegment =
  | { kind: 'threads'; key: string; label: string; items: ThreadEntry[] }
  | { kind: 'entry'; key: string; items: [ThreadListEntry] };

const visibleSegments = computed(() => {
  const segments: VisibleSegment[] = [];
  for (const item of props.items.slice(startIndex.value, endIndex.value)) {
    const run = item.kind === 'thread' ? threadRuns.value.get(item.id)?.run : undefined;
    if (item.kind !== 'thread' || !run) {
      segments.push({ kind: 'entry', key: item.id, items: [item] });
      continue;
    }
    const last = segments[segments.length - 1];
    if (last?.kind === 'threads' && last.key === run.key) last.items.push(item);
    else segments.push({ kind: 'threads', key: run.key, label: run.label, items: [item] });
  }
  return segments;
});

// aria-posinset/setsize describe the whole run, not the rendered window;
// without them a screen reader announces "1 of 12" for a run of 300.
function threadPosition(id: string): number | undefined {
  return threadRuns.value.get(id)?.position;
}
function threadSetSize(id: string): number | undefined {
  return threadRuns.value.get(id)?.run.size;
}
const topSpacerHeight = computed(() => itemOffsets.value[startIndex.value] ?? 0);
/**
 * Scroll past the last row far enough that it clears the floating undo FAB
 * (and the mobile compose FAB zone) instead of hiding behind it. Constant —
 * the FAB appears and disappears, but the list geometry must not.
 */
const LIST_BOTTOM_INSET = 96;
const bottomSpacerHeight = computed(() => {
  const endOffset = itemOffsets.value[endIndex.value] ?? totalHeight.value;
  return Math.max(0, totalHeight.value + LIST_BOTTOM_INSET - endOffset);
});

const dateHeaderPositions = computed(() => {
  const offsets = itemOffsets.value;
  return props.items.flatMap((item, index) => {
    if (item.kind !== 'date-header') return [];
    const offset = offsets[index];
    return offset === undefined ? [] : [{ item, top: offset }];
  });
});

/**
 * Stand-in for `position: sticky`: the virtualiser unmounts the real header
 * row once it leaves the render window, so the current section's header is
 * re-rendered as an overlay pinned to the top of the scroller. `lift` slides
 * it away while the next section's header pushes in.
 */
const stickyHeader = computed(() => {
  let current: (typeof dateHeaderPositions.value)[number] | null = null;
  let nextTop = Infinity;
  for (const header of dateHeaderPositions.value) {
    if (header.top < scrollTop.value) {
      current = header;
    } else {
      nextTop = header.top;
      break;
    }
  }
  if (!current) return null;
  return {
    item: current.item,
    lift: Math.max(0, rows.value.dateHeader - (nextTop - scrollTop.value)),
  };
});

/**
 * The section under the pointer. The bulk button of a date section only shows
 * while the pointer is inside that section, so tracking has to work across
 * plain rows — CSS hover on the header alone cannot express that.
 */
const hoveredSectionId = ref('');
let lastPointerClientY: number | null = null;

function sectionIdAt(contentY: number): string {
  let id = '';
  for (const header of dateHeaderPositions.value) {
    if (header.top <= contentY) {
      id = header.item.id;
    } else {
      break;
    }
  }
  return id;
}

function refreshHoveredSection() {
  const el = scrollerRef.value;
  if (!el || lastPointerClientY === null) return;
  const rect = el.getBoundingClientRect();
  hoveredSectionId.value = sectionIdAt(lastPointerClientY - rect.top + scrollTop.value);
}

function onPointerMove(event: PointerEvent) {
  lastPointerClientY = event.clientY;
  refreshHoveredSection();
}

function onPointerLeave() {
  lastPointerClientY = null;
  hoveredSectionId.value = '';
}

function isActive(thread: Thread): boolean {
  return thread.id === props.activeThreadId;
}

function isSelected(thread: Thread): boolean {
  return props.selectedThreadIds.includes(thread.id);
}

function refreshViewportHeight() {
  viewportHeight.value = scrollerRef.value?.clientHeight ?? 0;
}

// Start fetching roughly two viewports before the end so the next page is
// usually there by the time the user reaches it.
const LOAD_MORE_THRESHOLD_PX = 1200;

function maybeLoadMore() {
  const el = scrollerRef.value;
  if (!el) return;
  const remaining = el.scrollHeight - el.scrollTop - el.clientHeight;
  if (remaining < LOAD_MORE_THRESHOLD_PX) emit('loadMore');
}

let reportedViewportStartId = '';

/** The first rendered row that stands for a thread; headers and cards have none. */
function reportViewportStart() {
  for (let index = startIndex.value; index < props.items.length; index += 1) {
    const item = props.items[index];
    const threadId = item?.kind === 'thread'
      ? item.thread.id
      : item?.kind === 'group' ? item.group.threads[0]?.id : undefined;
    if (!threadId) continue;
    if (threadId !== reportedViewportStartId) {
      reportedViewportStartId = threadId;
      emit('viewportStart', threadId);
    }
    return;
  }
}

function onScroll(event: Event) {
  const el = event.target as HTMLElement;
  pendingScrollTop = el.scrollTop;
  if (scrollFrame !== null) return;

  // Scroll events may outpace paint on a high-refresh display. One reactive
  // update per animation frame keeps virtualization and sticky-header work
  // aligned with what the browser can actually show.
  scrollFrame = window.requestAnimationFrame(() => {
    scrollFrame = null;
    scrollTop.value = pendingScrollTop;
    // Wheel scrolling moves sections beneath a resting pointer.
    refreshHoveredSection();
    reportViewportStart();
    if (el.scrollTop < LOAD_MORE_THRESHOLD_PX) emit('loadPrevious');
    maybeLoadMore();
  });
}

function folderChips(folder: { previewItems: SmartInboxBucketPreviewItem[] }) {
  return folder.previewItems.map((preview) => ({
    id: preview.id,
    unread: preview.unreadCount > 0,
    title: `${preview.senderName}: ${preview.subject}`,
    senderName: preview.senderName,
    senderEmail: preview.senderEmail,
  }));
}

function targetKeysForItem(item: ThreadListEntry): string[] {
  if (item.kind === 'folder-row') {
    return [
      ...(topicsCardVisible.value ? ['topics'] : []),
      ...item.folders.map((folder) => `bucket:${folder.id}`),
    ];
  }
  if (item.kind === 'priority-card') {
    return [
      ...item.threads.map((thread) => `thread:${thread.id}`),
      ...(item.hiddenCount > 0 ? [`bucket:${item.bucket.id}`] : []),
    ];
  }
  if (item.kind === 'bucket') return [`bucket:${item.bucket.id}`];
  if (item.kind === 'group') return [`group:${item.group.id}`];
  if (item.kind === 'new-sender-stack') return [`new-sender:${item.stack.id}`];
  if (item.kind === 'thread') return [`thread:${item.thread.id}`];
  return [];
}

async function revealFocusedTarget() {
  if (!props.keyboardNavigationActive || !props.focusedTargetKey) return;
  const el = scrollerRef.value;
  if (!el) return;

  const itemIndex = props.items.findIndex((item) =>
    targetKeysForItem(item).includes(props.focusedTargetKey!),
  );
  if (itemIndex < 0) return;

  const focusedEntry = props.items[itemIndex];
  if (!focusedEntry) return;
  const itemTop = itemOffsets.value[itemIndex] ?? 0;
  const itemBottom = itemTop + itemHeight(focusedEntry);
  const visibleTop = el.scrollTop;
  const visibleBottom = visibleTop + el.clientHeight;
  let nextTop = visibleTop;
  if (itemTop < visibleTop) nextTop = itemTop;
  else if (itemBottom > visibleBottom) nextTop = itemBottom - el.clientHeight;

  if (nextTop !== visibleTop) {
    el.scrollTop = Math.max(0, nextTop);
    scrollTop.value = el.scrollTop;
  }

  await nextTick();
  const target = Array.from(el.querySelectorAll<HTMLElement>('[data-list-target-key]'))
    .find((node) => node.dataset.listTargetKey === props.focusedTargetKey);
  target?.focus({ preventScroll: true });
}

watch(
  [() => props.focusedTargetKey, () => props.keyboardNavigationActive],
  () => { void revealFocusedTarget(); },
  { flush: 'post' },
);

watch(
  // Reference comparison is enough: the store rebuilds this array on every
  // change. A deep watch here walked thousands of thread objects on each
  // mutation just to re-measure the viewport.
  () => props.items,
  async (items, previous) => {
    const el = scrollerRef.value;
    // Capture the actual visible row rather than the overscan start. Its
    // position survives a prepend, a trim, and inserted date headers.
    let offset = 0;
    let anchor: { id: string; offset: number } | undefined;
    const top = el?.scrollTop ?? 0;
    for (const item of previous) {
      const height = itemHeight(item);
      if (offset + height > top) {
        anchor = { id: item.id, offset: top - offset };
        break;
      }
      offset += height;
    }
    if (el && anchor) {
      let nextOffset = 0;
      for (const item of items) {
        if (item.id === anchor.id) {
          const nextTop = nextOffset + anchor.offset;
          el.scrollTop = nextTop;
          scrollTop.value = nextTop;
          pendingScrollTop = nextTop;
          break;
        }
        nextOffset += itemHeight(item);
      }
    }
    await nextTick();
    refreshViewportHeight();
    reportViewportStart();
    // Fill an initially short viewport. Loaded pages never trigger another
    // request just because trimming left the total height unchanged.
    if (totalHeight.value <= viewportHeight.value) maybeLoadMore();
    await revealFocusedTarget();
  },
);

watch(totalHeight, (next) => {
  const el = scrollerRef.value;
  if (!el) {
    return;
  }

  const maxTop = Math.max(0, next - viewportHeight.value);
  if (el.scrollTop > maxTop) {
    el.scrollTop = maxTop;
    scrollTop.value = maxTop;
  }
});

onMounted(() => {
  if (typeof window.matchMedia === 'function') {
    phoneTypeQuery = window.matchMedia(PHONE_TYPE_QUERY);
    phoneTypeScale.value = phoneTypeQuery.matches;
    phoneTypeQuery.addEventListener('change', onPhoneTypeChange);
  }
  refreshViewportHeight();
  maybeLoadMore();

  if (typeof ResizeObserver !== 'undefined' && scrollerRef.value) {
    resizeObserver = new ResizeObserver(() => {
      refreshViewportHeight();
      maybeLoadMore();
    });
    resizeObserver.observe(scrollerRef.value);
  }
});

onBeforeUnmount(() => {
  if (loadingFeedbackTimer !== null) {
    window.clearTimeout(loadingFeedbackTimer);
    loadingFeedbackTimer = null;
  }
  if (scrollFrame !== null) {
    window.cancelAnimationFrame(scrollFrame);
    scrollFrame = null;
  }
  resizeObserver?.disconnect();
  resizeObserver = null;
  phoneTypeQuery?.removeEventListener('change', onPhoneTypeChange);
  phoneTypeQuery = null;
});
</script>

<template>
  <div
    ref="scrollerRef"
    class="thread-list-scroller h-full min-h-0 overflow-x-hidden overflow-y-auto bg-list"
    :style="rowHeights"
    role="group"
    :aria-label="$t('thread.list.ariaLabel')"
    @scroll="onScroll"
    @pointermove="onPointerMove"
    @pointerleave="onPointerLeave"
  >
    <div v-if="props.loadPreviousFailed" class="sticky top-0 z-10 h-0 overflow-visible">
      <div class="thread-list-more-failed bg-list" role="alert">
        <span>{{ $t('thread.list.loadPreviousFailed') }}</span>
        <AppButton size="sm" variant="secondary" @click="emit('retryLoadMore')">
          {{ $t('common.retry') }}
        </AppButton>
      </div>
    </div>
    <div
      v-else-if="props.hasPreviousThreads && scrollTop < LOAD_MORE_THRESHOLD_PX"
      class="sticky top-0 z-10 h-0 overflow-visible"
    >
      <div class="flex justify-end px-3 pt-2">
        <AppButton
          data-load-previous
          size="sm"
          variant="secondary"
          :disabled="props.paginationBusy"
          @click="emit('loadPrevious')"
        >
          {{ $t('thread.list.loadPrevious') }}
        </AppButton>
      </div>
    </div>
    <div v-if="props.items.length > 0">
      <div v-if="stickyHeader" class="sticky-header-anchor">
        <div
          class="sticky-header flex h-[var(--date-header-h)] items-end bg-list px-5 pb-2 pt-3"
          :style="{ transform: `translateY(-${stickyHeader.lift}px)` }"
        >
          <SectionHeading :label="stickyHeader.item.label" :count="stickyHeader.item.count > 0 ? stickyHeader.item.count : undefined" class="w-full">
            <template #trailing>
              <AppButton
                v-if="stickyHeader.item.threadIds.length > 0"
                tabindex="-1"
                class="section-done-button"
                :class="{ 'section-done-button--visible': hoveredSectionId === stickyHeader.item.id }"
                :title="$t('thread.list.checkAllInSection', { label: stickyHeader.item.label })"
                :aria-label="$t('thread.list.checkAllInSection', { label: stickyHeader.item.label })"
                :disabled="props.bulkBusy"
                @click="emit('dateGroupDone', { groupId: stickyHeader.item.id, threadIds: stickyHeader.item.threadIds })"
                variant="ghost"
                size="sm"
              >
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <path d="M3.5 8.4l2.5 2.5 6.5-6.8" />
                </svg>
                {{ $t('thread.list.checkAll') }}
              </AppButton>
            </template>
          </SectionHeading>
        </div>
      </div>

      <div aria-hidden="true" :style="{ height: `${topSpacerHeight}px` }" />

      <template v-for="segment in visibleSegments" :key="segment.key">
        <div
          v-if="segment.kind === 'threads'"
          role="listbox"
          :aria-label="segment.label"
          aria-multiselectable="true"
        >
          <div
            v-for="item in segment.items"
            :key="item.id"
            class="thread-list-row flex h-[var(--thread-row-h)] items-center px-2"
            role="presentation"
          >
            <ThreadListItem
              class="w-full"
              :thread="item.thread"
              :pos-in-set="threadPosition(item.id)"
              :set-size="threadSetSize(item.id)"
              :active="isActive(item.thread)"
              :selected="isSelected(item.thread)"
              :keyboard-focused="props.keyboardNavigationActive && props.focusedTargetKey === `thread:${item.thread.id}`"
              :selection-mode="props.selectedThreadIds.length > 0"
              :data-list-target-key="`thread:${item.thread.id}`"
              @activate="emit('activate', $event)"
              @check="emit('check', $event)"
            />
          </div>
        </div>

        <template v-else>
          <template v-for="item in segment.items" :key="item.id">
            <section
              v-if="item.kind === 'folder-row'"
              class="border-b border-border/50 bg-list px-5 pt-[var(--folder-section-inset)]"
              :style="{ height: `${itemHeight(item)}px` }"
              :aria-label="$t('thread.list.smartGroups')"
            >
              <div class="-mx-3 grid min-w-0 gap-[var(--folder-card-gap)]">
                <TopicsFolderCard
                  v-if="topicsCardVisible && props.topics"
                  :topics="props.topics"
                  target-key="topics"
                  :keyboard-focused="props.keyboardNavigationActive && props.focusedTargetKey === 'topics'"
                  @open="emit('openTopics')"
                  @create="emit('createTopic')"
                />
                <FolderPreviewCard
                  v-for="folder in item.folders"
                  :key="folder.id"
                  :label="folder.label"
                  :count="folder.count"
                  :unread-count="folder.unreadCount"
                  :chips="folderChips(folder)"
                  :target-key="`bucket:${folder.id}`"
                  :keyboard-focused="props.keyboardNavigationActive && props.focusedTargetKey === `bucket:${folder.id}`"
                  :aria-label="$t('thread.list.folderAria', { label: folder.label, count: folder.count })"
                  @click="emit('openBucket', folder.id)"
                >
                  <template #icon>
                    <SmartBucketIcon
                      :bucket-key="folder.key"
                      :tone="folder.tone"
                      :size="36"
                      :icon-size="18"
                      shape="circle"
                    />
                  </template>
                  <template #chip="{ chip }">
                    <Avatar :name="chip.senderName" :email="chip.senderEmail" :size="18" />
                    <span class="min-w-0 truncate">{{ chip.senderName }}</span>
                  </template>
                </FolderPreviewCard>
              </div>
            </section>

            <section
              v-else-if="item.kind === 'priority-card'"
              class="bg-list px-5 py-3"
              :style="{ height: `${itemHeight(item)}px` }"
              :aria-label="$t('thread.bucket.priority')"
            >
              <article class="priority-card -mx-3 overflow-hidden rounded-2xl pb-2">
                <CardButton
                  class="priority-card__header group/priority relative flex h-[var(--card-header-h)] w-full cursor-pointer items-center gap-[7px] border-0 bg-transparent px-4 text-left"
                  :title="$t('thread.list.showAllPriority')"
                  @click="emit('openBucket', item.bucket.id)"
                >
                  <svg class="priority-card__bolt shrink-0" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M9.2 1.8L4.4 8.4h3.4l-1 5.8 4.8-7H8.2l1-5.4z" />
                  </svg>
                  <h3 class="priority-card__title m-0 text-sm font-strong tracking-[0.01em]">{{ item.bucket.label }}</h3>
                  <span class="pt-px text-2xs font-semibold text-tertiary tabular-nums">{{ item.bucket.count }}</span>
                  <svg class="shrink-0 text-tertiary opacity-0 transition-opacity group-hover/priority:opacity-100" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M6 3.5L10.5 8 6 12.5" />
                  </svg>
                </CardButton>

                <div role="listbox" :aria-label="item.bucket.label" aria-multiselectable="true">
                  <div
                    v-for="(thread, index) in item.threads"
                    :key="thread.id"
                    class="thread-list-row flex h-[var(--thread-row-h)] items-center px-2"
                    role="presentation"
                  >
                    <ThreadListItem
                      class="w-full"
                      :thread="thread"
                      tinted
                      :pos-in-set="index + 1"
                      :set-size="item.threads.length"
                      :active="isActive(thread)"
                      :selected="isSelected(thread)"
                      :keyboard-focused="props.keyboardNavigationActive && props.focusedTargetKey === `thread:${thread.id}`"
                      :selection-mode="props.selectedThreadIds.length > 0"
                      :data-list-target-key="`thread:${thread.id}`"
                      @activate="emit('activate', $event)"
                      @check="emit('check', $event)"
                    />
                  </div>
                </div>

                <CardButton
                  v-if="item.hiddenCount > 0"
                  class="priority-card__more relative mx-2 flex h-[var(--card-header-h)] w-[calc(100%-16px)] cursor-pointer items-center justify-center gap-1.5 rounded-lg border-0 bg-transparent text-xs font-semibold"
                  :class="{ 'list-target--keyboard-focused': props.keyboardNavigationActive && props.focusedTargetKey === `bucket:${item.bucket.id}` }"
                  :data-list-target-key="`bucket:${item.bucket.id}`"
                  :title="$t('thread.list.showAllPriority')"
                  @click="emit('openBucket', item.bucket.id)"
                >
                  {{ $t('thread.list.morePriority', { count: item.hiddenCount }) }}
                </CardButton>
              </article>
            </section>

            <div
              v-else-if="item.kind === 'date-header'"
              class="flex h-[var(--date-header-h)] items-end bg-list px-5 pb-2 pt-3"
            >
              <SectionHeading :label="item.label" :count="item.count > 0 ? item.count : undefined" class="w-full">
                <template #trailing>
                  <AppButton
                    v-if="item.threadIds.length > 0"
                    class="section-done-button"
                    :class="{ 'section-done-button--visible': hoveredSectionId === item.id }"
                    :title="$t('thread.list.checkAllInSection', { label: item.label })"
                    :aria-label="$t('thread.list.checkAllInSection', { label: item.label })"
                    :disabled="props.bulkBusy"
                    @click="emit('dateGroupDone', { groupId: item.id, threadIds: item.threadIds })"
                    variant="ghost"
                    size="sm"
                  >
                    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <path d="M3.5 8.4l2.5 2.5 6.5-6.8" />
                    </svg>
                    {{ $t('thread.list.checkAll') }}
                  </AppButton>
                </template>
              </SectionHeading>
            </div>

            <section
              v-else-if="item.kind === 'new-sender-stack'"
              class="bg-list px-5 py-3"
              :style="{ height: `${itemHeight(item)}px` }"
              :aria-label="$t('thread.newSenders.title')"
            >
              <article class="review-card -mx-3 overflow-hidden rounded-2xl pb-2">
                <div class="review-card__header relative flex h-[var(--card-header-h)] items-center gap-2 px-4">
                  <CardButton
                    class="group/newsender flex min-w-0 cursor-pointer items-center gap-[7px] border-0 bg-transparent p-0 text-left"
                    :title="$t('thread.newSenders.showAll')"
                    @click="emit('openNewSenders')"
                  >
                    <svg class="review-card__glyph shrink-0" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <circle cx="8" cy="5.7" r="2.5" />
                      <path d="M3.8 13a4.2 4.2 0 018.4 0" />
                    </svg>
                    <h3 class="review-card__title m-0 truncate text-sm font-strong tracking-[0.01em]">{{ item.stack.label }}</h3>
                    <span class="pt-px text-2xs font-semibold text-tertiary tabular-nums">{{ item.stack.count }}</span>
                    <svg class="shrink-0 text-tertiary opacity-0 transition-opacity group-hover/newsender:opacity-100" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <path d="M6 3.5L10.5 8 6 12.5" />
                    </svg>
                  </CardButton>
                  <AppButton
                    v-if="!props.senderReviewSetupComplete"
                    class="review-card__setup ml-auto"
                    :title="$t('thread.newSenders.acceptExistingTitle')"
                    @click="emit('acceptExistingSenders')"
                    variant="ghost"
                    size="sm"
                  >
                    {{ $t('thread.newSenders.acceptExisting') }}
                  </AppButton>
                  <CountBadge v-else-if="item.stack.unreadCount > 0" class="ml-auto" :value="item.stack.unreadCount" tone="accent" />
                </div>

                <div class="flex h-[var(--thread-row-h)] items-center px-2">
                  <article
                    class="review-card__row grid h-[var(--review-row-h)] w-full cursor-pointer grid-cols-[40px_minmax(0,1fr)_auto] items-center gap-3 rounded-xl px-3 transition-colors"
                    :class="{ 'list-target--keyboard-focused': props.keyboardNavigationActive && props.focusedTargetKey === `new-sender:${item.stack.id}` }"
                    :data-list-target-key="`new-sender:${item.stack.id}`"
                    role="button"
                    tabindex="0"
                    @click="emit('activate', { threadId: item.stack.senders[0]!.topThreadId, shiftKey: false, metaKey: false, ctrlKey: false })"
                    @keydown.enter.prevent="emit('activate', { threadId: item.stack.senders[0]!.topThreadId, shiftKey: false, metaKey: false, ctrlKey: false })"
                    @keydown.space.prevent="emit('activate', { threadId: item.stack.senders[0]!.topThreadId, shiftKey: false, metaKey: false, ctrlKey: false })"
                  >
                    <Avatar
                      :name="item.stack.senders[0]!.senderName"
                      :email="item.stack.senders[0]!.senderEmail"
                      :size="38"
                    />
                    <div class="min-w-0 text-left">
                      <div class="flex min-w-0 items-center gap-1.5">
                        <span class="min-w-0 truncate text-md font-strong text-primary">{{ item.stack.senders[0]!.senderName }}</span>
                        <CountBadge v-if="item.stack.senders[0]!.count > 1" :value="item.stack.senders[0]!.count" />
                      </div>
                      <div class="mt-0.5 truncate text-xs text-secondary">{{ item.stack.senders[0]!.senderEmail }}</div>
                      <div
                        class="mt-1 truncate text-xs"
                        :class="item.stack.senders[0]!.spoofingText ? 'font-medium text-accent-red' : 'text-tertiary'"
                      >
                        {{ item.stack.senders[0]!.spoofingText || item.stack.senders[0]!.subject || item.stack.senders[0]!.previewText }}
                      </div>
                    </div>

                    <div class="new-sender-actions flex shrink-0 items-center gap-1.5">
                      <AppButton

                        :title="$t('thread.newSenders.allowTitle')"
                        @click.stop="emit('newSenderAction', { sender: item.stack.senders[0]!, action: 'accept' })"
                        variant="primary"
                        size="sm"
                      >
                        {{ $t('thread.newSenders.allow') }}
                      </AppButton>
                      <AppButton

                        :title="$t('thread.actions.blockSender')"
                        @click.stop="emit('newSenderAction', { sender: item.stack.senders[0]!, action: 'block' })"
                        variant="danger"
                        size="sm"
                      >
                        {{ $t('common.block') }}
                      </AppButton>
                    </div>
                  </article>
                </div>

                <CardButton
                  v-if="item.stack.domainSuggestion"
                  class="review-card__more relative mx-2 flex h-[var(--card-header-h)] w-[calc(100%-16px)] min-w-0 cursor-pointer items-center justify-center gap-1.5 rounded-lg border-0 bg-transparent px-3 text-xs font-semibold"
                  :title="$t('thread.newSenders.allowDomainTitle', { domain: item.stack.domainSuggestion.domain })"
                  @click="emit('acceptDomain', item.stack.domainSuggestion)"
                >
                  <svg class="shrink-0" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <circle cx="8" cy="8" r="3.1" />
                    <path d="M11.1 8v1.2a1.7 1.7 0 003.4 0V8a6.5 6.5 0 10-2.6 5.2" />
                  </svg>
                  <span class="min-w-0 truncate">{{ $t('thread.newSenders.allowDomain', { domain: item.stack.domainSuggestion.domain }) }}</span>
                  <span class="shrink-0 font-medium tabular-nums opacity-70">{{ $t('thread.newSenders.senderCount', { count: item.stack.domainSuggestion.senderCount }) }}</span>
                </CardButton>
                <CardButton
                  v-else-if="item.stack.count > 1"
                  class="review-card__more relative mx-2 flex h-[var(--card-header-h)] w-[calc(100%-16px)] cursor-pointer items-center justify-center gap-1.5 rounded-lg border-0 bg-transparent text-xs font-semibold"
                  @click="emit('openNewSenders')"
                >
                  {{ $t('thread.newSenders.moreSenders', { count: item.stack.count - 1 }) }}
                </CardButton>
              </article>
            </section>

            <section
              v-else-if="item.kind === 'bucket'"
              class="group/bucket mx-2 flex h-[var(--bucket-h)] cursor-pointer items-center gap-3 rounded-xl bg-stack px-3 transition-colors hover:bg-stack-hover"
              :class="{ 'list-target--keyboard-focused': props.keyboardNavigationActive && props.focusedTargetKey === `bucket:${item.bucket.id}` }"
              :data-list-target-key="`bucket:${item.bucket.id}`"
              role="button"
              tabindex="0"
              @click="emit('openBucket', item.bucket.id)"
              @keydown.enter.prevent="emit('openBucket', item.bucket.id)"
              @keydown.space.prevent="emit('openBucket', item.bucket.id)"
            >
              <SmartBucketIcon :bucket-key="item.bucket.key" :tone="item.bucket.tone" />

              <div class="min-w-0 flex-1">
                <div class="flex min-w-0 items-baseline gap-2">
                  <h4 class="m-0 truncate text-md font-semibold text-primary">{{ item.bucket.label }}</h4>
                  <span class="shrink-0 text-xs font-semibold text-tertiary tabular-nums">{{ item.bucket.count }}</span>
                  <span
                    v-if="item.bucket.unreadCount > 0"
                    class="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-accent-primary px-[6px] text-3xs font-bold text-on-accent tabular-nums"
                  >
                    {{ item.bucket.unreadCount }}
                  </span>
                </div>
                <p class="mt-0.5 min-w-0 truncate text-xs text-tertiary">{{ item.bucket.previewText }}</p>
              </div>

              <div class="flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover/bucket:opacity-100 group-focus-within/bucket:opacity-100">
                <IconButton

                  :title="$t('thread.list.markAllRead')"
                  :aria-label="$t('thread.list.markAllRead')"
                  @click.stop="emit('bucketAction', { threadIds: item.bucket.threadIds, action: 'read' })"
                >
                  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M2.8 5.2L8 8.7l5.2-3.5" />
                    <rect x="2.5" y="4" width="11" height="8.5" rx="1.7" />
                  </svg>
                </IconButton>
                <IconButton

                  :title="$t('thread.list.checkAll')"
                  :aria-label="$t('thread.list.checkAll')"
                  @click.stop="emit('bucketAction', { threadIds: item.bucket.threadIds, action: 'done' })"
                >
                  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M3.5 8.4l2.5 2.5 6.5-6.8" />
                  </svg>
                </IconButton>
              </div>

              <!-- The row opens a page of its own, and says so even at rest. -->
              <svg
                class="shrink-0 text-tertiary"
                width="16"
                height="16"
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M6 3.5L10.5 8 6 12.5" />
              </svg>
            </section>

            <ThreadGroupCard
              v-else-if="item.kind === 'group'"
              :group="item.group"
              :busy="props.bulkBusy"
              :target-key="`group:${item.group.id}`"
              :keyboard-focused="props.keyboardNavigationActive && props.focusedTargetKey === `group:${item.group.id}`"
              @open="emit('openGroup', $event)"
              @done="emit('groupDone', $event)"
            />
          </template>
        </template>
      </template>

      <div aria-hidden="true" :style="{ height: `${bottomSpacerHeight}px` }" />
      <div v-if="props.loadMoreFailed && !props.loadPreviousFailed" class="thread-list-more-failed" role="alert">
        <span>{{ $t('thread.list.loadMoreFailed') }}</span>
        <AppButton size="sm" variant="secondary" @click="emit('retryLoadMore')">
          {{ $t('common.retry') }}
        </AppButton>
      </div>
    </div>

    <!-- A failed load leaves the array empty. Without this branch the empty
         state would claim the mailbox is empty, which is a different sentence. -->
    <ErrorState
      v-else-if="props.error"
      :title="$t('thread.list.loadFailed')"
      :detail="backendErrorLabel(props.error)"
      :busy="props.loading"
      @retry="emit('retry')"
    />

    <div
      v-else-if="props.loading && !showLoadingFeedback"
      class="mail-list-loading-reserve"
      aria-busy="true"
      :aria-label="$t('thread.list.loading')"
    />

    <div v-else-if="props.loading" class="mail-list-skeleton" role="status" :aria-label="$t('thread.list.loading')">
      <SkeletonRow :count="6" height="84px" />
    </div>

    <!-- Ordered after the failure and before the empty state on purpose: mail
         that is still on its way is neither a fault nor an empty mailbox. -->
    <div
      v-else-if="backfillRunning"
      class="mail-list-backfill"
      role="status"
      aria-busy="true"
    >
      <p class="mail-list-backfill__label">{{ backfillLabel }}</p>
      <SkeletonRow :count="4" height="84px" />
    </div>

    <div v-else class="flex h-full min-h-[200px] flex-col items-center justify-center gap-3 text-tertiary">
      <svg width="32" height="32" viewBox="0 0 32 32" fill="none" stroke="currentColor" stroke-width="1.2" opacity="0.28">
        <rect x="4" y="8" width="24" height="18" rx="3" />
        <path d="M4 12L16 18L28 12" />
      </svg>
      <p class="m-0 text-sm">{{ emptyLabel }}</p>
    </div>
  </div>
</template>

<style scoped>
.mail-list-backfill {
  display: flex;
  flex-direction: column;
  min-height: 200px;
}

.mail-list-backfill__label {
  margin: 0;
  padding: var(--space-lg) var(--space-lg) 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.thread-list-scroller {
  container-type: inline-size;
  scrollbar-gutter: stable;
}

.list-target--keyboard-focused {
  position: relative;
  z-index: 2;
  background: color-mix(in srgb, var(--accent-primary) 14%, var(--bg-list));
  box-shadow:
    inset 0 0 0 1px color-mix(in srgb, var(--accent-primary) 42%, transparent),
    var(--shadow-glow);
}

/* One warm, borderless surface — the rows render transparent on it (see the
   `tinted` prop on ThreadListItem), so the wash reads as a single card the way
   Spark draws it, not as pale rows floating on a tinted tray. Tokens only:
   a hex here would not survive the light theme. */
.priority-card {
  background: color-mix(in srgb, var(--accent-amber) 9%, var(--bg-list));
}

.priority-card__header::after {
  content: '';
  position: absolute;
  left: 16px;
  right: 16px;
  bottom: 0;
  height: 1px;
  background: color-mix(in srgb, var(--accent-amber) 22%, transparent);
}

.priority-card__bolt,
.priority-card__title {
  color: var(--accent-amber);
}

.priority-card__more {
  color: var(--text-tertiary);
  transition: background-color var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
}

.priority-card__more:hover {
  background: color-mix(in srgb, var(--accent-amber) 9%, transparent);
  color: var(--accent-amber);
}

/* The sender-review card is the priority card's blue sibling: same borderless
   wash, hairline under the header, quiet footer — one grammar, two tones. */
.review-card {
  background: color-mix(in srgb, var(--accent-primary) 7%, var(--bg-list));
}

.review-card__header::after {
  content: '';
  position: absolute;
  left: 16px;
  right: 16px;
  bottom: 0;
  height: 1px;
  background: color-mix(in srgb, var(--accent-primary) 20%, transparent);
}

.review-card__glyph,
.review-card__title {
  color: var(--accent-primary);
}

.review-card__setup {
  color: var(--accent-primary);
}

.review-card__setup:hover {
  background: color-mix(in srgb, var(--accent-primary) 12%, transparent);
}

.review-card__row:hover {
  background: color-mix(in srgb, var(--accent-primary) 8%, transparent);
}

.review-card__more {
  color: var(--text-tertiary);
  transition: background-color var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
}

.review-card__more:hover {
  background: color-mix(in srgb, var(--accent-primary) 9%, transparent);
  color: var(--accent-primary);
}

.mail-list-skeleton {
  min-height: 100%;
  overflow: hidden;
}

.mail-list-loading-reserve {
  min-height: 100%;
}

/* The row content exits first; then its reserved virtual-list slot closes.
   Keeping both phases in one timeline avoids the abrupt 92px jump that used
   to make checking off a mail look like a rendering failure. */
.thread-list-row {
  overflow: hidden;
}

.thread-list-row:has(.thread-row--leaving) {
  pointer-events: none;
  animation: thread-list-row-collapse var(--duration-slow) cubic-bezier(0.32, 0, 0.15, 1) forwards;
}

@keyframes thread-list-row-collapse {
  0%, 42% {
    height: var(--thread-row-h);
    padding-block: 0;
  }
  100% {
    height: 0;
    padding-block: 0;
  }
}

.sticky-header-anchor {
  position: sticky;
  top: 0;
  z-index: 30;
  height: 0;
}

.sticky-header {
  box-shadow: 0 10px 18px -14px rgba(0, 0, 0, 0.55);
}

.section-done-button {
  opacity: 0;
  pointer-events: none;
  transition: opacity var(--duration-fast) var(--ease-out);
}

.section-done-button--visible,
.section-done-button:focus-visible {
  opacity: 1;
  pointer-events: auto;
}

@container (max-width: 360px) {
  .review-card__row { gap: 8px; }
  .new-sender-actions { flex-direction: column; align-items: stretch; gap: 4px; }
  .new-sender-actions button { height: 26px; justify-content: center; padding-inline: 9px; }
}

@media (prefers-reduced-motion: reduce) {
  .thread-list-row:has(.thread-row--leaving) {
    animation: none;
  }
}

.thread-list-more-failed {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-md);
  padding: var(--space-lg) var(--space-xl);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}
</style>
