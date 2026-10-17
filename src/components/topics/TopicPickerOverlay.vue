<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useUiStore } from '@/stores/ui';
import { useTopicsStore } from '@/stores/topics';
import { useThreadActions } from '@/composables/useThreadActions';
import { useFocusTrap } from '@/composables/useFocusTrap';
import { useToast } from '@/composables/useToast';

const topics = useTopicsStore();
const threadActions = useThreadActions();
const toast = useToast();
const { t } = useI18n();

const query = ref('');
const activeIndex = ref(0);
const inputRef = ref<HTMLInputElement | null>(null);
const panelRef = ref<HTMLElement | null>(null);
const pendingCreateNames = new Set<string>();

const open = computed(() => topics.pickerTarget !== null);

// The search field takes the focus itself; the trap keeps Tab off the list
// behind the picker and hands the focus back to the row that opened it.
const trap = useFocusTrap(panelRef, {
  isOpen: () => open.value,
  moveFocusIn: () => false,
});

type PickerRow =
  | { kind: 'topic'; id: string; name: string; color: string | null; icon: string | null; checked: boolean; partial: boolean }
  | { kind: 'create'; id: 'create'; name: string };

const rows = computed<PickerRow[]>(() => {
  const target = topics.pickerTarget;
  if (!target) return [];
  const term = query.value.trim().toLowerCase();
  const list: PickerRow[] = topics.activeTopics
    .filter((topic) => !term || topic.name.toLowerCase().includes(term))
    .map((topic) => {
      const memberCount = target.itemIds.filter((itemId) =>
        topics.topicIdsFor(target.itemType, itemId).includes(topic.id),
      ).length;
      return {
        kind: 'topic' as const,
        id: topic.id,
        name: topic.name,
        color: topic.color,
        icon: topic.icon,
        checked: memberCount === target.itemIds.length,
        partial: memberCount > 0 && memberCount < target.itemIds.length,
      };
    });
  const name = query.value.trim();
  const exactMatch = topics.activeTopics.some(
    (topic) => topic.name.toLowerCase() === name.toLowerCase(),
  );
  if (name && !exactMatch) {
    list.push({ kind: 'create', id: 'create', name });
  }
  return list;
});

const activeId = computed(() =>
  rows.value.length > 0 ? `topic-picker-item-${activeIndex.value}` : undefined,
);

watch(open, async (value) => {
  if (!value) return;
  query.value = '';
  activeIndex.value = 0;
  await nextTick();
  inputRef.value?.focus();
});

watch(rows, (value) => {
  if (activeIndex.value >= value.length) activeIndex.value = Math.max(0, value.length - 1);
});

function move(delta: number) {
  const count = rows.value.length;
  if (count === 0) return;
  activeIndex.value = (activeIndex.value + delta + count) % count;
  document
    .getElementById(`topic-picker-item-${activeIndex.value}`)
    ?.scrollIntoView({ block: 'nearest' });
}

function close() {
  topics.closePicker();
}

// The back gesture should close the picker before it closes anything beneath.
const ui = useUiStore();
onBeforeUnmount(ui.registerOverlay({ isOpen: () => open.value, dismiss: close }));

function activate(row: PickerRow) {
  const target = topics.pickerTarget;
  if (!target) return;
  if (row.kind === 'create') {
    const key = row.name.trim().toLowerCase();
    if (pendingCreateNames.has(key)) return;
    pendingCreateNames.add(key);
    const creation = topics.create({ name: row.name });
    query.value = '';
    void creation.persisted
      .then(() => assign(creation.topic.id, false))
      .catch(() => {
        if (topics.pickerTarget && !query.value) query.value = row.name;
        toast.error(t('topics.picker.createFailed'));
      })
      .finally(() => pendingCreateNames.delete(key));
    return;
  }
  void assign(row.id, row.checked);
}

/** Toggle membership for all target items; the picker stays open for multi-assign. */
async function assign(topicId: string, currentlyChecked: boolean) {
  const target = topics.pickerTarget;
  if (!target) return;
  if (target.itemType === 'thread') {
    if (currentlyChecked) await threadActions.removeFromTopic(target.itemIds, topicId);
    else await threadActions.addToTopic(target.itemIds, topicId);
    return;
  }
  const topicName = topics.byId(topicId)?.name ?? '';
  try {
    if (currentlyChecked) {
      await Promise.all(
        target.itemIds.map((itemId) => topics.removeItemFromTopic(topicId, 'message', itemId)),
      );
      toast.success(t('topics.picker.messageRemoved', { name: topicName }));
    } else {
      await topics.addItemsToTopic(topicId, 'message', target.itemIds);
      toast.success(t('topics.picker.messageAdded', { name: topicName }));
    }
  } catch {
    toast.error(t('topics.picker.assignFailed'));
  }
}

function onEnter() {
  const row = rows.value[activeIndex.value];
  if (row) void activate(row);
}
</script>

