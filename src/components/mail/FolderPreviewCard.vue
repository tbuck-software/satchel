<script setup lang="ts" generic="T extends FolderPreviewChip">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import ListRowButton from '@/components/ui/ListRowButton.vue';

export interface FolderPreviewChip {
  id: string;
  unread: boolean;
  title?: string;
}

const props = defineProps<{
  label: string;
  count?: number;
  /** Unread total for the whole folder, announced with its total count. */
  unreadCount?: number;
  chips: T[];
  targetKey?: string;
  keyboardFocused?: boolean;
}>();

const { t } = useI18n();

// Unread previews lead, but both read and unread items count toward the total.
const sortedChips = computed(() =>
  [...props.chips].sort((left, right) => Number(right.unread) - Number(left.unread)),
);

function hiddenCount(visibleLimit: number): number {
  const total = Math.max(props.count ?? props.chips.length, props.chips.length);
  return Math.max(0, total - Math.min(visibleLimit, props.chips.length));
}

const hiddenAfterFour = computed(() => hiddenCount(4));
const hiddenAfterTwo = computed(() => hiddenCount(2));

const visibleChips = computed(() => sortedChips.value.slice(0, 4));
</script>

<template>
  <ListRowButton
    class="folder-preview-card group/folder relative grid h-[var(--folder-card-h,auto)] w-full cursor-pointer grid-cols-[36px_minmax(0,1fr)] items-center gap-3 rounded-xl border-0 bg-transparent px-3 py-2 text-left text-secondary transition-colors hover:bg-hover hover:text-primary"
    :class="{ 'folder-preview-card--keyboard-focused': props.keyboardFocused }"
    :data-list-target-key="props.targetKey"
  >
    <span class="folder-preview-card__icon relative block h-9 w-9">
      <slot name="icon" />
    </span>

    <span class="block min-w-0">
      <span class="flex min-h-6 min-w-0 items-center gap-1.5">
        <span class="min-w-0 truncate text-md font-bold text-primary">{{ props.label }}</span>
        <span
          v-if="props.count !== undefined"
          class="shrink-0 text-2xs font-semibold text-tertiary tabular-nums"
          :title="props.unreadCount ? t('thread.list.unreadCount', { count: props.unreadCount }) : undefined"
        >
          {{ props.count }}
        </span>
        <span v-if="$slots.actions" class="ml-auto flex shrink-0 items-center gap-1">
          <slot name="actions" />
        </span>
      </span>

      <span
        class="folder-preview-card__previews mt-1 min-w-0 gap-x-3 gap-y-0.5"
        :class="{ 'folder-preview-card__previews--stacked': visibleChips.length <= 2 }"
      >
        <span
          v-for="chip in visibleChips"
          :key="chip.id"
          class="folder-chip flex min-w-0 items-center gap-1.5 text-xs leading-[18px]"
          :class="chip.unread ? 'font-strong text-primary' : 'text-secondary'"
          :title="chip.title"
        >
          <slot name="chip" :chip="chip" />
        </span>
        <span v-if="hiddenAfterFour > 0" class="folder-chip-counter folder-chip-counter--regular">
          {{ t('thread.list.folderMore', { count: hiddenAfterFour }) }}
        </span>
        <span v-if="hiddenAfterTwo > 0" class="folder-chip-counter folder-chip-counter--compact">
          {{ t('thread.list.folderMore', { count: hiddenAfterTwo }) }}
        </span>
        <span
          v-if="sortedChips.length === 0 && $slots.empty"
          class="folder-preview-card__empty text-xs leading-[18px] text-tertiary"
        >
          <slot name="empty" />
        </span>
      </span>
    </span>
  </ListRowButton>
</template>

<style scoped>
/* The card is its own container: the chip clipping must follow the card's
   width in every host (virtualised list, topics folder), not the scroller's.
   It sits flat on the pane like a mail row: a bundle is a row that opens a
   page, and a bordered card per bundle stacked four boxes above the mail. */
.folder-preview-card {
  container-type: inline-size;
}

.folder-preview-card__icon {
  --avatar-dot-surface: var(--bg-list);
}

.folder-preview-card:hover .folder-preview-card__icon {
  --avatar-dot-surface: var(--bg-hover);
}

/* Explicit cells keep long names from consuming the second preview line.
   The rest count has its own cell, so it cannot be clipped below the card. */
.folder-preview-card__previews {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr)) auto;
  grid-auto-rows: 18px;
}

.folder-chip:nth-child(1) { grid-area: 1 / 1; }
.folder-chip:nth-child(2) { grid-area: 1 / 2; }
.folder-chip:nth-child(3) { grid-area: 2 / 1; }
.folder-chip:nth-child(4) { grid-area: 2 / 2; }

/* Two previews take a line each at full width instead of halving the names. */
.folder-preview-card__previews--stacked {
  grid-template-columns: minmax(0, 1fr) auto;
}

.folder-preview-card__previews--stacked .folder-chip:nth-child(2) { grid-area: 2 / 1; }
.folder-preview-card__previews--stacked .folder-chip-counter--regular { grid-area: 2 / 2; }

.folder-preview-card__empty {
  grid-column: 1 / -1;
}

.folder-preview-card--keyboard-focused {
  z-index: 2;
  background: color-mix(in srgb, var(--accent-primary) 14%, var(--bg-base));
  color: var(--text-primary);
  box-shadow:
    inset 0 0 0 1px color-mix(in srgb, var(--accent-primary) 42%, transparent),
    var(--shadow-glow);
}

.folder-chip-counter {
  grid-area: 2 / 3;
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-strong);
  line-height: 18px;
  font-variant-numeric: tabular-nums;
}

.folder-chip-counter--compact {
  display: none;
}

/* 344px card ≈ the previous 360px scroller breakpoint minus the horizontal
   padding between scroller and card. */
@container (max-width: 344px) {
  .folder-preview-card__previews {
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .folder-chip:nth-child(2) { grid-area: 2 / 1; }

  .folder-chip:nth-of-type(n + 3) {
    display: none;
  }

  .folder-chip-counter--regular {
    display: none;
  }

  .folder-chip-counter--compact {
    margin-inline-end: calc(2 * var(--space-2xl));
    grid-area: 2 / 2;
    display: inline-flex;
  }
}
</style>
