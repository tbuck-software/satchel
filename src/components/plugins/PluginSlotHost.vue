<script setup lang="ts">
import { onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendExecutePluginCommand,
  backendListPlugins,
  type BackendPlugin,
} from '@/services/backend';

const props = defineProps<{
  slotName: 'top_toolbar' | 'thread_view_panel' | 'sidebar_section' | 'assistant_context_panel';
  context?: Record<string, unknown>;
}>();

const plugins = ref<BackendPlugin[]>([]);
const outputs = ref<Array<{ pluginId: string; text: string; error?: string }>>([]);
const { t } = useI18n();

async function renderOutputs() {
  outputs.value = await Promise.all(
    plugins.value.map(async (plugin) => {
      try {
        const text = await backendExecutePluginCommand({
          pluginId: plugin.id,
          command: `render-slot:${props.slotName}`,
          payload: JSON.stringify({
            slot: props.slotName,
            context: props.context ?? {},
          }),
        });
        return { pluginId: plugin.id, text };
      } catch (error) {
        return {
          pluginId: plugin.id,
          text: '',
          error: (error as Error).message ?? String(error),
        };
      }
    }),
  );
}

async function load() {
  try {
    plugins.value = (await backendListPlugins()).filter((plugin) =>
      plugin.ui_slots.includes(props.slotName),
    );
    await renderOutputs();
  } catch {
    plugins.value = [];
    outputs.value = [];
  }
}

onMounted(load);
watch(() => props.slotName, load);
// A new context re-renders the known plugins; the list itself is asked for once
// per mount. The reader passes a fresh context for every thread it shows.
watch(() => JSON.stringify(props.context ?? {}), () => void renderOutputs());
</script>

<template>
  <div v-if="outputs.length" class="plugin-slot-host" :data-slot="props.slotName">
    <article v-for="output in outputs" :key="output.pluginId" class="plugin-slot-card">
      <strong>{{ output.pluginId }}</strong>
      <p v-if="output.error" class="slot-error">{{ output.error }}</p>
      <p v-else>{{ output.text || t('settings.plugins.noContent') }}</p>
    </article>
  </div>
</template>

<style scoped>
.plugin-slot-host {
  display: grid;
  gap: 8px;
}

.plugin-slot-card {
  display: grid;
  gap: 4px;
  padding: 8px 10px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.plugin-slot-card strong {
  color: var(--text-primary);
}

.plugin-slot-card p {
  margin: 0;
}

.slot-error {
  color: var(--accent-red-soft);
}
</style>
