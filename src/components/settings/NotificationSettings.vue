<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendGetNotificationPreferences,
  backendSetBackgroundSync,
  backendCommandsAvailable,
  backendSetNotificationPreferences,
  type BackendNotificationPreferences,
} from '@/services/backend';
import { ensureNotificationPermission } from '@/composables/useNotifications';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

const props = withDefaults(defineProps<{ section?: 'notifications' | 'sending' | 'data' }>(), {
  section: 'notifications',
});
const { t } = useI18n();

const prefs = ref<BackendNotificationPreferences>({
  enabled: true,
  scope: 'inbox',
  sound: true,
  undo_send_seconds: 5,
  trash_auto_delete_days: 30,
  send_read_receipts: false,
  body_window_days: 0,
});
const status = ref('');
const saveFailed = ref(false);
const permissionDenied = ref(false);

/**
 * Turning the switch on is the moment to ask the system.
 *
 * Android 13 and newer refuse to post anything until the user has been asked,
 * and refuse silently — the preference read "an" while nothing ever arrived.
 * Asking here rather than at startup also puts the prompt where the user can
 * see what it is for. A denial has to hold the switch back, which is what
 * ToggleSwitch's `before-change` is for.
 */
async function confirmEnabled(next: boolean): Promise<boolean> {
  if (!next) {
    permissionDenied.value = false;
    return true;
  }
  const granted = await ensureNotificationPermission();
  permissionDenied.value = !granted;
  return granted;
}

async function applyEnabled(next: boolean) {
  prefs.value.enabled = next;
  // The main switch takes the background connection with it: no wait.
  if (pending) clearTimeout(pending);
  pending = null;
  await save();
}

/**
 * Every change applies as it is made, like General's.
 *
 * A switch that has visibly moved reads as applied, and every platform treats
 * one that way. A Save button at the foot of the page meant a user could turn
 * notifications on, watch the switch slide over, walk away, and get nothing.
 * Typed numbers wait a moment so each keystroke is not its own write.
 */
let loaded = false;
let lastSaved = '';
let pending: ReturnType<typeof setTimeout> | null = null;
watch(prefs, (value) => {
  if (!loaded || JSON.stringify(value) === lastSaved) return;
  if (pending) clearTimeout(pending);
  pending = setTimeout(() => {
    pending = null;
    if (JSON.stringify(prefs.value) !== lastSaved) void save();
  }, 400);
}, { deep: true });
onBeforeUnmount(() => {
  if (!pending) return;
  clearTimeout(pending);
  void save();
});

const UNDO_PRESETS = [0, 5, 10, 20, 30];
const undoOptions = computed(() => {
  const current = prefs.value.undo_send_seconds;
  const values = UNDO_PRESETS.includes(current) ? UNDO_PRESETS : [...UNDO_PRESETS, current].sort((a, b) => a - b);
  return values.map((value) => ({
    value: String(value),
    label: value === 0 ? t('settings.notifications.undoOff') : t('settings.notifications.secondsShort', { n: value }),
  }));
});
const scopeOptions = computed(() => [
  { value: 'all', label: t('settings.notifications.scopeAllShort') },
  { value: 'inbox', label: t('settings.notifications.scopeInboxShort') },
  { value: 'off', label: t('common.none') },
]);

const heading = computed(() => ({
  notifications: [t('settings.notifications.title'), t('settings.notifications.description')],
  sending: [
    t('settings.notifications.sendingTitle'),
    t('settings.notifications.sendingDescription'),
  ],
  data: [t('settings.notifications.dataTitle'), t('settings.notifications.dataDescription')],
}[props.section]));

onMounted(async () => {
  if (!backendCommandsAvailable()) return;
  prefs.value = await backendGetNotificationPreferences();
  lastSaved = JSON.stringify(prefs.value);
  loaded = true;
});

async function save() {
  status.value = '';
  saveFailed.value = false;
  try {
    const saved = await backendSetNotificationPreferences(prefs.value);
    lastSaved = JSON.stringify(saved);
    prefs.value = saved;
    // Follows the switch: with notifications off there is nothing for the
    // background connection to announce, and holding the process open would
    // cost battery for silence.
    await backendSetBackgroundSync(prefs.value.enabled);
    status.value = t('settings.notifications.saved');
  } catch (err) {
    saveFailed.value = true;
    status.value = err instanceof Error ? err.message : String(err);
  }
}
</script>

