<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useUiStore } from '@/stores/ui';
import { useDraftsStore } from '@/stores/drafts';
import { useComposerStore } from '@/stores/composer';
import { useAccountsStore } from '@/stores/accounts';
import { useThreadsStore } from '@/stores/threads';
import { useDraftActions } from '@/composables/useDraftActions';
import AppButton from '@/components/ui/AppButton.vue';
import CountBadge from '@/components/ui/CountBadge.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import IconButton from '@/components/ui/IconButton.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import SkeletonRow from '@/components/ui/SkeletonRow.vue';
import TimeAgo from '@/components/common/TimeAgo.vue';
import Avatar from '@/components/common/Avatar.vue';
import ThreadView from '@/components/mail/ThreadView.vue';
import ListDetailLayout from '@/components/layout/ListDetailLayout.vue';
import type { BackendDraft } from '@/types/composer';
import type { Participant } from '@/types/mail';
import { isOwnEmail, participantDisplayName } from '@/utils/mailIdentity';

const { locale, t } = useI18n();
const ui = useUiStore();
const drafts = useDraftsStore();
const composer = useComposerStore();
const accounts = useAccountsStore();
const threads = useThreadsStore();
// Shared with the draft bubble inside a thread: same delete, same undo.
const draftActions = useDraftActions();

const selectedId = ref('');

onMounted(() => {
  void drafts.load();
});

// A reply draft represents its whole conversation in this list. Most of those
// threads are already in the inbox store; fetch only contexts outside its
// loaded page (archived and older conversations are common here).
watch(
  () => drafts.sorted.map((draft) => draft.thread_id).filter((id): id is string => Boolean(id)),
  (threadIds) => {
    for (const threadId of new Set(threadIds)) void threads.ensureThreadLoaded(threadId);
  },
  { immediate: true },
);

const scheduledIds = computed(() => new Set(drafts.scheduled.map((draft) => draft.id)));

/**
 * A scheduled draft sends itself at a fixed time; an ordinary one waits for
 * you. The store already told them apart for the counter — the list now does
 * too, because it changes what you should do about them.
 */
const groups = computed(() => {
  const scheduled: BackendDraft[] = [];
  const open: BackendDraft[] = [];
  for (const draft of drafts.sorted) {
    if (scheduledIds.value.has(draft.id)) scheduled.push(draft);
    else open.push(draft);
  }
  return [
    { key: 'scheduled', label: t('drafts.groups.scheduled'), items: scheduled },
    { key: 'open', label: t('drafts.title'), items: open },
  ].filter((group) => group.items.length > 0);
});

const selectedDraft = computed(() =>
  drafts.sorted.find((draft) => draft.id === selectedId.value),
);
const selectedThread = computed(() => {
  const threadId = selectedDraft.value?.thread_id;
  return threadId ? threads.threads.find((thread) => thread.id === threadId) : undefined;
});

// Keep the preview on something that still exists.
watch(
  () => drafts.sorted,
  (sorted) => {
    if (!sorted.some((draft) => draft.id === selectedId.value)) {
      selectedId.value = ui.isSinglePane ? '' : sorted[0]?.id ?? '';
    }
  },
  { immediate: true },
);

function closeDetail() {
  selectedId.value = '';
}

onBeforeUnmount(ui.registerBackNavigation({
  isOpen: () => ui.isSinglePane && Boolean(selectedId.value),
  dismiss: closeDetail,
}));

const subtitle = computed(() => {
  if (drafts.sorted.length === 0) return t('drafts.subtitle.nothing');
  const count = drafts.scheduled.length;
  if (count === 0) return t('drafts.subtitle.waiting');
  return t('drafts.subtitle.scheduled', count);
});

function threadFor(draft: BackendDraft) {
  return draft.thread_id
    ? threads.threads.find((thread) => thread.id === draft.thread_id)
    : undefined;
}

function participantLine(draft: BackendDraft): string {
  const thread = threadFor(draft);
  const participants = thread?.participants
    .filter((participant) => !isOwnEmail(accounts.accounts, participant.email))
    .map((participant) => participantDisplayName(accounts.accounts, participant));
  const names = participants?.length ? participants : draft.to;
  return names.length > 0
    ? t('drafts.participantLine', { names: names.join(', ') })
    : t('drafts.noRecipients');
}

function avatarParticipant(draft: BackendDraft): Participant | null {
  const participant = threadFor(draft)?.participants.find(
    (candidate) => !isOwnEmail(accounts.accounts, candidate.email),
  );
  if (participant) return participant;
  const email = draft.to[0]?.trim();
  return email ? { name: email, email } : null;
}

function messageCount(draft: BackendDraft): number {
  return threadFor(draft)?.messageCount ?? 1;
}

