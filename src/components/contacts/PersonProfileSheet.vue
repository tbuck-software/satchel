<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import { computed, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import type { Participant, Thread } from '@/types/mail';
import { backendBlockSender } from '@/services/backend';
import { useAccountsStore } from '@/stores/accounts';
import { useContactsStore } from '@/stores/contacts';
import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import { useToast } from '@/composables/useToast';
import { transportErrorLabel } from '@/utils/transportLabels';
import { formatListTime } from '@/utils/listTime';
import Avatar from '@/components/common/Avatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import { useI18n } from 'vue-i18n';

const props = defineProps<{
  open: boolean;
  person: Participant;
}>();

const emit = defineEmits<{
  dismiss: [];
  navigate: [];
}>();

const router = useRouter();
const { t } = useI18n();
const accounts = useAccountsStore();
const contacts = useContactsStore();
const settings = useSettingsStore();
const threads = useThreadsStore();
const toast = useToast();
const notes = ref('');
const saving = ref(false);
const saveError = ref('');

const normalizedEmail = computed(() => props.person.email.trim().toLowerCase());
const contact = computed(() => contacts.findByEmail(normalizedEmail.value));
const displayName = computed(() =>
  contact.value?.display_name?.trim() || props.person.name.trim() || props.person.email,
);

function includesPerson(thread: Thread): boolean {
  return [...thread.participants, ...(thread.sentParticipants ?? [])].some(
    (participant) => participant.email.trim().toLowerCase() === normalizedEmail.value,
  );
}

const matchingThreads = computed(() => threads.threads
  .filter(includesPerson)
  .sort((left, right) =>
    new Date(right.lastMessageAt).getTime() - new Date(left.lastMessageAt).getTime(),
  ));
const recentThreads = computed(() => matchingThreads.value.slice(0, 5));

const messageCount = computed(() => matchingThreads.value.reduce(
  (sum, thread) => sum + thread.messageCount,
  0,
));
const accountNames = computed(() => {
  const ids = new Set(recentThreads.value.map((thread) => thread.accountId));
  return accounts.accounts
    .filter((account) => ids.has(account.id))
    .map((account) => account.displayName || account.email);
});
const pinned = computed(() => settings.pinnedContacts.includes(normalizedEmail.value));

watch(
  () => [props.open, normalizedEmail.value, contact.value?.notes] as const,
  ([open]) => {
    if (!open) return;
    notes.value = contact.value?.notes ?? '';
    saveError.value = '';
  },
  { immediate: true },
);

async function saveNotes() {
  if (saving.value) return;
  saving.value = true;
  saveError.value = '';
  try {
    await contacts.saveProfile({
      email: props.person.email,
      displayName: displayName.value,
      notes: notes.value,
    });
  } catch {
    saveError.value = t('contacts.profile.noteSaveFailed');
  } finally {
    saving.value = false;
  }
}

function showAllMail() {
  emit('navigate');
  void router.push({
    name: 'search',
    query: { q: props.person.email, global: '1' },
  });
}

function openThread(threadId: string) {
  emit('navigate');
  void router.push({ name: 'thread', params: { id: threadId } });
}

async function deletePerson() {
  if (!contact.value) return;
  if (!window.confirm(t('contacts.profile.deleteConfirm', { email: props.person.email }))) return;
  try {
    await contacts.remove(props.person.email);
    emit('dismiss');
  } catch (error) {
    toast.error(t('contacts.profile.deleteFailed', { error: transportErrorLabel(error) }));
  }
}

async function blockPerson() {
  if (!window.confirm(t('contacts.profile.blockConfirm', { email: props.person.email }))) return;
  await backendBlockSender({ pattern: props.person.email });
  emit('dismiss');
}
</script>

<template>
  <Teleport to="body">
    <BottomSheet :label="$t('contacts.profile.label')" :open="props.open" @dismiss="emit('dismiss')">
      <article class="person-profile">
        <header class="person-profile__header">
          <IconButton :title="$t('common.back')" size="lg" @click="emit('dismiss')">
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="m10.5 3-5 5 5 5" />
            </svg>
          </IconButton>
          <Avatar :name="displayName" :email="props.person.email" :size="58" />
          <div class="person-profile__identity">
            <h2>{{ displayName }}</h2>
            <p>{{ props.person.email }}</p>
          </div>
        </header>

        <div class="person-profile__facts">
          <span>{{ $t('contacts.meta.conversations', matchingThreads.length) }}</span>
          <span>{{ $t('contacts.profile.messages', messageCount) }}</span>
          <span v-if="accountNames.length">{{ accountNames.join(', ') }}</span>
        </div>

        <div class="person-profile__actions">
          <AppButton variant="soft" @click="showAllMail">{{ $t('contacts.profile.allMail') }}</AppButton>
          <AppButton variant="ghost" @click="settings.togglePinnedContact(props.person.email)">
            {{ pinned ? $t('contacts.profile.unpin') : $t('contacts.profile.pin') }}
          </AppButton>
          <AppButton variant="danger" @click="void blockPerson()">{{ $t('common.block') }}</AppButton>
          <AppButton v-if="contact" variant="danger" @click="void deletePerson()">
            {{ $t('contacts.profile.delete') }}
          </AppButton>
        </div>

        <section class="person-profile__notes">
          <div>
            <h3>{{ $t('contacts.profile.notesHeading') }}</h3>
            <span>{{ $t('contacts.profile.notesHint') }}</span>
          </div>
          <textarea
            v-model="notes"
            rows="4"
            :placeholder="$t('contacts.profile.notesPlaceholder')"
          />
          <div class="person-profile__save-row">
            <span v-if="saveError" role="alert">{{ saveError }}</span>
            <AppButton variant="primary" size="sm" :disabled="saving" @click="void saveNotes()">
              {{ saving ? $t('contacts.profile.savingNote') : $t('contacts.profile.saveNote') }}
            </AppButton>
          </div>
        </section>

        <section v-if="recentThreads.length" class="person-profile__recent">
          <h3>{{ $t('contacts.profile.recentMail') }}</h3>
          <CardButton
            v-for="thread in recentThreads"
            :key="thread.id"
            @click="openThread(thread.id)"
          >
            <span>
              <strong>{{ (thread.normalizedSubject ?? thread.subject) || $t('contacts.profile.noSubject') }}</strong>
              <small>{{ thread.snippet }}</small>
            </span>
            <time :datetime="thread.lastMessageAt">{{ formatListTime(thread.lastMessageAt) }}</time>
          </CardButton>
        </section>
      </article>
    </BottomSheet>
  </Teleport>
</template>

<style scoped>
.person-profile {
  width: min(460px, 100vw);
  padding: 8px 18px 30px;
}

.person-profile__header {
  display: grid;
  grid-template-columns: 38px 58px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  padding: 8px 0 18px;
  border-bottom: 1px solid var(--border-color-muted);
}

.person-profile__header svg { width: 20px; height: 20px; }
.person-profile__identity { min-width: 0; }
.person-profile__identity h2 { margin: 0; color: var(--text-primary); font-size: var(--font-size-xl); }
.person-profile__identity p { margin: 3px 0 0; overflow: hidden; color: var(--text-tertiary); font-size: var(--font-size-sm); text-overflow: ellipsis; white-space: nowrap; }

.person-profile__facts {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
  padding: 14px 0;
}

.person-profile__facts span {
  padding: 5px 9px;
  border-radius: var(--radius-full);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.person-profile__actions { display: flex; flex-wrap: wrap; gap: 7px; }

.person-profile__notes,
.person-profile__recent {
  margin-top: 24px;
}

.person-profile__notes > div:first-child {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.person-profile h3 { margin: 0 0 9px; color: var(--text-primary); font-size: var(--font-size-md); }
.person-profile__notes > div:first-child span { color: var(--text-tertiary); font-size: var(--font-size-xs); }

.person-profile textarea {
  box-sizing: border-box;
  width: 100%;
  resize: vertical;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  padding: 11px 12px;
  background: var(--bg-base);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
  line-height: 1.5;
}

.person-profile textarea:focus {
  border-color: var(--accent-primary);
}

.person-profile__save-row { display: flex; align-items: center; justify-content: flex-end; gap: 10px; margin-top: 8px; }
.person-profile__save-row span { margin-right: auto; color: var(--accent-red); font-size: var(--font-size-xs); }

.person-profile__recent { display: grid; gap: 0; }
.person-profile__recent > button {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 12px;
  width: 100%;
  padding: 12px 0;
  border: 0;
  border-bottom: 1px solid var(--border-color-muted);
  background: transparent;
  color: inherit;
  text-align: left;
}
.person-profile__recent > button > span { min-width: 0; }
.person-profile__recent strong,
.person-profile__recent small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.person-profile__recent strong { color: var(--text-primary); font-size: var(--font-size-sm); }
.person-profile__recent small,
.person-profile__recent time { margin-top: 3px; color: var(--text-tertiary); font-size: var(--font-size-xs); }

@media (max-width: 767px) {
  .person-profile { width: 100%; }
}
</style>
