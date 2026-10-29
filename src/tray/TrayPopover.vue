<script setup lang="ts">
import { parseCalendarBoundary } from '@/utils/calendarDates';
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import Avatar from '@/components/common/Avatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import SkeletonRow from '@/components/ui/SkeletonRow.vue';
import {
  backendActOnTrayThread,
  backendOpenTrayDestination,
  backendOpenTrayThread,
  backendSupportsNativeCommands,
  backendTraySnapshot,
  type BackendTrayCalendarEvent,
  type BackendTraySnapshot,
  type BackendTrayThread,
} from '@/services/backend';

const { locale, t } = useI18n();
const snapshot = ref<BackendTraySnapshot | null>(null);
const loading = ref(true);
const loadError = ref('');
const actingThreadId = ref<string | null>(null);
const actionError = ref('');
let focusUnlisten: (() => void) | null = null;
let loadPromise: Promise<void> | null = null;

const native = backendSupportsNativeCommands();
const now = ref(Date.now());
const todayLabel = computed(() =>
  new Intl.DateTimeFormat(locale.value, {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
  }).format(new Date(now.value)),
);

const mockSnapshot = (): BackendTraySnapshot => {
  const now = Date.now();
  const nextStart = new Date(now);
  nextStart.setDate(nextStart.getDate() + 3);
  nextStart.setHours(10, 0, 0, 0);
  const nextEnd = new Date(nextStart.getTime() + 60 * 60_000);
  return {
    new_threads: [
      {
        id: 'tray-demo-1',
        sender_name: 'Mara Hoffmann',
        sender_address: 'mara@example.com',
        subject: t('tray.demo.releaseApproval'),
        received_at: new Date(now - 4 * 60_000).toISOString(),
        is_unread: true,
      },
      {
        id: 'tray-demo-2',
        sender_name: 'Jonas Weber',
        sender_address: 'jonas@example.com',
        subject: t('tray.demo.workshopNotes'),
        received_at: new Date(now - 18 * 60_000).toISOString(),
        is_unread: true,
      },
    ],
    upcoming_events: [
      {
        title: t('tray.demo.projectMeeting'),
        starts_at: nextStart.toISOString(),
        ends_at: nextEnd.toISOString(),
        all_day: false,
        location: 'Google Meet',
      },
    ],
  };
};

const threads = computed(() => snapshot.value?.new_threads ?? []);
const upcomingEvents = computed(() => snapshot.value?.upcoming_events ?? []);

function relativeMailTime(value: string): string {
  const elapsed = Math.max(0, now.value - Date.parse(value));
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 1) return t('tray.time.justNow');
  if (minutes < 60) return t('tray.time.minutesAgo', { count: minutes });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return t('tray.time.hoursAgo', { count: hours });
  return new Intl.DateTimeFormat(locale.value, { day: '2-digit', month: '2-digit' }).format(
    new Date(value),
  );
}

function eventTime(event: BackendTrayCalendarEvent): string {
  if (event.all_day) return t('tray.allDay');
  return new Intl.DateTimeFormat(locale.value, { hour: '2-digit', minute: '2-digit' }).format(
    new Date(event.starts_at),
  );
}

function localDayStart(value: Date): Date {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate());
}

function eventDay(event: BackendTrayCalendarEvent): string {
  const today = localDayStart(new Date(now.value));
  const start = localDayStart(parseCalendarBoundary(event.starts_at, event.all_day));
  const days = Math.round((start.getTime() - today.getTime()) / 86_400_000);
  if (days === 0) return t('common.today');
  if (days === 1) return t('common.tomorrow');
  return new Intl.DateTimeFormat(locale.value, {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
  }).format(start);
}