function hasAttachments(draft: BackendDraft): boolean {
  return Boolean(threadFor(draft)?.hasAttachments || draft.attachments?.length);
}

function snippet(draft: BackendDraft): string {
  return previewText(draft).replace(/\s+/g, ' ').trim().slice(0, 160);
}

/** Plain-text preview; falls back to stripped HTML for rich-text drafts. */
function previewText(draft: BackendDraft): string {
  const body = (draft.body ?? '').trim();
  if (body) return body;
  const html = draft.body_html ?? '';
  if (!html) return '';
  const doc = new DOMParser().parseFromString(html, 'text/html');
  return doc.body.textContent?.trim() ?? '';
}

function exactTime(value?: string | null): string {
  if (!value) return '';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString(locale.value);
}

async function discardDraft(id: string) {
  await draftActions.discard(id);
}
</script>

<template>
  <ListDetailLayout :has-detail="!!selectedDraft">
    <template #list>
    <div class="drafts">
      <PaneHeader :title="$t('drafts.title')" :subtitle="subtitle" :count="drafts.sorted.length">
        <template #actions>
          <AppButton size="sm" variant="soft" @click="composer.openNew()">{{ $t('drafts.newMail') }}</AppButton>
        </template>
      </PaneHeader>

      <div class="drafts__body">
        <SkeletonRow v-if="drafts.loading && drafts.sorted.length === 0" :count="4" height="76px" />

        <EmptyState
          v-else-if="drafts.sorted.length === 0"
          icon="✉️"
          :title="$t('drafts.empty.title')"
          :description="$t('drafts.empty.description')"
          :action-label="$t('drafts.empty.action')"
          @action="composer.openNew()"
        />

        <div v-else class="groups">
          <section v-for="group in groups" :key="group.key" class="group">
            <SectionHeading :label="group.label" :count="group.items.length" />

            <ul class="list">
              <li
                v-for="draft in group.items"
                :key="draft.id"
                class="row"
                :class="{
                  'row--scheduled': group.key === 'scheduled',
                  'row--busy': draftActions.isBusy(draft.id),
                  'row--active': draft.id === selectedId,
                }"
              >
                <CardButton
                  class="row__open"
                  @click="selectedId = draft.id"
                  @dblclick="composer.openDraft(draft.id)"
                >
                  <span
                    class="row__draft-avatar"
                    :class="{ 'row__draft-avatar--with-participant': avatarParticipant(draft) }"
                    aria-hidden="true"
                  >
                    <Avatar
                      v-if="avatarParticipant(draft)"
                      class="row__avatar-image"
                      :name="avatarParticipant(draft)?.name ?? ''"
                      :email="avatarParticipant(draft)?.email"
                      :size="40"
                    />
                    <svg class="row__draft-pencil" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M13.8 3.2l3 3L7.1 15.9H4.2v-2.8L13.8 3.2Z" />
                      <path d="M12.3 4.8l3 3" />
                    </svg>
                  </span>

                  <span class="row__content">
                    <span class="row__headline">
                      <span class="row__participants">{{ participantLine(draft) }}</span>
                      <TimeAgo
                        v-if="group.key === 'scheduled' ? draft.scheduled_for : draft.updated_at"
                        :date="(group.key === 'scheduled' ? draft.scheduled_for : draft.updated_at) as string"
                        class="row__time"
                        :class="{ 'row__time--scheduled': group.key === 'scheduled' }"
                        :title="group.key === 'scheduled'
                          ? $t('drafts.goesOut', { time: exactTime(draft.scheduled_for) })
                          : $t('drafts.lastSaved', { time: exactTime(draft.updated_at) })"
                      />
                    </span>

                    <span class="row__subject-line">
                      <span class="row__subject">{{ draft.subject || $t('compose.noSubject') }}</span>
                      <CountBadge v-if="messageCount(draft) > 1" :value="messageCount(draft)" />
                    </span>

                    <span class="row__preview-line">
                      <span class="row__snippet" :class="{ 'row__snippet--empty': !snippet(draft) }">
                        {{ snippet(draft) || $t('drafts.noContent') }}
                      </span>
                      <svg
                        v-if="hasAttachments(draft)"
                        class="row__attachment"
                        width="16"
                        height="16"
                        viewBox="0 0 16 16"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.5"
                        stroke-linecap="round"
                        :aria-label="$t('drafts.withAttachment')"
                      >
                        <path d="M14 8.5 8 14.5a3.5 3.5 0 0 1-5-5L9.5 3A2.5 2.5 0 0 1 13 6.5L6.5 13a1.5 1.5 0 0 1-2-2.1L11 4.5" />
                      </svg>
                    </span>
                  </span>
                </CardButton>

                <div class="row__actions">
                  <IconButton
                    size="sm"
                    :title="$t('drafts.editDraft')"
                    :disabled="draftActions.isBusy(draft.id)"
                    @click="composer.openDraft(draft.id)"
                  >
                    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M12 1.5L14.5 4L5.5 13H3V10.5L12 1.5Z" />
                    </svg>
                  </IconButton>
                  <IconButton
                    size="sm"
                    tone="danger"
                    :title="$t('drafts.deleteDraft')"
                    :disabled="draftActions.isBusy(draft.id)"
                    @click="void discardDraft(draft.id)"
                  >
                    <svg
                      width="14"
                      height="14"
                      viewBox="0 0 16 16"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.5"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    >
                      <path d="M2.5 4h11M5.5 4V2.8A1.3 1.3 0 0 1 6.8 1.5h2.4a1.3 1.3 0 0 1 1.3 1.3V4" />
                      <path d="M12.6 4v9a1.5 1.5 0 0 1-1.5 1.5h-6.2A1.5 1.5 0 0 1 3.4 13V4" />
                    </svg>
                  </IconButton>
                </div>
              </li>
            </ul>
          </section>
        </div>
      </div>
    </div>
    </template>

    <template #detail>
      <ThreadView
        v-if="selectedDraft?.thread_id && selectedThread"
        :thread-id="selectedDraft.thread_id"
        collapse-to-latest
        :show-back="ui.isSinglePane"
        @back="closeDetail"
      />
      <article v-else-if="selectedDraft" class="preview">
        <header class="preview__header">
          <IconButton v-if="ui.isSinglePane" :title="$t('common.back')" @click="closeDetail">
            <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="m10.5 3-5 5 5 5" />
            </svg>
          </IconButton>
          <div class="preview__copy">
            <h2 class="preview__subject">{{ selectedDraft.subject || $t('compose.noSubject') }}</h2>
            <p v-if="selectedDraft.to.length > 0" class="preview__line">{{ $t('common.to') }} {{ selectedDraft.to.join(', ') }}</p>
            <p v-if="selectedDraft.cc.length > 0" class="preview__line">{{ $t('common.cc') }} {{ selectedDraft.cc.join(', ') }}</p>
            <p v-if="selectedDraft.bcc.length > 0" class="preview__line">{{ $t('common.bcc') }} {{ selectedDraft.bcc.join(', ') }}</p>
            <p v-if="selectedDraft.scheduled_for" class="preview__line preview__line--scheduled">
              {{ $t('drafts.goesOutAutomatically', { time: exactTime(selectedDraft.scheduled_for) }) }}
            </p>
            <p v-else class="preview__line">{{ $t('drafts.lastSaved', { time: exactTime(selectedDraft.updated_at) }) }}</p>
          </div>
          <div class="preview__actions">
            <AppButton
              variant="primary"
              size="sm"
              :disabled="draftActions.isBusy(selectedDraft.id)"
              @click="composer.openDraft(selectedDraft.id)"
            >
              {{ $t('common.edit') }}
            </AppButton>
            <AppButton
              variant="ghost"
              size="sm"
              :disabled="draftActions.isBusy(selectedDraft.id)"
              @click="void discardDraft(selectedDraft.id)"
            >
              {{ $t('common.delete') }}
            </AppButton>
          </div>
        </header>

        <div class="preview__body">
          <p v-if="previewText(selectedDraft)" class="preview__text">{{ previewText(selectedDraft) }}</p>
          <p v-else class="preview__empty">{{ $t('drafts.noContent') }}</p>

          <ul v-if="selectedDraft.attachments?.length" class="preview__attachments">
            <li v-for="attachment in selectedDraft.attachments" :key="attachment.filename">
              <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M13.2 7.6L8.3 12.5C6.9 13.9 4.7 13.9 3.3 12.5C1.9 11.1 1.9 8.9 3.3 7.5L8.7 2.1C9.6 1.2 11.1 1.2 12 2.1C12.9 3 12.9 4.5 12 5.4L6.9 10.5C6.4 11 5.7 11 5.2 10.5C4.7 10 4.7 9.3 5.2 8.8L9.8 4.2" />
              </svg>
              {{ attachment.filename }}
            </li>
          </ul>
        </div>
      </article>
      <div v-else class="detail-placeholder">
        <EmptyState
          icon="✉️"
          :title="$t('drafts.detail.emptyTitle')"
          :description="$t('drafts.detail.emptyDescription')"
        />
      </div>
    </template>
  </ListDetailLayout>
