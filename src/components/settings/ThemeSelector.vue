<script setup lang="ts">
import { useSettingsStore } from '@/stores/settings';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

const settings = useSettingsStore();

function setTheme(value: string) {
  if (value === 'light' || value === 'dark') settings.theme = value;
}

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
        <SegmentedControl
          :model-value="settings.theme"
          :options="[
            { value: 'light', label: $t('settings.theme.light') },
            { value: 'dark', label: $t('settings.theme.dark') },
          ]"
          :aria-label="$t('settings.theme.appearance')"
          @update:model-value="setTheme"
        />
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
