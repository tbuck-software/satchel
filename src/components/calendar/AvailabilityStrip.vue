<script setup lang="ts">
/**
 * Who is busy on the proposed day: a row for the account owner and one per
 * attendee, the proposed slot drawn across them. Day steps and "next time
 * everyone is free" move the slot; so does a click on a free spot. Wide it is
 * a timeline, on a phone a short list that says the same in words.
 *
 * People nobody would answer for (no calendar access, an outside address, a
 * failed request) get an "unknown" row, not an error.
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useAvailability, type BackendBusyBlock, type BackendFreeBusyReport } from '@/composables/useAvailability';
import { useCalendarUiStore } from '@/stores/calendarUi';
import { useUiStore } from '@/stores/ui';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { addDays, formatTime, startOfDay } from '@/utils/calendarGrid';

/** "Next time everyone is free" looks this many working days ahead. */
const SEARCH_WORKING_DAYS = 10;
const SNAP_MINUTES = 15;

const props = defineProps<{
  accountId: string | null;
  attendees: string[];
  start: Date;
  end: Date;
  disabled?: boolean;
}>();

const emit = defineEmits<{ move: [start: Date, end: Date] }>();

const { t, locale } = useI18n();
const ui = useUiStore();
const calendarUi = useCalendarUiStore();
const availability = useAvailability();

const report = ref<BackendFreeBusyReport | null>(null);
const loading = ref(false);
const failed = ref(false);
const searching = ref(false);
const searchMessage = ref('');
const notCounted = ref<string[]>([]);

const day = computed(() => startOfDay(props.start));
const durationMs = computed(() => Math.max(props.end.getTime() - props.start.getTime(), SNAP_MINUTES * 60_000));
const durationMinutes = computed(() => Math.round(durationMs.value / 60_000));

let generation = 0;
let timer: ReturnType<typeof setTimeout> | undefined;

async function load() {
  if (!availability.commandsAvailable()) return;
  const current = ++generation;
  loading.value = true;
  failed.value = false;
  try {
    const result = await availability.freeBusy(
      { account_id: props.accountId, attendees: props.attendees },
      { start: day.value, end: addDays(day.value, 1) },
    );
    if (current === generation) report.value = result;
  } catch {
    if (current === generation) failed.value = true;
  } finally {
    if (current === generation) loading.value = false;
  }
}

// Typing an address or stepping through days asks once it settles.
watch(
  () => [day.value.getTime(), props.accountId, props.attendees.join(','), calendarUi.availabilityExcluded.join('|')],
  () => {
    clearTimeout(timer);
    timer = setTimeout(() => void load(), 250);
  },
  { immediate: true },
);
onBeforeUnmount(() => clearTimeout(timer));

/** The hours shown: the working day, stretched to hold the slot, an hour either side. */
const view = computed(() => {
  const hours = calendarUi.workingHours;
  const slotStart = (props.start.getTime() - day.value.getTime()) / 60_000;
  const slotEnd = slotStart + durationMinutes.value;
  const from = Math.max(0, Math.floor(Math.min(hours.startMinute, slotStart) / 60) * 60 - 60);
  const to = Math.min(24 * 60, Math.ceil(Math.max(hours.endMinute, slotEnd) / 60) * 60 + 60);
  return {
    start: day.value.getTime() + from * 60_000,
    end: day.value.getTime() + to * 60_000,
    fromMinute: from,
    toMinute: to,
  };
});

function percent(time: number): number {
  const { start, end } = view.value;
  return Math.min(100, Math.max(0, ((time - start) / (end - start)) * 100));
}

const ticks = computed(() => {
  const marks = [];
  const step = ui.isMobile ? 240 : 120;
  for (let minute = Math.ceil(view.value.fromMinute / step) * step; minute < view.value.toMinute; minute += step) {
    const time = day.value.getTime() + minute * 60_000;
    marks.push({ minute, left: percent(time), label: formatTime(new Date(time)) });
  }
  return marks;
});

