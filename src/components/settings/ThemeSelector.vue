<script setup lang="ts">
import { useSettingsStore } from '@/stores/settings';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import LanguageSelector from './LanguageSelector.vue';

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

      <!-- One card for the section: on its own, the language row was a second
           card with no heading of its own. -->
      <LanguageSelector />

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.singleKeyShortcuts') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.singleKeyShortcutsHint') }}</span>
        </div>
        <ToggleSwitch v-model="settings.singleKeyShortcuts" :label="$t('settings.theme.singleKeyShortcuts')" />
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
        <ToggleSwitch v-model="settings.autoDarkHtmlMails" :label="$t('settings.theme.autoDark')" />
      </div>

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.plainTextRead') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.plainTextReadHint') }}</span>
        </div>
        <ToggleSwitch v-model="settings.preferPlainTextReading" :label="$t('settings.theme.plainTextRead')" />
      </div>

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.plainTextCompose') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.plainTextComposeHint') }}</span>
        </div>
        <ToggleSwitch v-model="settings.composePlainText" :label="$t('settings.theme.plainTextCompose')" />
      </div>

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.theme.collapseQuoted') }}</span>
          <span class="setting-hint">{{ $t('settings.theme.collapseQuotedHint') }}</span>
        </div>
        <ToggleSwitch v-model="settings.collapseQuotedHistory" :label="$t('settings.theme.collapseQuoted')" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.section {
  display: grid;
  gap: var(--space-5);
}

.section-header h3 {
  margin: 0;
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
  color: var(--text-primary);
}

.section-desc {
  margin: var(--space-xs) 0 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}
</style>

<style scoped src="./settingsRows.css"></style>