<template>
  <section class="settings-section">
    <header>
      <h3 class="settings-page-title">{{ heading[0] }}</h3>
      <p>{{ heading[1] }}</p>
    </header>

    <div v-if="props.section === 'notifications'" class="setting-group">
      <div class="setting-row">
        <div class="setting-info">
          <strong>{{ $t('settings.notifications.enabledLabel') }}</strong>
          <span>{{ $t('settings.notifications.enabledHint') }}</span>
        </div>
        <ToggleSwitch
          :model-value="prefs.enabled"
          :label="$t('settings.notifications.enabledLabel')"
          :before-change="confirmEnabled"
          @update:model-value="applyEnabled"
        />
      </div>
      <!-- Without this the switch simply refused to move and the user had no
           way to know the system had said no, or where to change it. -->
      <p v-if="permissionDenied" class="permission-note">
        {{ $t('settings.notifications.permissionDenied') }}
      </p>
      <div class="setting-row">
        <div class="setting-info">
          <strong>{{ $t('settings.notifications.scopeLabel') }}</strong>
          <span>{{ $t('settings.notifications.scopeHint') }}</span>
        </div>
        <SegmentedControl
          v-model="prefs.scope"
          :options="scopeOptions.map((option) => ({ ...option, disabled: !prefs.enabled }))"
          :aria-label="$t('settings.notifications.scopeLabel')"
        />
      </div>
      <div class="setting-row">
        <div class="setting-info">
          <strong>{{ $t('settings.notifications.soundLabel') }}</strong>
          <span>{{ $t('settings.notifications.soundHint') }}</span>
        </div>
        <ToggleSwitch
          v-model="prefs.sound"
          :disabled="!prefs.enabled"
          :label="$t('settings.notifications.soundLabel')"
        />
      </div>
    </div>

    <div v-else-if="props.section === 'sending'" class="setting-group">
      <div class="setting-row">
        <div class="setting-info">
          <strong>{{ $t('settings.notifications.undoLabel') }}</strong>
          <span>{{ $t('settings.notifications.undoHint') }}</span>
        </div>
        <SegmentedControl
          :model-value="String(prefs.undo_send_seconds)"
          :options="undoOptions"
          :aria-label="$t('settings.notifications.undoLabel')"
          @update:model-value="prefs.undo_send_seconds = Number($event)"
        />
      </div>
      <div class="setting-row">
        <div class="setting-info">
          <strong>{{ $t('settings.notifications.receiptsLabel') }}</strong>
          <span>{{ $t('settings.notifications.receiptsHint') }}</span>
        </div>
        <ToggleSwitch
          v-model="prefs.send_read_receipts"
          :label="$t('settings.notifications.receiptsLabel')"
        />
      </div>
    </div>

    <div v-else class="setting-group">
      <label class="setting-row">
        <span class="setting-info">
          <strong>{{ $t('settings.notifications.bodyWindowLabel') }}</strong>
          <span>{{ $t('settings.notifications.bodyWindowHint') }}</span>
        </span>
        <span class="number-control">
          <input v-model.number="prefs.body_window_days" type="number" min="0" max="3650" step="1" />
          <small>{{ $t('settings.notifications.days') }}</small>
        </span>
      </label>
      <label class="setting-row">
        <span class="setting-info">
          <strong>{{ $t('settings.notifications.trashLabel') }}</strong>
          <span>{{ $t('settings.notifications.trashHint') }}</span>
        </span>
        <span class="number-control">
          <input v-model.number="prefs.trash_auto_delete_days" type="number" min="0" max="3650" step="1" />
          <small>{{ $t('settings.notifications.days') }}</small>
        </span>
      </label>
    </div>

    <p v-if="saveFailed" class="status error" role="alert">{{ status }}</p>
  </section>
</template>

<style scoped>
.settings-section { display:grid; gap:20px; }
.number-control { display:flex; align-items:center; gap:6px; border-radius: var(--radius-sm); background:var(--bg-overlay); padding:3px 9px; }
.number-control input { width:58px; border:0; background:transparent; color:var(--text-primary); font:inherit; text-align:right; }
.number-control small { color:var(--text-tertiary); }
.status { margin:0; font-size: var(--font-size-sm); }
.status.error { color:var(--accent-red); }

.permission-note {
  margin: 0 0 8px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--accent-amber) 30%, var(--border-color));
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-amber) 10%, var(--bg-raised));
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  line-height: 1.5;
}
</style>

<style scoped src="./settingsRows.css"></style>