const slot = computed(() => ({
  left: percent(props.start.getTime()),
  width: Math.max(percent(props.start.getTime() + durationMs.value) - percent(props.start.getTime()), 0.5),
}));

interface Row {
  key: string;
  label: string;
  known: boolean;
  blocks: Array<{ key: string; left: number; width: number; tentative: boolean }>;
  conflict: boolean;
  summary: string;
}

function overlaps(block: BackendBusyBlock): boolean {
  return Date.parse(block.starts_at) < props.start.getTime() + durationMs.value
    && Date.parse(block.ends_at) > props.start.getTime();
}

function timeList(blocks: BackendBusyBlock[]): string {
  return blocks
    .map((block) => `${formatTime(new Date(block.starts_at))}–${formatTime(new Date(block.ends_at))}`)
    .join(', ');
}

/** Overlapping blocks as one, so a double-booked hour reads once. Tentative only if all of it is. */
function merged(blocks: BackendBusyBlock[]): BackendBusyBlock[] {
  const sorted = [...blocks].sort((a, b) => Date.parse(a.starts_at) - Date.parse(b.starts_at));
  const out: BackendBusyBlock[] = [];
  for (const block of sorted) {
    const last = out[out.length - 1];
    if (last && Date.parse(block.starts_at) <= Date.parse(last.ends_at)) {
      if (Date.parse(block.ends_at) > Date.parse(last.ends_at)) last.ends_at = block.ends_at;
      last.tentative = last.tentative && block.tentative;
    } else {
      out.push({ ...block });
    }
  }
  return out;
}

function row(key: string, label: string, known: boolean, blocks: BackendBusyBlock[]): Row {
  const busy = merged(blocks);
  const visible = busy.filter((block) =>
    Date.parse(block.ends_at) > view.value.start && Date.parse(block.starts_at) < view.value.end);
  const conflict = busy.some(overlaps);
  const summary = !known
    ? t('availability.strip.unknown')
    : busy.length === 0
      ? t('availability.strip.freeAllDay')
      : t('availability.strip.busyTimes', { times: timeList(busy) });
  return {
    key,
    label,
    known,
    conflict,
    summary,
    blocks: visible.map((block) => {
      const left = percent(Date.parse(block.starts_at));
      return {
        key: `${block.starts_at}-${block.ends_at}`,
        left,
        width: Math.max(percent(Date.parse(block.ends_at)) - left, 0.5),
        tentative: block.tentative,
      };
    }),
  };
}

const rows = computed<Row[]>(() => {
  const data = report.value;
  const own = row('you', t('availability.strip.you'), true, data?.own ?? []);
  const others = data?.attendees.map((entry) => row(entry.address, entry.address, entry.known, entry.busy)) ?? [];
  return [own, ...others];
});

const anyUnknown = computed(() => rows.value.some((entry) => !entry.known));
const conflicts = computed(() => rows.value.filter((entry) => entry.conflict).length);
const dayLabel = computed(() =>
  day.value.toLocaleDateString(locale.value, { weekday: 'long', month: 'short', day: 'numeric' }));
const slotLabel = computed(() =>
  `${formatTime(props.start)}–${formatTime(new Date(props.start.getTime() + durationMs.value))}`);

function stepDay(offset: number) {
  const start = addDays(props.start, offset);
  emit('move', start, new Date(start.getTime() + durationMs.value));
}

/** A click on the timeline moves the slot to start there, on the quarter hour. */
function moveToPointer(event: PointerEvent) {
  if (props.disabled) return;
  const track = event.currentTarget as HTMLElement;
  const box = track.getBoundingClientRect();
  if (box.width <= 0) return;
  const ratio = Math.min(1, Math.max(0, (event.clientX - box.left) / box.width));
  const minute = view.value.fromMinute + ratio * (view.value.toMinute - view.value.fromMinute);
  const snapped = Math.round(minute / SNAP_MINUTES) * SNAP_MINUTES;
  const start = new Date(day.value);
  start.setHours(0, snapped, 0, 0);
  emit('move', start, new Date(start.getTime() + durationMs.value));
}

