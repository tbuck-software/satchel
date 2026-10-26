<script setup lang="ts" generic="T extends FolderPreviewChip">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import ListRowButton from '@/components/ui/ListRowButton.vue';

export interface FolderPreviewChip {
  id: string;
  unread: boolean;
  title?: string;
}

const props = withDefaults(
  defineProps<{
    label: string;
    count?: number;
    /** Unread total for the whole folder, announced with its total count. */
    unreadCount?: number;
    chips: T[];
    /**
     * How many names the one preview line carries: two on the desktop, one
     * on a phone. The rest is a count in its own cell, so it is never clipped.
     */
    previewLimit?: number;
    targetKey?: string;
    keyboardFocused?: boolean;
  }>(),
  { previewLimit: 2 },
);

const { t } = useI18n();

// Unread previews lead, but both read and unread items count toward the total.
const sortedChips = computed(() =>
  [...props.chips].sort((left, right) => Number(right.unread) - Number(left.unread)),
);

const visibleChips = computed(() => sortedChips.value.slice(0, Math.max(1, props.previewLimit)));

const hiddenCount = computed(() => {
  const total = Math.max(props.count ?? props.chips.length, props.chips.length);
  return Math.max(0, total - visibleChips.value.length);
});
</script>

<template>
  <ListRowButton
    class="folder-preview-card group/folder relative grid h-[var(--folder-card-h,auto)] w-full cursor-pointer grid-cols-[36px_minmax(0,1fr)] items-center gap-3 rounded-xl border-0 bg-transparent px-3 py-2 text-left text-secondary transition-colors hover:bg-hover hover:text-primary"
    :class="{ 'folder-preview-card--keyboard-focused': props.keyboardFocused }"
    :data-list-target-key="props.targetKey"
  >
    <span class="folder-preview-card__icon relative grid h-9 w-9 place-items-center">
      <slot name="icon" />
    </span>

    <span class="block min-w-0">
      <span class="flex min-h-6 min-w-0 items-center gap-1.5">
        <span class="folder-preview-card__label min-w-0 truncate text-sm font-semibold text-primary">{{ props.label }}</span>
        <span
          v-if="props.count !== undefined"
          class="shrink-0 text-2xs font-semibold text-tertiary tabular-nums"
          :title="props.unreadCount ? t('thread.list.unreadCount', { count: props.unreadCount }) : undefined"
        >
          {{ props.count }}
        </span>
        <!-- As tall as the title line, so a 26px button leaves the row the
             same height as the bundles without one; the button's own inset
             is pulled out so its glyph ends where the preview counts end. -->
        <span v-if="$slots.actions" class="-mr-1.5 ml-auto flex h-6 shrink-0 items-center gap-1">
          <slot name="actions" />
        </span>
      </span>

      <!-- One line: names first, the remainder in a cell of its own. -->
      <span class="folder-preview-card__previews mt-0.5 flex min-w-0 items-center gap-3">
        <span
          v-for="chip in visibleChips"
          :key="chip.id"
          class="folder-chip flex min-w-0 items-center gap-1.5 text-xs"
          :class="chip.unread ? 'font-medium text-primary' : 'text-secondary'"
          :title="chip.title"
        >
          <slot name="chip" :chip="chip" />
        </span>
        <span v-if="hiddenCount > 0" class="folder-chip-counter">
          {{ t('thread.list.folderMore', { count: hiddenCount }) }}
        </span>
        <span
          v-if="sortedChips.length === 0 && $slots.empty"
          class="folder-preview-card__empty min-w-0 truncate text-xs text-tertiary"
        >
          <slot name="empty" />
        </span>
      </span>
    </span>
  </ListRowButton>
</template>

<style scoped>
/* It sits flat on the pane like a mail row: a bundle is a row that opens a
   page, and a bordered card per bundle stacked four boxes above the mail. */
.folder-preview-card__icon {
  --avatar-dot-surface: var(--bg-list);
}

.folder-preview-card:hover .folder-preview-card__icon {
  --avatar-dot-surface: var(--bg-hover);
}

/* Names shrink by their length, the count never does. The line is one text
   line tall, so a long name truncates instead of wrapping into the next row. */
.folder-preview-card__previews {
  height: calc(var(--font-size-xs) * 1.5);
}

.folder-chip {
  flex: 0 1 auto;
  line-height: 1.5;
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
  flex-shrink: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-semibold);
  font-variant-numeric: tabular-nums;
}
</style>
