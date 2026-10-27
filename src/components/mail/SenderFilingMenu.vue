<script setup lang="ts">
/**
 * Everything that files a *sender* rather than a thread: where their mail
 * lands and whether it counts as priority. One component for both entry
 * points — the row's right-click flyout and the avatar in the reading pane —
 * so the two can never disagree about what state the sender is in.
 *
 * State comes from the backend on open. Offering "priorisieren" *and*
 * "Priorität entfernen" side by side, as the old menu did, is what you fall
 * back to when the current value is unknown.
 */
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import type { ThreadCategory } from '@/types/mail';
import { useListSearchStore } from '@/stores/listSearch';
import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import { type InboxSenderBucket } from '@/stores/settings';
import { useThreadActions } from '@/composables/useThreadActions';
import MenuItem from '@/components/ui/MenuItem.vue';

const props = defineProps<{
  accountId: string;
  email: string;
}>();

const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const router = useRouter();
const listSearch = useListSearchStore();
const settings = useSettingsStore();
const threadsStore = useThreadsStore();
const actions = useThreadActions();

const normalizedEmail = computed(() => props.email.trim().toLowerCase());
const isPriority = ref(false);
const category = ref<ThreadCategory | null>(null);

const BUCKETS = computed<
  { category: ThreadCategory; bucket: InboxSenderBucket; label: string; icon: 'person' | 'bell' | 'newsletter' }[]
>(() => [
  { category: 'personal', bucket: 'personal', label: t('mail.senderFiling.personal'), icon: 'person' },
  { category: 'updates', bucket: 'notifications', label: t('mail.senderFiling.notifications'), icon: 'bell' },
  { category: 'promotions', bucket: 'newsletter', label: t('mail.senderFiling.newsletter'), icon: 'newsletter' },
]);

/** Local mirror while the backend answer is in flight — and the only source
 *  in web mode, where the command does not exist. */
function localFallback(): ThreadCategory | null {
  switch (settings.inboxSenderBuckets[normalizedEmail.value]) {
    case 'personal': return 'personal';
    case 'notifications': return 'updates';
    case 'newsletter': return 'promotions';
    case 'social': return 'social';
    default: return null;
  }
}

let preferenceRequest = 0;
watch(
  normalizedEmail,
  async (email) => {
    const request = ++preferenceRequest;
    if (!email) return;
    category.value = localFallback();
    isPriority.value = false;
    try {
      const preference = await threadsStore.getSenderPreference(email);
      if (request !== preferenceRequest) return;
      isPriority.value = preference.is_priority;
      category.value = preference.category;
    } catch {
      // Web mode: the local bucket mirror stays.
    }
  },
  { immediate: true },
);

async function togglePriority() {
  const next = !isPriority.value;
  if (await actions.attempt(() => threadsStore.setSenderPriorityByEmail(props.accountId, normalizedEmail.value, next))) {
    isPriority.value = next;
    emit('close');
  }
}

async function choose(entry: (typeof BUCKETS)['value'][number]) {
  if (await actions.attempt(() => threadsStore.setSenderCategoryByEmail(
    props.accountId, normalizedEmail.value, entry.category, entry.bucket,
  ))) {
    category.value = entry.category;
    emit('close');
  }
}

/** The sender's mail in every mailbox, as the list search — like a sidebar contact. */
function showAll() {
  emit('close');
  listSearch.open(`from:${normalizedEmail.value}`, 'all');
  void router.push({ name: 'inbox' });
}
</script>

<template>
  <div class="menu-group">
    <MenuItem
      icon="bolt"
      :label="$t('mail.senderFiling.alwaysPrioritize')"
      :checked="isPriority"
      @select="togglePriority"
    />
  </div>

  <div class="menu-group">
    <MenuItem
      v-for="entry in BUCKETS"
      :key="entry.category"
      :icon="entry.icon"
      :label="entry.label"
      :checked="category === entry.category"
      @select="choose(entry)"
    />
  </div>

  <div class="menu-group">
    <MenuItem icon="search" :label="$t('mail.senderFiling.showAllMail')" @select="showAll" />
  </div>
</template>