async function findNextFree() {
  if (searching.value || props.disabled) return;
  searching.value = true;
  searchMessage.value = '';
  try {
    const result = await availability.freeSlots(
      { account_id: props.accountId, attendees: props.attendees },
      {
        // Strictly after the current proposal, so a free slot steps on.
        from: new Date(props.start.getTime() + 60_000),
        working_days: SEARCH_WORKING_DAYS,
        duration_minutes: durationMinutes.value,
      },
    );
    notCounted.value = result.unknown;
    const next = result.days.flatMap((entry) => entry.windows)[0];
    if (!next) {
      searchMessage.value = t('availability.strip.noneFree', { count: SEARCH_WORKING_DAYS });
      return;
    }
    const start = new Date(next.starts_at);
    emit('move', start, new Date(start.getTime() + durationMs.value));
  } catch (error) {
    searchMessage.value = t('availability.strip.searchFailed', { error: backendErrorLabel(error) });
  } finally {
    searching.value = false;
  }
}

const canSearch = computed(() => durationMinutes.value >= 5 && durationMinutes.value <= 24 * 60);
</script>

<template>
  <section class="strip" :class="{ 'strip--compact': ui.isMobile }" :aria-label="$t('availability.strip.title')" data-testid="availability-strip">
    <header class="strip__head">
      <span class="strip__title">{{ $t('availability.strip.title') }}</span>
      <span class="strip__day">
        <IconButton size="sm" :title="$t('availability.strip.previousDay')" :disabled="disabled" @click="stepDay(-1)">
          <AppIcon><path d="m10 3-5 5 5 5" /></AppIcon>
        </IconButton>
        <span class="strip__day-label">{{ dayLabel }}</span>
        <IconButton size="sm" :title="$t('availability.strip.nextDay')" :disabled="disabled" @click="stepDay(1)">
          <AppIcon><path d="m6 3 5 5-5 5" /></AppIcon>
        </IconButton>
      </span>
    </header>

    <p class="strip__status" role="status">
      <template v-if="failed">{{ $t('availability.strip.loadFailed') }}</template>
      <template v-else-if="loading && !report">{{ $t('availability.strip.loading') }}</template>
      <template v-else-if="conflicts > 0">{{ $t('availability.strip.conflicts', { count: conflicts, time: slotLabel }, conflicts) }}</template>
      <template v-else>{{ $t('availability.strip.allFreeAt', { time: slotLabel }) }}</template>
    </p>

    <ul v-if="ui.isMobile" class="strip__list">
      <li v-for="entry in rows" :key="entry.key" class="strip__list-row" :class="{ 'strip__list-row--conflict': entry.conflict, 'strip__list-row--unknown': !entry.known }">
        <span class="strip__name">{{ entry.label }}</span>
        <span class="strip__verdict">
          {{ !entry.known ? $t('availability.strip.unknown') : entry.conflict ? $t('availability.strip.busyThen') : $t('availability.strip.freeThen') }}
        </span>
        <span v-if="entry.known" class="strip__times">{{ entry.summary }}</span>
      </li>
    </ul>

    <div v-else class="strip__grid">
      <span class="strip__axis-spacer" aria-hidden="true" />
      <div class="strip__axis" aria-hidden="true">
        <span v-for="tick in ticks" :key="tick.minute" class="strip__tick" :style="{ left: `${tick.left}%` }">{{ tick.label }}</span>
      </div>
      <template v-for="entry in rows" :key="entry.key">
        <span class="strip__name" :title="entry.label">{{ entry.label }}</span>
        <div
          class="strip__track"
          :class="{ 'strip__track--unknown': !entry.known }"
          :title="`${entry.label}: ${entry.summary}`"
          @pointerdown="moveToPointer"
        >
          <span class="sr-only">{{ entry.label }}: {{ entry.summary }}</span>
          <span
            v-for="block in entry.blocks"
            :key="block.key"
            class="strip__busy"
            :class="{ 'strip__busy--tentative': block.tentative }"
            :style="{ left: `${block.left}%`, width: `${block.width}%` }"
            aria-hidden="true"
          />
          <span
            class="strip__slot"
            :class="{ 'strip__slot--conflict': entry.conflict }"
            :style="{ left: `${slot.left}%`, width: `${slot.width}%` }"
            aria-hidden="true"
          />
        </div>
      </template>
    </div>

    <footer class="strip__foot">
      <AppButton size="sm" variant="secondary" :disabled="disabled || searching || !canSearch" data-testid="availability-next-free" @click="findNextFree">
        {{ searching ? $t('availability.strip.searching') : $t('availability.strip.nextFree') }}
      </AppButton>
      <span v-if="searchMessage" class="strip__note" role="status">{{ searchMessage }}</span>
    </footer>
    <p v-if="notCounted.length" class="strip__note">{{ $t('availability.strip.notCounted', { people: notCounted.join(', ') }) }}</p>
    <p v-else-if="anyUnknown" class="strip__note">{{ $t('availability.strip.unknownHint') }}</p>
  </section>
