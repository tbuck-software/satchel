<script setup lang="ts">
/**
 * The phone's status: a small chip on the list's subtitle line, beside the
 * account it is about.
 *
 * It replaces the bands that sat above the page title (sync line, failure
 * banner, connection error, expired sign-in). Those stacked, took a third of
 * the screen when two came at once, and squeezed their buttons into a
 * five-line column. Here the state takes one word or two; the sentence, the
 * server's answer and the fix are one tap away in the status sheet. The tab
 * bar's Mail dot carries the urgent ones to the other screens.
 *
 * Offline and routine syncing are plain status text. An older-mail import
 * opens the same progress details as the desktop sidebar.
 */
import { computed, defineAsyncComponent, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import CardButton from '@/components/ui/CardButton.vue';
import { useShellStatus } from '@/composables/useShellStatus';

const ShellStatusSheet = defineAsyncComponent(() => import('./ShellStatusSheet.vue'));
const SyncProgressDetails = defineAsyncComponent(() => import('./SyncProgressDetails.vue'));
const progressOpen = ref(false);
const SyncFailures = defineAsyncComponent(() => import('./SyncFailures.vue'));

const { t } = useI18n();
const status = useShellStatus();
const sheetOpen = ref(false);
const failuresOpen = ref(false);
const showProgress = computed(() => status.progress.history.value
  && (status.kind.value === 'syncing' || status.kind.value === 'offline'));
const actionable = computed(() => status.kind.value !== null
  && (showProgress.value || (status.kind.value !== 'offline' && status.kind.value !== 'syncing')));
const title = computed(() => showProgress.value
  ? [status.progress.counts.value, status.progress.phase.value, t('app.sync.progress.details')].filter(Boolean).join(' · ')
  : [status.label.value, t('app.status.details')].join(' · '));

let opener: HTMLElement | null = null;
function open(event: MouseEvent) {
  opener = event.currentTarget as HTMLElement;
  opener.focus();
  if (showProgress.value) progressOpen.value = true;
  else if (status.needsSheet.value) sheetOpen.value = true;
  else failuresOpen.value = true;
}

/*
 * The sheet hands the focus back to what opened it, but on a busy page that
 * hand-back can miss and the focus drops to <body> with the leaving sheet.
 * Then the chip takes it.
 */
function close() {
  progressOpen.value = false;
  sheetOpen.value = false;
  failuresOpen.value = false;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const active = document.activeElement;
    if (active && active !== document.body && !active.closest('.sheet-root')) return;
    const target = opener?.isConnected ? opener : document.querySelector<HTMLElement>('.large-title__switch');
    target?.focus({ preventScroll: true });
  }));
}

/** From the sheet to the queue: one sheet at a time. */
function review() {
  sheetOpen.value = false;
  failuresOpen.value = true;
}
</script>

<template>
  <span v-if="status.kind.value || sheetOpen || failuresOpen || progressOpen" class="shell-status">
    <CardButton
      v-if="actionable"
      class="shell-status__chip"
      :class="[`shell-status__chip--${status.tone.value}`, { 'shell-status__chip--quiet': showProgress }, `shell-status__chip--${status.kind.value}`]"
      :title="title"
      aria-haspopup="dialog"
      :aria-expanded="sheetOpen || failuresOpen || progressOpen"
      @click="open"
    >
      <span class="shell-status__dot" aria-hidden="true" />
      <span class="shell-status__label">{{ status.label.value }}</span>
      <span v-if="status.more.value > 0" class="shell-status__more">{{ t('app.status.more', { count: status.more.value }) }}</span>
    </CardButton>
    <span
      v-else-if="status.kind.value"
      class="shell-status__chip shell-status__chip--quiet"
      :class="[`shell-status__chip--${status.tone.value}`, `shell-status__chip--${status.kind.value}`]"
      role="status"
    >
      <span class="shell-status__dot" aria-hidden="true" />
      <span class="shell-status__label">{{ status.label.value }}</span>
    </span>
    <SyncProgressDetails v-if="progressOpen" :open="progressOpen" @close="close" />
    <ShellStatusSheet v-if="sheetOpen" :open="sheetOpen" @close="close" @review="review" />
    <SyncFailures v-if="failuresOpen" :open="failuresOpen" @close="close" />
  </span>
</template>

<style scoped>
.shell-status {
  display: inline-flex;
  flex-shrink: 0;
  min-width: 0;
  max-width: 60%;
}

.shell-status__chip {
  --chip-tone: var(--text-secondary);
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: var(--space-1-5);
  min-width: 0;
  height: 22px;
  padding: 0 var(--space-sm);
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--chip-tone) 12%, transparent);
  color: var(--chip-tone);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-medium);
  line-height: 1;
  white-space: nowrap;
}

/* A 22px chip, a 44px finger: the hit area grows, the chip does not. */
.shell-status__chip:is(button)::after {
  content: '';
  position: absolute;
  inset: calc((22px - var(--touch-target)) / 2) calc(-1 * var(--space-xs));
}

.shell-status__chip--danger { --chip-tone: var(--accent-red-soft); }
.shell-status__chip--warning { --chip-tone: var(--accent-amber-soft); }

/* Plain states read as part of the caption, not as an alert. */
.shell-status__chip--quiet {
  padding: 0;
  background: none;
  font-weight: var(--weight-regular);
}

.shell-status__chip--syncing { --chip-tone: var(--text-tertiary); }

.shell-status__dot {
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}

.shell-status__chip--syncing .shell-status__dot {
  background: var(--accent-primary);
  animation: shell-status-pulse 1.4s var(--ease-in-out) infinite;
}

.shell-status__label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.shell-status__more {
  flex-shrink: 0;
  opacity: 0.8;
  font-variant-numeric: tabular-nums;
}

@keyframes shell-status-pulse {
  50% { opacity: 0.3; }
}

@media (prefers-reduced-motion: reduce) {
  .shell-status__chip--syncing .shell-status__dot { animation: none; }
}
</style>
