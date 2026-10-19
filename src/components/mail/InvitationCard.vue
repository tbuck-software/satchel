<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useConfirm } from '@/composables/useConfirm';
import type { BackendCalendarInvite } from '@/types/calendar';
import { useInvitesStore } from '@/stores/invites';
import { icsDescriptionToText } from '@/utils/inviteText';
import { useLinkOpener } from '@/composables/useLinkOpener';
import AppButton from '@/components/ui/AppButton.vue';
import ExternalLinkDialog from './ExternalLinkDialog.vue';
import PlainTextLinks from './PlainTextLinks.vue';

const props = defineProps<{
  invite: BackendCalendarInvite;
}>();

const { locale, t } = useI18n();
const { ask } = useConfirm();
const invitesStore = useInvitesStore();
const responding = ref<'accepted' | 'declined' | 'tentative' | null>(null);
const responseError = ref('');
const linkOpener = useLinkOpener();

const status = computed(() => props.invite.rsvp_status);
// A cancelled meeting is not something you can still reply to.
const cancelled = computed(() => (props.invite.cancelled ?? 0) === 1);
const recurring = computed(() => Boolean(props.invite.recurring));
const occurrence = computed(() => Boolean(props.invite.recurrence_id));
const wholeSeries = computed(() => recurring.value && !occurrence.value);
const unboundedSeries = computed(
  () => wholeSeries.value && Boolean(props.invite.recurrence_unbounded),
);

const state = computed(() => {
  if (cancelled.value) return 'cancelled';
  if (status.value === 'accepted' || status.value === 'declined' || status.value === 'tentative') {
    return status.value;
  }
  return 'open';
});

const stateLabel = computed(() => {
  switch (state.value) {
    case 'accepted':
      return t('mail.invite.state.accepted');
    case 'tentative':
      return t('mail.invite.state.tentative');
    case 'declined':
      return t('mail.invite.state.declined');
    case 'cancelled':
      return t('mail.invite.state.cancelled');
    default:
      return t('mail.invite.state.open');
  }
});

const startDate = computed(() =>
  props.invite.starts_at ? new Date(props.invite.starts_at) : null,
);

const leafMonth = computed(() =>
  startDate.value
    ? startDate.value.toLocaleDateString(locale.value, { month: 'short' }).replace('.', '')
    : '',
);

const leafDay = computed(() => (startDate.value ? startDate.value.getDate() : null));

const when = computed(() => {
  const start = startDate.value;
  if (!start) return '';
  const end = props.invite.ends_at ? new Date(props.invite.ends_at) : null;
  const time = (date: Date) => date.toLocaleTimeString(locale.value, { hour: '2-digit', minute: '2-digit' });
  if (!end) {
    return `${start.toLocaleDateString(locale.value, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })}, ${time(start)}`;
  }
  if (start.toDateString() === end.toDateString()) {
    return `${start.toLocaleDateString(locale.value, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })}, ${time(start)} – ${time(end)}`;
  }
  const day = (date: Date) => date.toLocaleDateString(locale.value, { day: 'numeric', month: 'short', year: 'numeric' });
  return `${day(start)}, ${time(start)} – ${day(end)}, ${time(end)}`;
});

// Meeting locations are usually conference URLs; show the host, open the full link.
const locationUrl = computed(() => {
  const raw = props.invite.location?.trim();
  if (!raw) return null;
  try {
    const parsed = new URL(raw);
    return parsed.protocol === 'https:' || parsed.protocol === 'http:' ? parsed : null;
  } catch {
    return null;
  }
});

function openLocation(event: MouseEvent) {
  const url = locationUrl.value;
  if (!url) return;
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  linkOpener.open({
    href: url.toString(),
    label: url.hostname,
    anchor: { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom },
  });
}

// The organizer's notes from the ICS DESCRIPTION — often the only place
// where agenda, dial-in codes, or context live.
const notes = computed(() => icsDescriptionToText(props.invite.description ?? ''));
const notesExpanded = ref(false);
const notesOverflow = computed(
  () => notes.value.split('\n').length > 5 || notes.value.length > 400,
);

async function respond(response: 'accepted' | 'declined' | 'tentative') {
  if (responding.value !== null) return;
  if (unboundedSeries.value) {
    const confirmed = await ask({
      title: t('mail.invite.unboundedSeriesTitle'),
      body: t('mail.invite.unboundedSeriesConfirm'),
      confirmLabel: t('mail.invite.unboundedSeriesAction'),
    });
    if (!confirmed) return;
  }
  responseError.value = '';
  responding.value = response;
  try {
    await invitesStore.respond(props.invite.id, response);
  } catch (err) {
    responseError.value = err instanceof Error ? err.message : String(err);
  } finally {
    responding.value = null;
  }
}

function actionLabel(response: 'accepted' | 'declined' | 'tentative') {
  if (responding.value === response) return t('mail.invite.sending');
  if (response === 'accepted') {
    return t(wholeSeries.value ? 'mail.invite.acceptSeries' : 'mail.invite.accept');
  }
  if (response === 'tentative') {
    return t(wholeSeries.value ? 'mail.invite.maybeSeries' : 'mail.invite.maybe');
  }
  return t(wholeSeries.value ? 'mail.invite.declineSeries' : 'mail.invite.decline');
}
</script>