function eventStatus(event: BackendTrayCalendarEvent): string {
  const currentTime = now.value;
  const start = parseCalendarBoundary(event.starts_at, event.all_day).getTime();
  const end = parseCalendarBoundary(event.ends_at, event.all_day).getTime();
  if (start <= currentTime && end > currentTime) {
    const minutes = Math.max(1, Math.ceil((end - currentTime) / 60_000));
    if (minutes < 60) return t('tray.event.runningMinutes', { count: minutes });
    if (minutes < 24 * 60) return t('tray.event.runningHours', { count: Math.ceil(minutes / 60) });
    return t('tray.event.runningUntil', { day: eventDay(event) });
  }
  const minutes = Math.max(0, Math.round((start - currentTime) / 60_000));
  if (minutes < 60) return t('tray.event.inMinutes', { count: minutes });
  if (
    localDayStart(new Date(start)).getTime()
    === localDayStart(new Date(currentTime)).getTime()
  ) {
    return t('tray.event.inHours', { count: Math.round(minutes / 60) });
  }
  const days = Math.max(
    1,
    Math.round(
      (localDayStart(new Date(start)).getTime()
        - localDayStart(new Date(currentTime)).getTime())
      / 86_400_000,
    ),
  );
  return days === 1 ? t('tray.event.tomorrow') : t('tray.event.inDays', { count: days });
}

function eventLocation(value: string | null | undefined): string {
  if (!value) return '';
  try {
    const host = new URL(value).hostname.toLowerCase();
    if (host.includes('zoom')) return 'Zoom';
    if (host === 'meet.google.com') return 'Google Meet';
    if (host.includes('teams.microsoft.com')) return 'Microsoft Teams';
    return host.replace(/^www\./, '');
  } catch {
    return value;
  }
}

function threadAriaLabel(thread: BackendTrayThread): string {
  const state = thread.is_unread ? t('tray.unread') : t('tray.read');
  return `${state}, ${threadSender(thread)}, ${threadSubject(thread)}, ${relativeMailTime(thread.received_at)}`;
}

function threadSender(thread: BackendTrayThread): string {
  return thread.sender_name || t('tray.unknownSender');
}

function threadSubject(thread: BackendTrayThread): string {
  return thread.subject || t('tray.noSubject');
}

async function loadSnapshot() {
  if (loadPromise) return loadPromise;
  now.value = Date.now();
  loading.value = snapshot.value === null;
  loadError.value = '';
  loadPromise = (async () => {
    try {
      snapshot.value = native ? await backendTraySnapshot() : mockSnapshot();
    } catch (error) {
      loadError.value = error instanceof Error ? error.message : String(error);
    } finally {
      loading.value = false;
    }
  })();
  try {
    await loadPromise;
  } finally {
    loadPromise = null;
  }
}

async function hidePopover() {
  if (!native) return;
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().hide();
}

async function openThread(threadId: string) {
  if (native) await backendOpenTrayThread(threadId);
}

async function actOnThread(
  threadId: string,
  action: 'done' | 'waiting',
  event: MouseEvent,
) {
  event.stopPropagation();
  if (actingThreadId.value) return;
  actingThreadId.value = threadId;
  actionError.value = '';
  try {
    if (native) await backendActOnTrayThread(threadId, action);
    if (snapshot.value) {
      snapshot.value = {
        ...snapshot.value,
        new_threads: snapshot.value.new_threads.filter((thread) => thread.id !== threadId),
      };
    }
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    actionError.value = t('tray.actionFailed', { detail });
  } finally {
    actingThreadId.value = null;
  }
}

async function openDestination(
  destination: 'inbox' | 'calendar' | 'compose' | 'settings' | 'calendar_settings',
) {
  if (native) await backendOpenTrayDestination(destination);
}

function onKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape') return;
  event.preventDefault();
  void hidePopover();
}

onMounted(async () => {
  window.addEventListener('keydown', onKeydown);
  if (native) {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const currentWindow = getCurrentWindow();
    focusUnlisten = await currentWindow.onFocusChanged(({ payload }) => {
      if (payload) {
        void loadSnapshot();
      } else {
        void currentWindow.hide();
      }
    });
    // The webview is normally created hidden. If it was opened before its
    // listener finished registering, visibility is the reliable fallback.
    if (await currentWindow.isVisible()) await loadSnapshot();
  } else {
    await loadSnapshot();
  }
});

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown);
  focusUnlisten?.();
});
</script>

