<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import CardButton from '@/components/ui/CardButton.vue';
import { computed, nextTick, onMounted, ref } from 'vue';
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
const editorOpen = ref(false);
const editorRef = ref<HTMLElement | null>(null);
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

function startNew() {
  reset();
  editorOpen.value = true;
  void nextTick(() => editorRef.value?.scrollIntoView({ behavior: 'smooth', block: 'nearest' }));
}

function reset() {
  editorOpen.value = false;
  editingId.value = null;
  email.value = '';
  displayName.value = '';
  isDefault.value = false;
}

function edit(id: string) {
  editorOpen.value = true;
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
    <h3 class="settings-page-title">{{ $t('settings.sections.aliases.label') }}</h3>
    <p class="hint">{{ $t('settings.aliases.hint') }}</p>

    <div class="setting-group">
      <label class="setting-row">
        <span class="setting-label">{{ $t('settings.aliases.account') }}</span>
        <select v-model="accountId">
          <option v-for="acc in accountsStore.accounts" :key="acc.id" :value="acc.id">
            {{ acc.displayName }} &lt;{{ acc.email }}&gt;
          </option>
        </select>
      </label>
    </div>

    <!-- A list with an Add row; the form opens on demand. -->
    <div class="setting-group list">
      <div v-for="alias in accountAliases" :key="alias.id" class="setting-row collection-row">
          <CardButton class="row" @click="edit(alias.id)">
            <span>
              <strong>{{ alias.email }}</strong>
              <span v-if="alias.display_name"> · {{ alias.display_name }}</span>
              <span v-if="alias.is_default === 1" class="badge">{{ $t('common.default') }}</span>
            </span>
          </CardButton>
          <AppButton class="delete" @click="remove(alias.id)" variant="ghost" size="sm">{{ $t('common.remove') }}</AppButton>
      </div>
      <div v-if="!(accountAliases.length > 0)" class="setting-row setting-row--single">
        <span class="setting-hint">{{ $t('settings.aliases.empty') }}</span>
      </div>
      <div v-if="!editorOpen" class="setting-row setting-row--single">
        <AppButton variant="secondary" size="sm" @click="startNew">
          <template #icon><AppIcon><path d="M8 3v10M3 8h10" /></AppIcon></template>
          {{ $t('settings.aliases.addTitle') }}
        </AppButton>
      </div>
    </div>

    <div v-if="editorOpen" ref="editorRef" class="editor">
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
        <AppButton class="ghost" @click="reset" variant="ghost" size="sm">{{ $t('common.cancel') }}</AppButton>
        <AppButton class="primary" @click="save" variant="primary" size="sm">{{ $t('common.save') }}</AppButton>
      </div>
    </div>
    <p v-if="status" class="status" role="status">{{ status }}</p>
  </section>
</template>

<style scoped src="./settingsForms.css"></style>

<style scoped src="./settingsRows.css"></style>