<template>
  <Teleport to="body">
    <Transition name="fade">
      <div v-if="open" class="backdrop" @click.self="close">
        <section
          ref="panelRef"
          class="panel"
          role="dialog"
          aria-modal="true"
          :aria-label="topics.pickerTarget?.itemType === 'message' ? $t('topics.picker.titleMessage') : $t('topics.picker.title')"
          @keydown.esc.stop.prevent="close"
          @keydown.tab="trap.onTab"
        >
          <div class="search-row">
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" opacity="0.4" aria-hidden="true">
              <circle cx="7" cy="7" r="5" />
              <path d="M11 11L14 14" />
            </svg>
            <input
              ref="inputRef"
              v-model="query"
              type="text"
              :placeholder="$t('topics.picker.searchPlaceholder')"
              :aria-label="$t('topics.picker.searchLabel')"
              role="combobox"
              aria-expanded="true"
              aria-controls="topic-picker-list"
              :aria-activedescendant="activeId"
              @keydown.down.prevent="move(1)"
              @keydown.up.prevent="move(-1)"
              @keydown.enter.prevent="onEnter"
            />
          </div>

          <div v-if="rows.length > 0" id="topic-picker-list" class="list" role="listbox" :aria-label="$t('topics.title')">
            <button
              v-for="(row, index) in rows"
              :id="`topic-picker-item-${index}`"
              :key="row.id"
              type="button"
              role="option"
              :aria-selected="index === activeIndex"
              class="row"
              :class="{ active: index === activeIndex }"
              @click="activate(row)"
              @mouseenter="activeIndex = index"
            >
              <template v-if="row.kind === 'topic'">
                <span class="row-check" :class="{ 'row-check--on': row.checked, 'row-check--partial': row.partial }" aria-hidden="true">
                  <svg v-if="row.checked" width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M2.5 8.5L6.5 12.5L13.5 4" />
                  </svg>
                  <span v-else-if="row.partial" class="row-check__dash" />
                </span>
                <span v-if="row.icon" class="row-icon">{{ row.icon }}</span>
                <span v-else class="row-orb" :style="{ background: row.color || 'var(--accent-primary)' }" aria-hidden="true" />
                <span class="row-title">{{ row.name }}</span>
              </template>
              <template v-else>
                <span class="row-check" aria-hidden="true">
                  <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
                    <path d="M8 3V13M3 8H13" />
                  </svg>
                </span>
                <span class="row-title">{{ $t('topics.picker.createRow', { name: row.name }) }}</span>
              </template>
            </button>
          </div>
          <p v-else class="empty-hint">{{ $t('topics.picker.empty') }}</p>

          <footer class="hint-row" aria-hidden="true">
            <span>{{ $t('topics.picker.hintSelect') }}</span>
            <span>{{ $t('topics.picker.hintAssign') }}</span>
            <span>{{ $t('topics.picker.hintClose') }}</span>
          </footer>
        </section>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  background: var(--bg-backdrop);
  backdrop-filter: blur(4px);
  display: grid;
  place-items: start center;
  padding:
    calc(16vh + var(--satchel-safe-area-top))
    var(--satchel-safe-area-right)
    var(--satchel-safe-area-bottom)
    var(--satchel-safe-area-left);
  z-index: 100;
}

.panel {
  width: min(440px, 92vw);
  background: var(--bg-surface);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
  display: grid;
}

.search-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 11px 14px;
  border-bottom: 1px solid var(--border-color-muted);
}

.search-row input {
  flex: 1;
  border: 0;
  background: transparent;
  padding: 0;
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-md);
  outline: none;
  min-width: 0;
}

.search-row input::placeholder {
  color: var(--text-tertiary);
}

.list {
  padding: 6px;
  max-height: 40vh;
  overflow-y: auto;
  display: grid;
  gap: 2px;
}

.row {
  width: 100%;
  text-align: left;
  border: 0;
  background: transparent;
  border-radius: var(--radius-sm);
  padding: 7px 9px;
  display: flex;
  align-items: center;
  gap: 9px;
  font: inherit;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.row.active {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.row-check {
  display: grid;
  place-items: center;
  width: 16px;
  height: 16px;
  border: 1.5px solid var(--border-color);
  border-radius: var(--radius-xs);
  color: var(--text-on-accent);
  flex-shrink: 0;
}

.row-check--on {
  background: var(--accent-primary);
  border-color: var(--accent-primary);
}

.row-check--partial {
  border-color: var(--accent-primary);
}

.row-check__dash {
  width: 8px;
  height: 2px;
  border-radius: 1px;
  background: var(--accent-primary);
}

.row-icon {
  font-size: var(--font-size-md);
  line-height: 1;
  flex-shrink: 0;
}

.row-orb {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  flex-shrink: 0;
}

.row-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.empty-hint {
  margin: 0;
  padding: 18px 14px;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
  text-align: center;
}

.hint-row {
  display: flex;
  gap: 14px;
  justify-content: center;
  padding: 7px 12px;
  border-top: 1px solid var(--border-color-muted);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity var(--duration-fast) var(--ease-out);
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