</template>

<style scoped>
.drafts {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  min-width: 0;
}

.drafts__body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 4px var(--space-md) var(--space-xl);
}

.groups {
  display: flex;
  flex-direction: column;
  gap: var(--space-lg);
}

.group {
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
}

.list {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
  list-style: none;
  margin: 0;
  padding: 0;
}

.row {
  position: relative;
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  border: var(--border-muted);
  transition:
    background var(--duration-fast) var(--ease-out),
    opacity var(--duration-fast) var(--ease-out);
}

.row:hover {
  background: var(--bg-hover);
}

.row--busy {
  opacity: 0.55;
  pointer-events: none;
}

/* Scheduled drafts leave on their own — the accent marks the deadline. */
.row--scheduled {
  border-color: color-mix(in srgb, var(--accent-primary) 28%, transparent);
}

.row--active {
  border-color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 8%, var(--bg-surface));
}

.row__open {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  width: 100%;
  min-width: 0;
  min-height: 88px;
  padding: 10px 12px;
  border: 0;
  background: none;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.row__draft-avatar {
  position: relative;
  display: inline-flex;
  width: 40px;
  height: 40px;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  border: 1px solid color-mix(in srgb, var(--text-tertiary) 72%, transparent);
  border-radius: 50%;
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--bg-overlay) 56%, transparent);
  overflow: hidden;
}

