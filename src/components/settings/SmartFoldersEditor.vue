<script setup lang="ts">
import CollectionLoadState from '@/components/ui/CollectionLoadState.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import CardButton from '@/components/ui/CardButton.vue';
import { nextTick, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useSmartFoldersStore } from '@/stores/smartFolders';
import { useCrudActions } from '@/composables/useCrudActions';

const { t } = useI18n();
const smart = useSmartFoldersStore();
const crud = useCrudActions();
const editingId = ref<string | null>(null);
const editorOpen = ref(false);
const editorRef = ref<HTMLElement | null>(null);
const name = ref('');
const query = ref('');
const icon = ref('');
const status = ref('');

onMounted(() => smart.load());

function startNew() {
  reset();
  editorOpen.value = true;
  void nextTick(() => editorRef.value?.scrollIntoView({ behavior: 'smooth', block: 'nearest' }));
}

function reset() {
  editorOpen.value = false;
  editingId.value = null;
  name.value = '';
  query.value = '';
  icon.value = '';
  status.value = '';
}

function edit(id: string) {
  editorOpen.value = true;
  const f = smart.items.find((s) => s.id === id);
  if (!f) return;
  editingId.value = id;
  name.value = f.name;
  query.value = f.query;
  icon.value = f.icon ?? '';
}

async function save() {
  if (!name.value.trim() || !query.value.trim()) {
    status.value = t('settings.smartFolders.validation');
    return;
  }
  try {
    await smart.upsert({
      id: editingId.value,
      name: name.value.trim(),
      query: query.value.trim(),
      icon: icon.value.trim() || null,
    });
    reset();
    status.value = t('settings.smartFolders.saved');
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  }
}

async function remove(id: string) {
  const folder = smart.items.find((f) => f.id === id);
  if (!folder) return;
  const removed = await crud.removeWithUndo({
    id,
    name: folder.name,
    remove: () => smart.remove(id),
    restore: (snapshot) => smart.restore(snapshot),
  });
  if (removed && editingId.value === id) reset();
}
</script>

<template>
  <section>
    <h3 class="settings-page-title">{{ $t('settings.sections.smart-folders.label') }}</h3>
    <p>{{ $t('settings.smartFolders.purpose') }}</p>

    <CollectionLoadState
      :loading="smart.loading"
      :failed="smart.loadFailed"
      :has-content="smart.items.length > 0"
      :error-label="$t('settings.smartFolders.loadFailed')"
      @retry="smart.load()"
    />
    <!-- A list with an Add row; the form opens on demand. -->
    <div class="setting-group list" :aria-busy="smart.loading">
      <div v-for="f in smart.items" :key="f.id" class="setting-row collection-row">
          <CardButton class="row" @click="edit(f.id)">
            <span class="ico">{{ f.icon || '🔍' }}</span>
            <span>
              <strong>{{ f.name }}</strong>
              <code class="q">{{ f.query }}</code>
            </span>
          </CardButton>
          <AppButton class="delete" @click="remove(f.id)" variant="ghost" size="sm">{{ $t('common.delete') }}</AppButton>
      </div>
      <div v-if="!(smart.items.length > 0) && !smart.loading && !smart.loadFailed" class="setting-row setting-row--single">
        <span class="setting-hint">{{ $t('settings.smartFolders.empty') }}</span>
      </div>
      <div v-if="!editorOpen" class="setting-row setting-row--single">
        <AppButton variant="secondary" size="sm" @click="startNew">
          <template #icon><AppIcon><path d="M8 3v10M3 8h10" /></AppIcon></template>
          {{ $t('settings.smartFolders.newTitle') }}
        </AppButton>
      </div>
    </div>

    <div v-if="editorOpen" ref="editorRef" class="editor">
      <h4>{{ editingId ? $t('settings.smartFolders.editTitle') : $t('settings.smartFolders.newTitle') }}</h4>
      <p class="hint">
        {{ $t('settings.smartFolders.hintLead') }}
        <code>{{ $t('settings.smartFolders.syntaxExample') }}</code>
        {{ $t('settings.smartFolders.hintMiddle') }}
        <code>{{ $t('settings.smartFolders.syntaxFrom') }}</code>,
        <code>{{ $t('settings.smartFolders.syntaxIn') }}</code>,
        <code>{{ $t('settings.smartFolders.syntaxTag') }}</code>,
        <code>{{ $t('settings.smartFolders.syntaxHasAttachment') }}</code
        >{{ $t('settings.smartFolders.hintTail') }}
      </p>
      <label class="field"><span>{{ $t('common.name') }}</span><input v-model="name" /></label>
      <label class="field"><span>{{ $t('settings.smartFolders.iconLabel') }}</span><input v-model="icon" maxlength="2" /></label>
      <label class="field">
        <span>{{ $t('settings.smartFolders.queryLabel') }}</span>
        <input v-model="query" :placeholder="$t('settings.smartFolders.queryPlaceholder')" />
      </label>
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
