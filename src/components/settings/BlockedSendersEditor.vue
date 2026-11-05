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
    <h3 class="settings-page-title">{{ $t('settings.sections.blocked.label') }}</h3>
    <p class="hint">
      {{ $t('settings.blocked.hintBefore') }}
      <code>{{ $t('settings.blocked.exampleDomain') }}</code>{{ $t('settings.blocked.hintAfter') }}
    </p>

    <!-- One list: the add field is its first row, the entries follow. -->
    <div class="setting-group list">
      <form class="setting-row add-row" @submit.prevent="add">
        <input v-model="newPattern" :placeholder="$t('settings.blocked.placeholder')" :aria-label="$t('settings.blocked.placeholder')" />
        <AppButton type="submit" variant="secondary" size="sm" :disabled="!newPattern.trim()">{{ $t('settings.blocked.add') }}</AppButton>
      </form>
      <div v-for="item in items" :key="item.id" class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ item.pattern }}</span>
          <span class="setting-hint">{{ item.pattern_type }}</span>
        </div>
        <AppButton variant="ghost" size="sm" @click="remove(item.id)">
          {{ $t('settings.blocked.unblock') }}
        </AppButton>
      </div>
      <div v-if="items.length === 0" class="setting-row setting-row--single">
        <span class="setting-hint">{{ $t('settings.blocked.empty') }}</span>
      </div>
    </div>

    <p v-if="status" class="status">{{ status }}</p>
  </section>
</template>

<style scoped>
.setting-row.add-row { grid-template-columns: minmax(0, 1fr) auto; }
.add-row input { width: 100%; max-width: none; }
</style>

<style scoped src="./settingsForms.css"></style>

<style scoped src="./settingsRows.css"></style>
