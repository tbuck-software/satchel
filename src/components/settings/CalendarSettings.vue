<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import NavItem from '@/components/ui/NavItem.vue';
import GoogleConnections from '@/components/settings/GoogleConnections.vue';
import { useCalendarUiStore } from '@/stores/calendarUi';
import { useInvitesStore } from '@/stores/invites';
import { translateBackendError } from '@/services/backend';
import {
  collectSources,
  itemFromEvent,
  itemFromInvite,
  type CalendarItem,
} from '@/utils/calendarGrid';

const emit = defineEmits<{ manageAccounts: [] }>();
const { t } = useI18n();
const invites = useInvitesStore();
const calendarUi = useCalendarUiStore();
const syncing = ref(false);
const syncMessage = ref('');

function localizedBackendError(error: unknown): string {
  return translateBackendError(error) ?? (error instanceof Error ? error.message : String(error));
}

const sources = computed(() => {
  const items: CalendarItem[] = [
    ...invites.events.map(itemFromEvent),
    ...invites.items.map(itemFromInvite).filter((item): item is CalendarItem => item !== null),
  ];
  return collectSources(items);
});

onMounted(() => void invites.load());

async function syncNow() {
  if (syncing.value) return;
  syncing.value = true;
  syncMessage.value = '';
  try {
    const errors = await invites.syncRemote();
    syncMessage.value = errors.length
      ? t('settings.calendar.syncPartial', { error: localizedBackendError(errors[0]) })
      : t('settings.calendar.syncUpToDate');
  } catch (error) {
    syncMessage.value = t('settings.calendar.syncFailed', {
      error: localizedBackendError(error),
    });
  } finally {
    syncing.value = false;
  }
}
</script>

<template>
  <section class="calendar-settings">
    <header>
      <h3>{{ $t('settings.sections.calendar.label') }}</h3>
      <p>{{ $t('settings.calendar.description') }}</p>
    </header>

    <GoogleConnections purpose="calendar" @connected="syncNow" />

    <div class="source-panel">
      <div class="source-panel__heading">
        <span>{{ $t('settings.calendar.visibleCalendars') }}</span>
        <small>
          {{ $t('settings.calendar.sourceCount', { count: sources.length }, sources.length) }}
        </small>
      </div>
      <div v-if="sources.length" class="source-list">
        <NavItem
          v-for="source in sources"
          :key="source.key"
          class="source-row"
          :class="{ 'source-row--hidden': calendarUi.isSourceHidden(source.key) }"
          :label="source.label"
          :pressed="!calendarUi.isSourceHidden(source.key)"
          @click="calendarUi.toggleSource(source.key)"
        >
          <template #leading>
            <span class="source-dot" :style="{ backgroundColor: source.color }" aria-hidden="true">
              <svg v-if="!calendarUi.isSourceHidden(source.key)" width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                <path d="m2 5.2 2 2 4-4" />
              </svg>
            </span>
          </template>
          <template #trailing>
            <span class="source-count">
              {{ $t('settings.calendar.eventCount', { count: source.count }, source.count) }}
            </span>
          </template>
        </NavItem>
      </div>
      <p v-else class="source-empty">{{ $t('settings.calendar.noSources') }}</p>
    </div>

    <p v-if="syncMessage" class="sync-message" role="status">{{ syncMessage }}</p>

    <div class="calendar-actions">
      <AppButton variant="primary" :disabled="syncing" @click="syncNow">
        {{ syncing ? $t('settings.calendar.syncing') : $t('settings.calendar.syncNow') }}
      </AppButton>
      <AppButton variant="soft" @click="emit('manageAccounts')">
        {{ $t('settings.calendar.manageAccounts') }}
      </AppButton>
    </div>
  </section>
</template>

<style scoped>
.calendar-settings {
  display: grid;
  gap: 20px;
}

header h3 {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
}

header p {
  margin: 4px 0 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.source-panel {
  overflow: hidden;
  border: var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
}

.source-panel__heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 42px;
  padding: 0 var(--space-md);
  border-bottom: var(--border-muted);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-strong);
}

.source-panel__heading small,
.source-count {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-medium);
}

.source-list {
  display: grid;
  padding: 6px;
}

.source-dot {
  display: grid;
  place-items: center;
  width: 16px;
  height: 16px;
  border-radius: var(--radius-xs);
  color: var(--text-on-accent);
}

.source-row--hidden .source-dot {
  opacity: 0.3;
}

/* The row is a NavItem; only the muted state is ours. */
.source-row--hidden :deep(.nav-item__label) {
  color: var(--text-tertiary);
}

.source-empty {
  margin: 0;
  padding: 24px;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
  text-align: center;
}

.sync-message {
  margin: -8px 0 0;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.calendar-actions {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
}
</style>
