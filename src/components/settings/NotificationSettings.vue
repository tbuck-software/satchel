<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendGetNotificationPreferences,
  backendSetBackgroundSync,
  backendSetNotificationPreferences,
  backendSupportsNativeCommands,
  type BackendNotificationPreferences,
} from '@/services/backend';
import { ensureNotificationPermission } from '@/composables/useNotifications';
import AppButton from '@/components/ui/AppButton.vue';
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
const saving = ref(false);
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
  // Persist straight away rather than waiting for the Speichern button.
  //
  // A switch that has visibly moved reads as applied, and every platform
  // outside this form treats one that way. Leaving it unsaved meant a user
  // could turn notifications on, watch the switch slide over, walk away, and
  // get nothing — with no clue that a button elsewhere on the page still had
  // to be pressed.
  await save();
}

const heading = computed(() => ({
  notifications: [t('settings.notifications.title'), t('settings.notifications.description')],
  sending: [
    t('settings.notifications.sendingTitle'),
    t('settings.notifications.sendingDescription'),
  ],
  data: [t('settings.notifications.dataTitle'), t('settings.notifications.dataDescription')],
}[props.section]));

onMounted(async () => {
  if (!backendSupportsNativeCommands()) return;
  prefs.value = await backendGetNotificationPreferences();
});

async function save() {
  saving.value = true;
  status.value = '';
  saveFailed.value = false;
  try {
    prefs.value = await backendSetNotificationPreferences(prefs.value);
    // Follows the switch: with notifications off there is nothing for the
    // background connection to announce, and holding the process open would
    // cost battery for silence.
    await backendSetBackgroundSync(prefs.value.enabled);
    status.value = t('settings.notifications.saved');
  } catch (err) {
    saveFailed.value = true;
    status.value = err instanceof Error ? err.message : String(err);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <section class="settings-section">
    <header>
      <h3>{{ heading[0] }}</h3>
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
      <label class="setting-row">
        <span class="setting-info">
          <strong>{{ $t('settings.notifications.scopeLabel') }}</strong>
          <span>{{ $t('settings.notifications.scopeHint') }}</span>
        </span>
        <select v-model="prefs.scope" :disabled="!prefs.enabled">
          <option value="all">{{ $t('settings.notifications.scopeAll') }}</option>
          <option value="inbox">{{ $t('settings.notifications.scopeInbox') }}</option>
          <option value="off">{{ $t('common.none') }}</option>
        </select>
      </label>
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
      <label class="setting-row">
        <span class="setting-info">
          <strong>{{ $t('settings.notifications.undoLabel') }}</strong>
          <span>{{ $t('settings.notifications.undoHint') }}</span>
        </span>
        <span class="number-control">
          <input v-model.number="prefs.undo_send_seconds" type="number" min="0" max="60" step="1" />
          <small>{{ $t('settings.notifications.seconds') }}</small>
        </span>
      </label>
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

    <div class="save-row">
      <span :class="['status', { error: saveFailed }]">{{ status }}</span>
      <AppButton variant="primary" :disabled="saving" @click="save">
        {{ saving ? $t('common.saving') : $t('common.save') }}
      </AppButton>
    </div>
  </section>
</template>

<style scoped>
.settings-section { display:grid; gap:20px; }
header h3 { margin:0; color:var(--text-primary); font-size:var(--font-size-2xl); letter-spacing:-.02em; }
header p { margin:5px 0 0; color:var(--text-tertiary); font-size:var(--font-size-sm); }
.setting-group { overflow:hidden; border-radius:var(--radius-md); background:var(--bg-raised); }
.setting-row { display:flex; align-items:center; justify-content:space-between; gap:18px; padding:15px 16px; color:inherit; }
.setting-row + .setting-row { border-top:1px solid var(--border-color-muted); }
.setting-info { min-width:0; display:grid; gap:3px; }
.setting-info strong { color:var(--text-primary); font-size:var(--font-size-md); font-weight: var(--weight-medium); }
.setting-info span { color:var(--text-tertiary); font-size:var(--font-size-xs); line-height:1.4; }
select { max-width:230px; border:0; border-radius: var(--radius-sm); background:var(--bg-overlay); color:var(--text-primary); padding:8px 10px; font:inherit; font-size: var(--font-size-sm); }
.number-control { display:flex; align-items:center; gap:6px; border-radius: var(--radius-sm); background:var(--bg-overlay); padding:3px 9px; }
.number-control input { width:58px; border:0; background:transparent; color:var(--text-primary); font:inherit; text-align:right; }
.number-control small { color:var(--text-tertiary); }
.save-row { display:flex; align-items:center; justify-content:flex-end; gap:12px; }
.status { margin-right:auto; color:var(--accent-green); font-size: var(--font-size-xs); }
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
