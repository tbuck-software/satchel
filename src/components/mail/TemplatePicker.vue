<script setup lang="ts">
import CollectionLoadState from '@/components/ui/CollectionLoadState.vue';
import AppButton from '@/components/ui/AppButton.vue';
import CardButton from '@/components/ui/CardButton.vue';
import { useDismissable } from '@/composables/useDismissable';
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useTemplatesStore } from '@/stores/templates';
import { useToast } from '@/composables/useToast';
import { translateBackendError } from '@/utils/backendErrors';
import ModalShell from '@/components/ui/ModalShell.vue';

const emit = defineEmits<{
  (e: 'apply', value: {
    subject?: string | null;
    body_text: string;
    body_html?: string | null;
    cc?: string[];
    bcc?: string[];
    attachments_json?: string | null;
  }): void;
  (e: 'close'): void;
}>();

const { t } = useI18n();
const templatesStore = useTemplatesStore();
const toast = useToast();
const filter = ref('');
const variables = ref<Record<string, string>>({
  recipient_name: '',
  my_name: '',
});

const recipientPlaceholder = '{{recipient_name}}';
const myNamePlaceholder = '{{my_name}}';

onMounted(() => {
  templatesStore.load();
});

const filtered = computed(() => {
  const lower = filter.value.toLowerCase();
  return templatesStore.items.filter((t) =>
    !lower || t.name.toLowerCase().includes(lower) || t.shortcut?.toLowerCase().includes(lower),
  );
});

async function pick(id: string) {
  try {
    const rendered = await templatesStore.render(id, variables.value);
    emit('apply', rendered);
  } catch (err) {
    console.warn('template render failed', err);
    toast.error(t('compose.templates.renderFailed', {
      error: translateBackendError(err) ?? (err instanceof Error ? err.message : String(err)),
    }));
  }
}

// Back closes this before whatever it was opened from.
useDismissable(() => true, () => emit('close'));
</script>

<template>
  <ModalShell
    :label="$t('compose.templates.title')"
    width="560px"
    height="auto"
    @dismiss="emit('close')"
  >
    <div class="picker-body">
      <h3>{{ $t('compose.templates.title') }}</h3>
      <input v-model="filter" :placeholder="$t('compose.templates.searchPlaceholder')" autofocus />
      <div class="vars">
        <label>
          <span>{{ recipientPlaceholder }}</span>
          <input v-model="variables.recipient_name" />
        </label>
        <label>
          <span>{{ myNamePlaceholder }}</span>
          <input v-model="variables.my_name" />
        </label>
      </div>
      <CollectionLoadState
        :loading="templatesStore.loading"
        :failed="templatesStore.loadFailed"
        :has-content="templatesStore.items.length > 0"
        :error-label="$t('settings.templates.loadFailed')"
        @retry="templatesStore.load()"
      />
      <ul class="list" :aria-busy="templatesStore.loading">
        <li v-for="tpl in filtered" :key="tpl.id">
          <CardButton @click="pick(tpl.id)">
            <span class="name">{{ tpl.name }}</span>
            <span v-if="tpl.shortcut" class="shortcut">{{ tpl.shortcut }}</span>
            <span class="preview">{{ tpl.body_text.slice(0, 80) }}</span>
          </CardButton>
        </li>
        <li v-if="filtered.length === 0 && !templatesStore.loading && !templatesStore.loadFailed" class="empty">
          {{ $t('compose.templates.empty') }}
        </li>
      </ul>
      <div class="actions">
        <AppButton class="ghost" @click="emit('close')" variant="ghost" size="sm">{{ $t('common.close') }}</AppButton>
      </div>
    </div>
  </ModalShell>
</template>

<style scoped>
.picker-body {
  display: grid;
  gap: var(--space-md);
  align-content: start;
  min-height: 0;
  padding: var(--space-lg);
  overflow: auto;
}

h3 {
  margin: 0;
  color: var(--text-primary);
}

input {
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  padding: 8px 10px;
  color: var(--text-primary);
  font-family: inherit;
}

.vars {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}

.vars label {
  display: grid;
  gap: 3px;
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.list button {
  width: 100%;
  text-align: left;
  background: var(--bg-raised);
  border: 1px solid var(--border-color);
  padding: 8px 12px;
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  cursor: pointer;
  display: grid;
  gap: 4px;
}

.list button:hover {
  background: var(--bg-hover);
}

.shortcut {
  font-family: var(--font-mono);
  background: color-mix(in srgb, var(--accent-primary) 15%, transparent);
  padding: 1px 6px;
  border-radius: var(--radius-xs);
  align-self: start;
  font-size: var(--font-size-xs);
}

.preview {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.empty {
  color: var(--text-tertiary);
  font-style: italic;
  font-size: var(--font-size-sm);
}

.actions {
  display: flex;
  justify-content: flex-end;
}
</style>