<template>
  <main class="briefing-shell" :aria-label="t('tray.briefing')">
    <header class="briefing-header">
      <p class="briefing-date">{{ todayLabel }}</p>
      <IconButton
        size="sm"
        :title="t('tray.calendarSettings')"
        @click="openDestination('calendar_settings')"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
          <circle cx="12" cy="12" r="3" />
        </svg>
      </IconButton>
    </header>

    <div v-if="loading" class="briefing-loading" :aria-label="t('tray.loading')">
      <SkeletonRow v-for="index in 3" :key="index" :lines="2" />
    </div>

    <section v-else-if="loadError" class="briefing-error" role="alert">
      <div class="error-mark">!</div>
      <div>
        <strong>{{ t('tray.unavailable') }}</strong>
        <p>{{ loadError }}</p>
      </div>
      <AppButton size="sm" variant="soft" @click="loadSnapshot">{{ t('common.retry') }}</AppButton>
    </section>

    <div v-else class="briefing-content">
      <section class="briefing-section" aria-labelledby="new-mail-heading">
        <SectionHeading id="new-mail-heading" :label="t('tray.newMail')">
          <template #trailing>
            <AppButton size="sm" variant="ghost" @click="openDestination('inbox')">
              {{ t('tray.inbox') }}
            </AppButton>
          </template>
        </SectionHeading>

        <div v-if="threads.length" class="mail-list">
          <article
            v-for="thread in threads"
            :key="thread.id"
            class="mail-card"
          >
            <button
              type="button"
              class="mail-card__open"
              :aria-label="threadAriaLabel(thread)"
              @click="openThread(thread.id)"
            >
              <Avatar :name="threadSender(thread)" :email="null" :size="36" />
              <span class="mail-copy">
                <span class="mail-line">
                  <span class="mail-sender">
                    <span v-if="thread.is_unread" class="unread-dot" aria-hidden="true" />
                    <strong>{{ threadSender(thread) }}</strong>
                  </span>
                  <time :datetime="thread.received_at">{{ relativeMailTime(thread.received_at) }}</time>
                </span>
                <span class="mail-subject">{{ threadSubject(thread) }}</span>
              </span>
            </button>
            <div
              class="mail-card__actions"
              :aria-busy="actingThreadId === thread.id"
            >
              <IconButton
                size="sm"
                :title="t('tray.markDone')"
                :disabled="actingThreadId !== null"
                @click="actOnThread(thread.id, 'done', $event)"
              >
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <path d="M4 8l3 3 5-6" />
                </svg>
              </IconButton>
              <IconButton
                size="sm"
                :title="t('tray.waitForReply')"
                :disabled="actingThreadId !== null"
                @click="actOnThread(thread.id, 'waiting', $event)"
              >
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <circle cx="8" cy="8" r="6" />
                  <path d="M8 4v4l2.5 1.5" />
                </svg>
              </IconButton>
            </div>
          </article>
          <p v-if="actionError" class="mail-action-error" role="alert">
            {{ actionError }}
          </p>
        </div>

        <div v-else class="all-clear">
          <span class="all-clear__mark" aria-hidden="true">
            <svg width="18" height="18" viewBox="0 0 18 18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
              <path d="m4 9 3 3 7-7" />
            </svg>
          </span>
          <span>
            <strong>{{ t('tray.allClear.title') }}</strong>
            <small>{{ t('tray.allClear.description') }}</small>
          </span>
        </div>
      </section>

      <section class="briefing-section" aria-labelledby="next-event-heading">
        <SectionHeading id="next-event-heading" :label="t('tray.upNext')">
          <template #trailing>
            <AppButton size="sm" variant="ghost" @click="openDestination('calendar')">
              {{ t('tray.calendar') }}
            </AppButton>
          </template>
        </SectionHeading>

        <div v-if="upcomingEvents.length" class="event-list">
          <button
            v-for="event in upcomingEvents"
            :key="`${event.starts_at}-${event.title}`"
            type="button"
            class="event-card"
            :aria-label="`${event.title}, ${eventDay(event)}, ${eventTime(event)}, ${eventStatus(event)}`"
            @click="openDestination('calendar')"
          >
            <span class="event-time">
              <strong>{{ eventDay(event) }}</strong>
              <small>{{ eventTime(event) }}</small>
            </span>
            <span class="event-copy">
              <strong>{{ event.title }}</strong>
              <small>
                {{ eventStatus(event) }}<template v-if="event.location"> · {{ eventLocation(event.location) }}</template>
              </small>
            </span>
            <svg class="event-chevron" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="m6 3 5 5-5 5" />
            </svg>
          </button>
        </div>
        <div v-else class="calendar-clear">
          {{ t('tray.noEvents') }}
        </div>
      </section>
    </div>

    <footer class="briefing-footer">
      <AppButton size="sm" variant="soft" @click="openDestination('inbox')">
        {{ t('tray.openApp') }}
      </AppButton>
      <AppButton size="sm" variant="primary" @click="openDestination('compose')">
        <template #icon>
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M8 3v10M3 8h10" />
          </svg>
        </template>
        {{ t('tray.newMessage') }}
      </AppButton>
    </footer>
  </main>