<template>
  <aside class="invite" :class="[`invite--${state}`, { cancelled }]">
    <div v-if="leafDay !== null" class="leaf" aria-hidden="true">
      <span class="leaf__month">{{ leafMonth }}</span>
      <span class="leaf__day">{{ leafDay }}</span>
    </div>

    <div class="invite__body">
      <p class="eyebrow">{{ stateLabel }}</p>
      <h4 class="title">{{ props.invite.summary || $t('mail.invite.noTitle') }}</h4>
      <p v-if="when" class="when">{{ when }}</p>
      <p v-if="props.invite.location || props.invite.organizer_email" class="meta">
        <AppButton
          v-if="locationUrl"
          class="meta__link"
          :title="locationUrl.toString()"
          @click="openLocation"
          variant="link"
          size="sm"
        >
          {{ locationUrl.hostname }}
        </AppButton>
        <span v-else-if="props.invite.location">{{ props.invite.location }}</span>
        <span v-if="props.invite.location && props.invite.organizer_email" class="meta__sep">·</span>
        <span v-if="props.invite.organizer_email">
          {{ $t('mail.invite.invitedBy', { name: props.invite.organizer_name || props.invite.organizer_email }) }}
        </span>
      </p>

      <div v-if="notes" class="notes" :class="{ 'notes--clamped': notesOverflow && !notesExpanded }">
        <PlainTextLinks :text="notes" @open-link="linkOpener.open" />
      </div>
      <AppButton
        v-if="notesOverflow"
        class="notes-toggle"
        @click="notesExpanded = !notesExpanded"
        variant="link"
        size="sm"
      >
        {{ notesExpanded ? $t('mail.invite.showLess') : $t('mail.invite.showMore') }}
      </AppButton>

      <p v-if="cancelled" class="cancelled-note">{{ $t('mail.invite.cancelledNote') }}</p>
      <p v-else-if="unboundedSeries" class="recurrence-warning">
        {{ $t('mail.invite.unboundedSeriesWarning') }}
      </p>
      <p v-else-if="wholeSeries" class="recurrence-note">
        {{ $t('mail.invite.wholeSeriesNote') }}
      </p>
      <p v-else-if="occurrence" class="recurrence-note">
        {{ $t('mail.invite.occurrenceNote') }}
      </p>

      <p v-if="responseError" class="response-error" role="alert">{{ responseError }}</p>

      <div v-if="!cancelled" class="actions">
        <AppButton
          size="sm"
          :variant="state === 'open' ? 'primary' : 'ghost'"
          :disabled="responding !== null"
          @click="respond('accepted')"
        >
          {{ actionLabel('accepted') }}
        </AppButton>
        <AppButton
          size="sm"
          variant="ghost"
          :disabled="responding !== null"
          @click="respond('tentative')"
        >
          {{ actionLabel('tentative') }}
        </AppButton>
        <AppButton
          size="sm"
          variant="ghost"
          :disabled="responding !== null"
          @click="respond('declined')"
        >
          {{ actionLabel('declined') }}
        </AppButton>
      </div>
    </div>
    <ExternalLinkDialog
      :assessment="linkOpener.pending.value"
      @cancel="linkOpener.cancel"
      @confirm="linkOpener.confirm"
    />
  </aside>
</template>

<style scoped>
.invite {
  display: flex;
  gap: var(--space-lg);
  margin: var(--space-lg) 0;
  --invite-accent: var(--accent-primary);
}

.invite--accepted {
  --invite-accent: var(--accent-green);
}
.invite--tentative {
  --invite-accent: var(--accent-amber);
}
.invite--declined,
.invite--cancelled {
  --invite-accent: var(--accent-red);
}

.leaf {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2px;
  flex-shrink: 0;
  width: 52px;
  height: 56px;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
}

.leaf__month {
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--invite-accent);
  line-height: 1;
}

.leaf__day {
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  line-height: 1;
}

.cancelled .leaf__month,
.cancelled .leaf__day {
  color: var(--text-tertiary);
}

.recurrence-warning,
.recurrence-note,
.response-error {
  margin: var(--space-xs) 0 0;
  font-size: var(--font-size-xs);
  line-height: 1.45;
}

.recurrence-warning {
  color: var(--accent-amber);
}

.recurrence-note {
  color: var(--text-secondary);
}

.response-error {
  color: var(--accent-red);
}

.invite__body {
  min-width: 0;
  display: grid;
  gap: 3px;
  align-content: start;
}

.eyebrow {
  margin: 0;
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-semibold);
  text-transform: uppercase;
  letter-spacing: 0.07em;
  color: var(--invite-accent);
}

.title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.cancelled .title {
  text-decoration: line-through;
  color: var(--text-secondary);
}

.when {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.meta {
  margin: 0;
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
  font-size: var(--font-size-sm);
  color: var(--text-tertiary);
}

.meta__sep {
  flex-shrink: 0;
}

.meta__link { overflow: hidden; text-overflow: ellipsis; }

.notes {
  margin-top: var(--space-xs);
  min-width: 0;
}

.notes :deep(pre) {
  margin: 0;
  font-family: var(--font-sans);
  font-size: var(--font-size-sm);
  line-height: 1.5;
  color: var(--text-secondary);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.notes--clamped :deep(pre) {
  display: -webkit-box;
  -webkit-line-clamp: 5;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.notes-toggle { justify-self: start; }

.cancelled-note {
  margin: var(--space-xs) 0 0;
  font-size: var(--font-size-sm);
  color: var(--accent-red);
}

.actions {
  display: flex;
  gap: var(--space-sm);
  margin-top: var(--space-sm);
}
</style>
