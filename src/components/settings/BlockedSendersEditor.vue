<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendBlockSender,
  backendListBlockedSenders,
  backendSupportsNativeCommands,
  backendUnblockSender,
  type BackendBlockedSender,
} from '@/services/backend';
import AppButton from '@/components/ui/AppButton.vue';

const { t } = useI18n();
const items = ref<BackendBlockedSender[]>([]);
const newPattern = ref('');
const status = ref('');

async function load() {
  if (!backendSupportsNativeCommands()) return;
  items.value = await backendListBlockedSenders();
}

onMounted(load);

async function add() {
  if (!newPattern.value.trim()) return;
  try {
    await backendBlockSender({ pattern: newPattern.value.trim() });
    newPattern.value = '';
    await load();
    status.value = t('settings.blocked.added');
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  }
}

async function remove(id: string) {
  await backendUnblockSender(id);
  await load();
}
</script>

<template>
  <section>
    <h3>{{ $t('settings.sections.blocked.label') }}</h3>
    <p class="hint">
      {{ $t('settings.blocked.hintBefore') }}
      <code>{{ $t('settings.blocked.exampleDomain') }}</code>{{ $t('settings.blocked.hintAfter') }}
    </p>

    <div class="add">
      <input v-model="newPattern" :placeholder="$t('settings.blocked.placeholder')" @keydown.enter="add" />
      <AppButton variant="primary" @click="add">{{ $t('settings.blocked.add') }}</AppButton>
    </div>

    <ul v-if="items.length > 0" class="list">
      <li v-for="item in items" :key="item.id">
        <span>
          <strong>{{ item.pattern }}</strong>
          <small>· {{ item.pattern_type }}</small>
        </span>
        <AppButton variant="secondary" size="sm" @click="remove(item.id)">
          {{ $t('settings.blocked.unblock') }}
        </AppButton>
      </li>
    </ul>
    <p v-else class="empty">{{ $t('settings.blocked.empty') }}</p>

    <p v-if="status" class="status">{{ status }}</p>
  </section>
</template>

<style scoped>
section { display: grid; gap: 12px; }
h3 { margin: 0; color: var(--text-primary); }
.hint { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-sm); }
code { background: var(--bg-raised); padding: 1px 6px; border-radius: var(--radius-xs); font-size: 0.9em; }
.add { display: flex; gap: 8px; }
.add input { flex: 1; border: 1px solid var(--border-color); background: var(--bg-raised); border-radius: var(--radius-sm); padding: 8px 10px; color: var(--text-primary); font-family: inherit; }
.list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.list li { display: flex; justify-content: space-between; align-items: center; background: var(--bg-raised); padding: 8px 12px; border-radius: var(--radius-sm); color: var(--text-primary); font-size: var(--font-size-sm); }
.list small { color: var(--text-tertiary); margin-left: 6px; }
.empty { color: var(--text-tertiary); font-size: var(--font-size-sm); margin: 0; }
.status { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-xs); }
</style>