</template>

<style scoped>
.briefing-shell {
  /* The global phone breakpoint keys off this window's width. A menu-bar panel
     is physically narrow but still a pointer UI, so keep Satchel's desk scale. */
  --font-size-3xs: 10px;
  --font-size-2xs: 11px;
  --font-size-xs: 12px;
  --font-size-sm: 13px;
  --font-size-md: 14px;
  --font-size-lg: 15px;
  --font-size-xl: 17px;
  --font-size-2xl: 22px;

  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  overflow: hidden;
  border: var(--border-default);
  border-radius: var(--radius-lg);
  background: var(--bg-base);
}

.briefing-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 52px;
  padding: 10px var(--space-lg);
  border-bottom: var(--border-muted);
}

.briefing-date {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-lg);
  font-weight: var(--weight-strong);
  letter-spacing: -0.01em;
  text-transform: capitalize;
}

.briefing-content {
  display: grid;
  align-content: start;
  gap: 14px;
  min-height: 0;
  padding: 10px;
  overflow: auto;
}

.briefing-section {
  display: grid;
  gap: 3px;
}

.briefing-section :deep(.section-heading) {
  min-height: 28px;
  padding: 0 6px;
}

.briefing-section :deep(.section-heading__label) {
  font-size: var(--font-size-xs);
  font-weight: var(--weight-strong);
}

.briefing-section :deep(.app-button--sm) {
  height: 26px;
  padding: 0 8px;
}

.mail-list {
  display: grid;
  gap: 1px;
}

.mail-card {
  position: relative;
  display: block;
  align-items: center;
  min-height: 62px;
  min-width: 0;
  padding: 7px 6px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  transition:
    background var(--duration-fast) var(--ease-out),
    opacity var(--duration-normal) var(--ease-out);
}

.mail-card:hover {
  background: var(--bg-hover);
}

.mail-card__actions {
  position: absolute;
  top: 50%;
  right: 8px;
  display: flex;
  gap: 2px;
  padding: 2px;
  border: var(--border-muted);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  box-shadow: var(--shadow-sm);
  opacity: 0;
  pointer-events: none;
  transform: translate(4px, -50%);
  transition:
    opacity var(--duration-fast) var(--ease-out),
    transform var(--duration-fast) var(--ease-out);
}

.mail-card:hover .mail-card__actions,
.mail-card:focus-within .mail-card__actions {
  opacity: 1;
  pointer-events: auto;
  transform: translate(0, -50%);
}

.mail-card:hover .mail-copy,
.mail-card:focus-within .mail-copy {
  padding-right: 64px;
}

