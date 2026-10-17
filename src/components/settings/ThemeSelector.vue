<script setup lang="ts">
import { useSettingsStore } from '@/stores/settings';
import AppButton from '@/components/ui/AppButton.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

const settings = useSettingsStore();

function setAttachmentOpenBehavior(value: string) {
  settings.attachmentOpenBehavior = value === 'system' ? 'system' : 'preview';
}
</script>

<template>
  <div class="section">
    <header class="section-header">
      <h3>{{ $t('settings.theme.title') }}</h3>
      <p class="section-desc">{{ $t('settings.theme.description') }}</p>
    </header>

    <div class="setting-group">
      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.appearance') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.appearanceHint') }}</span>
        </div>
        <AppButton variant="secondary" @click="settings.toggleTheme()">
          <template #icon>
            <!-- Sun -->
            <svg v-if="settings.theme === 'dark'" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="8" cy="8" r="3" />
              <path d="M8 1v2M8 13v2M1 8h2M13 8h2M3.3 3.3l1.4 1.4M11.3 11.3l1.4 1.4M3.3 12.7l1.4-1.4M11.3 4.7l1.4-1.4" />
            </svg>
            <!-- Moon -->
            <svg v-else width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M13.5 8.5a5.5 5.5 0 01-6-6 5.5 5.5 0 106 6z" />
            </svg>
          </template>
          {{ settings.theme === 'dark' ? $t('settings.theme.dark') : $t('settings.theme.light') }}
        </AppButton>
      </div>

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.singleKeyShortcuts') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.singleKeyShortcutsHint') }}</span>
        </div>
        <ToggleSwitch v-model="settings.singleKeyShortcuts" />
      </div>

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.attachmentOpen') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.attachmentOpenHint') }}</span>
        </div>
        <SegmentedControl
          :model-value="settings.attachmentOpenBehavior"
          :options="[
            { value: 'preview', label: 'Satchel' },
            { value: 'system', label: $t('settings.theme.attachmentOpenSystem') },
          ]"
          :aria-label="$t('settings.theme.attachmentOpenAria')"
          @update:model-value="setAttachmentOpenBehavior"
        />
      </div>

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.autoDark') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.autoDarkHint') }}</span>
        </div>
        <ToggleSwitch v-model="settings.autoDarkHtmlMails" />
      </div>

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.plainTextRead') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.plainTextReadHint') }}</span>
        </div>
        <ToggleSwitch v-model="settings.preferPlainTextReading" />
      </div>

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.plainTextCompose') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.plainTextComposeHint') }}</span>
        </div>
        <ToggleSwitch v-model="settings.composePlainText" />
      </div>

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.collapseQuoted') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.collapseQuotedHint') }}</span>
        </div>
        <ToggleSwitch v-model="settings.collapseQuotedHistory" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.section {
  display: grid;
  gap: 20px;
}

.section-header h3 {
  margin: 0;
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
  color: var(--text-primary);
}

.section-desc {
  margin: 4px 0 0;
  font-size: var(--font-size-sm);
  color: var(--text-tertiary);
}

.setting-group {
  display: grid;
  gap: 0;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  overflow: hidden;
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 14px 16px;
}

.setting-row + .setting-row {
  border-top: 1px solid var(--border-color-muted);
}

.setting-info {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.setting-label {
  font-size: var(--font-size-md);
  font-weight: var(--weight-medium);
  color: var(--text-primary);
}

.setting-hint {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}
</style>