</template>

<style scoped>
.strip {
  display: grid;
  gap: var(--space-xs);
  border: var(--border-muted);
  border-radius: var(--radius-md);
  padding: var(--space-sm);
}

.strip__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-xs);
  min-width: 0;
}

.strip__title {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-strong);
}

.strip__day {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2xs);
  min-width: 0;
}

.strip__day-label {
  overflow: hidden;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.strip__status,
.strip__note {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.strip__grid {
  display: grid;
  grid-template-columns: minmax(4.5rem, 9rem) minmax(0, 1fr);
  align-items: center;
  column-gap: var(--space-sm);
  row-gap: var(--space-2xs);
}

.strip__axis {
  position: relative;
  height: 1.1rem;
}

.strip__tick {
  position: absolute;
  top: 0;
  transform: translateX(-50%);
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  white-space: nowrap;
}

.strip__name {
  overflow: hidden;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.strip__track {
  position: relative;
  height: 1.25rem;
  overflow: hidden;
  border-radius: var(--radius-xs);
  background: var(--bg-inset);
  cursor: pointer;
}

.strip__track--unknown {
  background: repeating-linear-gradient(
    135deg,
    var(--bg-inset) 0 6px,
    var(--bg-hover) 6px 12px
  );
}

.strip__busy {
  position: absolute;
  top: 0;
  bottom: 0;
  background: color-mix(in srgb, var(--accent-primary) 55%, transparent);
}

.strip__busy--tentative {
  background: repeating-linear-gradient(
    135deg,
    color-mix(in srgb, var(--accent-primary) 55%, transparent) 0 4px,
    transparent 4px 8px
  );
}

.strip__slot {
  position: absolute;
  top: 0;
  bottom: 0;
  border: 2px solid var(--accent-green);
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--accent-green) 14%, transparent);
  pointer-events: none;
}

.strip__slot--conflict {
  border-color: var(--accent-red);
  background: color-mix(in srgb, var(--accent-red) 14%, transparent);
}

.strip__foot {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-sm);
}

.strip__list {
  display: grid;
  gap: var(--space-xs);
  margin: 0;
  padding: 0;
  list-style: none;
}

.strip__list-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: var(--space-2xs) var(--space-sm);
  border-left: 3px solid var(--accent-green);
  padding-left: var(--space-sm);
}

.strip__list-row--conflict {
  border-left-color: var(--accent-red);
}

.strip__list-row--unknown {
  border-left-color: var(--border-color);
}

.strip__list-row .strip__name {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}

.strip__verdict {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.strip__times {
  grid-column: 1 / -1;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}
</style>