.row__draft-avatar--with-participant {
  border-color: transparent;
  color: var(--text-on-accent);
  background: var(--bg-overlay);
}

.row__avatar-image {
  filter: brightness(0.42) saturate(0.8);
}

.row__draft-pencil {
  position: absolute;
  z-index: 1;
  inset: 10px;
  color: currentColor;
}

.row__draft-avatar--with-participant .row__draft-pencil {
  color: var(--text-on-accent);
  filter: drop-shadow(0 1px 2px rgb(0 0 0 / 0.7));
}

.row__content {
  display: flex;
  min-width: 0;
  flex-direction: column;
  justify-content: center;
}

.row__headline {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: start;
  gap: var(--space-sm);
  min-width: 0;
}

.row__participants {
  min-width: 0;
  font-size: var(--font-size-md);
  font-weight: var(--weight-strong);
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__time {
  flex-shrink: 0;
}

.row__time--scheduled {
  color: var(--accent-primary);
}

.row__subject-line,
.row__preview-line {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 7px;
}

.row__subject-line {
  margin-top: 2px;
}

.row__subject {
  min-width: 0;
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
  color: color-mix(in srgb, var(--text-primary) 78%, var(--text-secondary));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__preview-line {
  margin-top: 4px;
}

.row__snippet {
  flex: 1;
  min-width: 0;
  font-size: var(--font-size-xs);
  line-height: 1.5;
  color: color-mix(in srgb, var(--text-secondary) 55%, var(--text-tertiary));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__snippet--empty {
  font-style: italic;
}

.row__attachment {
  flex-shrink: 0;
  color: var(--text-tertiary);
  opacity: 0.7;
}

.row__actions {
  position: absolute;
  z-index: 2;
  top: 8px;
  right: 8px;
  display: flex;
  align-items: center;
  gap: var(--space-2xs);
  flex-shrink: 0;
  padding-left: 12px;
  background: linear-gradient(90deg, transparent, var(--bg-surface) 18%);
  opacity: 0;
  pointer-events: none;
  transition:
    opacity var(--duration-fast) var(--ease-out),
    background var(--duration-fast) var(--ease-out);
}

.row:hover .row__actions,
.row:focus-within .row__actions {
  opacity: 1;
  pointer-events: auto;
  background: linear-gradient(90deg, transparent, var(--bg-hover) 18%);
}

.row:hover .row__time,
.row:focus-within .row__time {
  opacity: 0;
}

/* ── Detail pane ── */
.preview {
  height: 100%;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  overflow: hidden;
}

.preview__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-md);
  padding: var(--space-lg) var(--space-xl) var(--space-md);
  border-bottom: 1px solid var(--border-color-muted);
}

.preview__copy {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 3px;
}

.preview__subject {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.preview__line {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  overflow-wrap: anywhere;
}

.preview__line--scheduled {
  color: var(--accent-primary);
}

.preview__actions {
  display: flex;
  gap: var(--space-xs);
  flex-shrink: 0;
}

.preview__body {
  overflow-y: auto;
  padding: var(--space-lg) var(--space-xl) var(--space-xl);
}

.preview__text {
  margin: 0;
  max-width: 720px;
  font-size: var(--font-size-md);
  line-height: 1.6;
  color: var(--text-primary);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.preview__empty {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

.preview__attachments {
  margin: var(--space-lg) 0 0;
  padding: 0;
  list-style: none;
  display: grid;
  gap: var(--space-2xs);
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.detail-placeholder {
  height: 100%;
  display: grid;
  place-items: center;
}
</style>
