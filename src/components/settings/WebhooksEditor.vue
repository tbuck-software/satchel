<script setup lang="ts">
import { nextTick, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { useConfirm } from '@/composables/useConfirm';
import { useToast } from '@/composables/useToast';
import {
  backendCommandsAvailable,
  backendDeleteWebhook,
  backendListWebhooks,
  backendUpsertWebhook,
  type BackendWebhook,
} from '@/services/backend';
import { backendErrorLabel } from '@/utils/backendErrorLabels';

/**
 * Webhooks: where Satchel posts when mail arrives or a thread changes.
 *
 * They sit with the accounts and feeds (what goes in and out), as a list with
 * an Add row; the form opens on demand like every other collection.
 */
const { t } = useI18n();
const { ask } = useConfirm();
const toast = useToast();

const webhooks = ref<BackendWebhook[]>([]);
const adding = ref(false);
const url = ref('');
const events = ref('message.received, thread.updated');
const urlInput = ref<HTMLInputElement | null>(null);

async function load() {
  if (!backendCommandsAvailable()) return;
  try {
    webhooks.value = await backendListWebhooks();
  } catch {
    webhooks.value = [];
  }
}

/**
 * `events_json` is storage, not copy. A row whose JSON is unreadable still has
 * a URL worth showing and a delete button worth reaching.
 */
function eventList(webhook: BackendWebhook): string[] {
  try {
    const parsed: unknown = JSON.parse(webhook.events_json);
    return Array.isArray(parsed) ? parsed.map((event) => String(event)) : [];
  } catch {
    return [];
  }
}

async function setEnabled(webhook: BackendWebhook, enabled: boolean) {
  const previous = webhook.enabled;
  // Optimistic: the switch has to move under the finger, not after a round trip.
  webhook.enabled = enabled ? 1 : 0;
  try {
    await backendUpsertWebhook({ id: webhook.id, url: webhook.url, events: eventList(webhook), enabled });
  } catch (error) {
    webhook.enabled = previous;
    toast.error(t('settings.plugins.webhookToggleFailed', { error: backendErrorLabel(error) }));
  }
}

async function remove(webhook: BackendWebhook) {
  // A webhook forwards message metadata to a foreign host. Name the host in
  // the question, so nobody deletes the wrong one from a list of two.
  const confirmed = await ask({
    title: t('settings.plugins.webhookDeleteTitle'),
    body: t('settings.plugins.webhookDeleteConfirm', { url: webhook.url }),
    confirmLabel: t('settings.plugins.deleteWebhook'),
    tone: 'danger',
  });
  if (!confirmed) return;
  try {
    await backendDeleteWebhook(webhook.id);
    webhooks.value = webhooks.value.filter((entry) => entry.id !== webhook.id);
    toast.success(t('settings.plugins.webhookDeleted', { url: webhook.url }));
  } catch (error) {
    toast.error(t('settings.plugins.webhookDeleteFailed', { error: backendErrorLabel(error) }));
  }
}

function startAdding() {
  adding.value = true;
  void nextTick(() => urlInput.value?.focus());
}

function cancel() {
  adding.value = false;
  url.value = '';
}

async function add() {
  try {
    await backendUpsertWebhook({
      url: url.value.trim(),
      events: events.value.split(',').map((event) => event.trim()).filter(Boolean),
      enabled: true,
    });
    cancel();
    await load();
  } catch (error) {
    toast.error(t('settings.plugins.webhookSaveFailed', { error: (error as Error).message ?? String(error) }));
  }
}

onMounted(load);
</script>

<template>
  <section class="webhooks">
    <header>
      <h3 class="settings-page-title">{{ $t('settings.plugins.webhooksTitle') }}</h3>
      <p>{{ $t('settings.plugins.webhooksDescription') }}</p>
    </header>
    <div class="setting-group">
      <div v-for="webhook in webhooks" :key="webhook.id" class="setting-row webhook-row">
        <div class="setting-info">
          <span class="setting-label webhook-url">{{ webhook.url }}</span>
          <span class="setting-hint webhook-events">
            {{ eventList(webhook).length ? eventList(webhook).join(' · ') : $t('settings.plugins.webhookNoEvents') }}
          </span>
        </div>
        <div class="webhooks__actions">
          <ToggleSwitch
            :model-value="webhook.enabled !== 0"
            :label="webhook.url"
            @update:model-value="setEnabled(webhook, $event)"
          />
          <AppButton variant="ghost" size="sm" @click="remove(webhook)">{{ $t('common.delete') }}</AppButton>
        </div>
      </div>
      <div v-if="webhooks.length === 0 && !adding" class="setting-row setting-row--single">
        <span class="setting-hint">{{ $t('settings.plugins.webhooksEmpty') }}</span>
      </div>
      <form v-if="adding" class="setting-row setting-row--single webhooks__form" @submit.prevent="add">
        <div class="webhooks__fields">
          <label class="field">
            <span class="setting-label">{{ $t('settings.plugins.webhookUrl') }}</span>
            <input ref="urlInput" v-model="url" type="url" :placeholder="$t('settings.plugins.webhookUrlPlaceholder')" />
          </label>
          <label class="field">
            <span class="setting-label">{{ $t('settings.plugins.webhookEvents') }}</span>
            <input v-model="events" />
          </label>
          <div class="webhooks__form-actions">
            <AppButton variant="ghost" size="sm" @click="cancel">{{ $t('common.cancel') }}</AppButton>
            <AppButton type="submit" variant="primary" size="sm" :disabled="!url.trim()">
              {{ $t('settings.plugins.addWebhook') }}
            </AppButton>
          </div>
        </div>
      </form>
      <div v-else class="setting-row setting-row--single">
        <AppButton variant="secondary" size="sm" @click="startAdding">
          <template #icon><AppIcon><path d="M8 3v10M3 8h10" /></AppIcon></template>
          {{ $t('settings.plugins.newWebhook') }}
        </AppButton>
      </div>
    </div>
  </section>
</template>

<style scoped>
.webhooks {
  display: grid;
  gap: var(--space-sm);
}

.webhook-url {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.webhooks__actions {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
}

.webhooks__fields {
  display: grid;
  gap: var(--space-md);
  width: 100%;
}

.field {
  display: grid;
  gap: var(--space-xs);
}

.field input {
  padding: 8px 10px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
}

.webhooks__form-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
}

.setting-row.webhooks__form > :last-child {
  justify-self: stretch;
}
</style>

<style scoped src="./settingsRows.css"></style>