.mail-card__open {
  display: grid;
  grid-template-columns: 36px minmax(0, 1fr);
  align-items: center;
  gap: 10px;
  width: 100%;
  min-width: 0;
  border: 0;
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.mail-sender {
  display: flex;
  align-items: center;
  min-width: 0;
  gap: 6px;
}

.unread-dot {
  width: 6px;
  height: 6px;
  flex-shrink: 0;
  border-radius: var(--radius-full);
  background: var(--accent-primary);
}

.mail-copy,
.event-copy {
  display: grid;
  min-width: 0;
  gap: 2px;
}

.mail-copy {
  transition: padding-right var(--duration-fast) var(--ease-out);
}

.mail-action-error {
  margin: 2px 8px 0;
  color: var(--accent-red);
  font-size: var(--font-size-2xs);
}

.mail-line {
  display: flex;
  align-items: baseline;
  min-width: 0;
  gap: var(--space-sm);
}

.mail-line strong,
.event-copy strong {
  min-width: 0;
  overflow: hidden;
  color: var(--text-primary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-strong);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mail-line time {
  flex-shrink: 0;
  margin-left: auto;
  color: var(--text-tertiary);
  font-size: var(--font-size-3xs);
  font-variant-numeric: tabular-nums;
}

.mail-subject,
.event-copy small,
.all-clear small {
  overflow: hidden;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.all-clear {
  display: flex;
  align-items: center;
  gap: var(--space-md);
  min-height: 64px;
  padding: 8px 6px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
}

.all-clear__mark {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  flex-shrink: 0;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-green) 14%, transparent);
  color: var(--accent-green);
}

.all-clear > span:last-child {
  display: grid;
  min-width: 0;
  gap: 2px;
}

.all-clear strong {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
}

.event-list {
  display: grid;
  gap: 2px;
}

.event-card {
  position: relative;
  display: grid;
  grid-template-columns: 86px minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-md);
  width: 100%;
  min-height: 62px;
  overflow: hidden;
  padding: 10px 12px;
  border: 0;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-surface) 32%, var(--bg-base));
  text-align: left;
  cursor: pointer;
  transition:
    border-color var(--duration-fast) var(--ease-out),
    background var(--duration-fast) var(--ease-out);
}

.event-card::before {
  position: absolute;
  inset: 11px auto 11px 0;
  width: 3px;
  border-radius: 0 var(--radius-full) var(--radius-full) 0;
  background: var(--accent-primary);
  content: '';
}

.event-card:hover {
  background: var(--bg-hover);
}

.event-time {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.event-time strong {
  overflow: hidden;
  color: var(--accent-primary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-strong);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.event-time small {
  color: var(--text-tertiary);
  font-family: var(--font-mono);
  font-size: var(--font-size-2xs);
  font-variant-numeric: tabular-nums;
}

.event-chevron {
  color: var(--text-tertiary);
}

.calendar-clear {
  padding: 14px 8px;
  border: 0;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-surface) 32%, var(--bg-base));
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  text-align: center;
}

.briefing-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: auto;
  padding: 10px var(--space-lg);
  border-top: var(--border-muted);
  background: var(--bg-base);
}

.briefing-loading {
  display: grid;
  gap: var(--space-sm);
  padding: var(--space-lg);
}

.briefing-error {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: var(--space-md);
  margin: var(--space-lg);
  padding: var(--space-md);
  border: 1px solid color-mix(in srgb, var(--accent-red) 30%, var(--border-color));
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--accent-red) 8%, var(--bg-surface));
}

.briefing-error > :last-child {
  grid-column: 1 / -1;
  justify-self: start;
}

.briefing-error strong {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}

.briefing-error p {
  margin-top: 2px;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.error-mark {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-red) 15%, transparent);
  color: var(--accent-red);
  font-weight: var(--weight-bold);
}

@media (prefers-reduced-motion: reduce) {
  .mail-card,
  .mail-card__actions,
  .mail-copy,
  .event-card {
    transition: none;
  }
}
</style>
