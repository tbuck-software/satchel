<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useAliasesStore } from '@/stores/aliases';
import { useCrudActions } from '@/composables/useCrudActions';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

const { t } = useI18n();
const accountsStore = useAccountsStore();
const aliasesStore = useAliasesStore();
const crud = useCrudActions();

const accountId = ref<string>('');
const editingId = ref<string | null>(null);
const email = ref('');
const displayName = ref('');
const isDefault = ref(false);
const status = ref('');

onMounted(async () => {
  await Promise.all([accountsStore.load(), aliasesStore.load()]);
  accountId.value = accountsStore.activeAccountId ?? accountsStore.accounts[0]?.id ?? '';
});

const accountAliases = computed(() =>
  accountId.value ? aliasesStore.aliasesFor(accountId.value) : [],
);

function reset() {
  editingId.value = null;
  email.value = '';
  displayName.value = '';
  isDefault.value = false;
}

function edit(id: string) {
  const alias = aliasesStore.items.find((a) => a.id === id);
  if (!alias) return;
  editingId.value = id;
  email.value = alias.email;
  displayName.value = alias.display_name ?? '';
  isDefault.value = alias.is_default === 1;
}

async function save() {
  if (!accountId.value || !email.value.trim().includes('@')) {
    status.value = t('settings.aliases.invalidEmail');
    return;
  }
  try {
    await aliasesStore.upsert({
      id: editingId.value,
      accountId: accountId.value,
      email: email.value.trim(),
      displayName: displayName.value.trim() || null,
      isDefault: isDefault.value,
    });
    status.value = t('settings.aliases.saved');
    reset();
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  }
}

async function remove(id: string) {
  const alias = aliasesStore.items.find((a) => a.id === id);
  if (!alias) return;
  const removed = await crud.removeWithUndo({
    id,
    name: alias.email,
    remove: () => aliasesStore.remove(id),
    restore: (snapshot) => aliasesStore.restore(snapshot),
  });
  if (removed && editingId.value === id) reset();
}
</script>

<template>
  <section>
    <h3>{{ $t('settings.aliases.title') }}</h3>
    <p class="hint">{{ $t('settings.aliases.hint') }}</p>

    <label class="field">
      <span>{{ $t('settings.aliases.account') }}</span>
      <select v-model="accountId">
        <option v-for="acc in accountsStore.accounts" :key="acc.id" :value="acc.id">
          {{ acc.displayName }} &lt;{{ acc.email }}&gt;
        </option>
      </select>
    </label>

    <ul v-if="accountAliases.length > 0" class="list">
      <li v-for="alias in accountAliases" :key="alias.id">
        <button type="button" class="row" @click="edit(alias.id)">
          <span>
            <strong>{{ alias.email }}</strong>
            <span v-if="alias.display_name"> · {{ alias.display_name }}</span>
            <span v-if="alias.is_default === 1" class="badge">{{ $t('common.default') }}</span>
          </span>
        </button>
        <button type="button" class="delete" @click="remove(alias.id)">{{ $t('common.remove') }}</button>
      </li>
    </ul>
    <p v-else class="empty">{{ $t('settings.aliases.empty') }}</p>

    <div class="editor">
      <h4>{{ editingId ? $t('settings.aliases.editTitle') : $t('settings.aliases.addTitle') }}</h4>
      <label class="field">
        <span>{{ $t('common.email') }}</span>
        <input v-model="email" type="email" :placeholder="$t('settings.aliases.emailPlaceholder')" />
      </label>
      <label class="field">
        <span>{{ $t('settings.aliases.displayName') }}</span>
        <input v-model="displayName" :placeholder="$t('settings.aliases.displayNamePlaceholder')" />
      </label>
      <ToggleSwitch v-model="isDefault">{{ $t('settings.aliases.useAsDefault') }}</ToggleSwitch>

      <div class="actions">
        <button v-if="editingId" type="button" class="ghost" @click="reset">{{ $t('common.cancel') }}</button>
        <button type="button" class="primary" @click="save">{{ $t('common.save') }}</button>
      </div>
      <p v-if="status" class="status">{{ status }}</p>
    </div>
  </section>
</template>

<style scoped>
section {
  display: grid;
  gap: 14px;
}

h3,
h4 {
  margin: 0;
  color: var(--text-primary);
}

.hint {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

.field {
  display: grid;
  gap: 4px;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.field input,
.field select {
  border: 1px solid var(--border-color);
  background: var(--bg-raised);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  color: var(--text-primary);
  font-family: inherit;
}

.list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.list li {
  display: flex;
  gap: 6px;
}

.row {
  flex: 1;
  background: var(--bg-raised);
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  padding: 8px 12px;
  cursor: pointer;
  text-align: left;
  color: var(--text-primary);
}
.row:hover {
  background: var(--bg-hover);
}

.badge {
  background: color-mix(in srgb, var(--accent-primary) 22%, transparent);
  color: var(--accent-primary);
  font-size: var(--font-size-xs);
  padding: 1px 8px;
  border-radius: var(--radius-full);
  margin-left: 8px;
}

.delete {
  background: transparent;
  border: 1px solid var(--border-color-muted);
  color: var(--text-secondary);
  border-radius: var(--radius-sm);
  padding: 0 12px;
  font-size: var(--font-size-xs);
  cursor: pointer;
}
.delete:hover {
  border-color: var(--accent-red);
  color: var(--accent-red);
}

.empty {
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
  margin: 0;
}

.editor {
  display: grid;
  gap: 10px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  padding: 14px;
  background: var(--bg-base);
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.primary {
  background: var(--accent-primary);
  color: var(--text-on-accent);
  border: 0;
  padding: 8px 16px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-weight: var(--weight-semibold);
}

.ghost {
  background: transparent;
  border: 1px solid var(--border-color);
  color: var(--text-primary);
  padding: 8px 16px;
  border-radius: var(--radius-sm);
  cursor: pointer;
}

.status {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}
</style>
