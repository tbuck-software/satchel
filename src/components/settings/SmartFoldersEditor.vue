<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useSmartFoldersStore } from '@/stores/smartFolders';
import { useCrudActions } from '@/composables/useCrudActions';

const { t } = useI18n();
const smart = useSmartFoldersStore();
const crud = useCrudActions();
const editingId = ref<string | null>(null);
const name = ref('');
const query = ref('');
const icon = ref('');
const status = ref('');

onMounted(() => smart.load());

function reset() {
  editingId.value = null;
  name.value = '';
  query.value = '';
  icon.value = '';
  status.value = '';
}

function edit(id: string) {
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
    <h3>{{ $t('settings.smartFolders.title') }}</h3>
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

    <ul v-if="smart.items.length > 0" class="list">
      <li v-for="f in smart.items" :key="f.id">
        <button type="button" class="row" @click="edit(f.id)">
          <span class="ico">{{ f.icon || '🔍' }}</span>
          <span>
            <strong>{{ f.name }}</strong>
            <code class="q">{{ f.query }}</code>
          </span>
        </button>
        <button type="button" class="delete" @click="remove(f.id)">{{ $t('common.delete') }}</button>
      </li>
    </ul>
    <p v-else class="empty">{{ $t('settings.smartFolders.empty') }}</p>

    <div class="editor">
      <h4>{{ editingId ? $t('settings.smartFolders.editTitle') : $t('settings.smartFolders.newTitle') }}</h4>
      <label class="field"><span>{{ $t('common.name') }}</span><input v-model="name" /></label>
      <label class="field"><span>{{ $t('settings.smartFolders.iconLabel') }}</span><input v-model="icon" maxlength="2" /></label>
      <label class="field">
        <span>{{ $t('settings.smartFolders.queryLabel') }}</span>
        <input v-model="query" :placeholder="$t('settings.smartFolders.queryPlaceholder')" />
      </label>
      <div class="actions">
        <button v-if="editingId" type="button" class="ghost" @click="reset">{{ $t('common.cancel') }}</button>
        <button type="button" class="primary" @click="save">{{ $t('common.save') }}</button>
      </div>
      <p v-if="status" class="status">{{ status }}</p>
    </div>
  </section>
</template>

<style scoped>
section { display: grid; gap: 14px; }
h3, h4 { margin: 0; color: var(--text-primary); }
.hint { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-sm); }
code { background: var(--bg-raised); padding: 1px 6px; border-radius: var(--radius-xs); font-size: 0.9em; }
.list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
.list li { display: flex; gap: 6px; }
.row { flex: 1; display: flex; gap: 8px; align-items: center; background: var(--bg-raised); border: 1px solid var(--border-color-muted); border-radius: var(--radius-sm); padding: 8px 12px; cursor: pointer; text-align: left; color: var(--text-primary); }
.row:hover { background: var(--bg-hover); }
.ico { font-size: 18px; }
.q { display: block; font-size: var(--font-size-xs); color: var(--text-tertiary); margin-top: 2px; }
.delete { background: transparent; border: 1px solid var(--border-color-muted); color: var(--text-secondary); padding: 0 12px; border-radius: var(--radius-sm); cursor: pointer; font-size: var(--font-size-xs); }
.empty { color: var(--text-tertiary); font-size: var(--font-size-sm); margin: 0; }
.editor { display: grid; gap: 10px; border: 1px solid var(--border-color-muted); border-radius: var(--radius-md); padding: 14px; background: var(--bg-base); }
.field { display: grid; gap: 4px; font-size: var(--font-size-sm); color: var(--text-secondary); }
.field input { border: 1px solid var(--border-color); background: var(--bg-raised); border-radius: var(--radius-sm); padding: 8px 10px; color: var(--text-primary); font-family: inherit; }
.actions { display: flex; justify-content: flex-end; gap: 8px; }
.primary { background: var(--accent-primary); color: var(--text-on-accent); border: 0; padding: 8px 16px; border-radius: var(--radius-sm); cursor: pointer; font-weight: var(--weight-semibold); }
.ghost { background: transparent; border: 1px solid var(--border-color); color: var(--text-primary); padding: 8px 16px; border-radius: var(--radius-sm); cursor: pointer; }
.status { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-xs); }
</style>
