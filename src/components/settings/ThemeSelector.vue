<script setup lang="ts">
import { useSettingsStore } from '@/stores/settings';
import { useUiStore } from '@/stores/ui';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import LanguageSelector from './LanguageSelector.vue';

const settings = useSettingsStore();
const ui = useUiStore();

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

    <!-- Two groups instead of one slab: how the app looks and is driven,
         then how mail is opened, shown and written. -->
    <div class="setting-groups">
      <section class="setting-cluster">
        <SectionHeading :label="$t('settings.theme.groupInterface')" />
        <div class="setting-group setting-group--general">
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

          <!-- Language sits in the Interface card: on its own, its row was a
               second card with no heading of its own. -->
          <LanguageSelector />

          <div v-if="!ui.isMobile" class="setting-row">
            <div class="setting-info">
              <span class="setting-label">{{ $t('settings.theme.singleKeyShortcuts') }}</span>
              <span class="setting-hint">{{ $t('settings.theme.singleKeyShortcutsHint') }}</span>
            </div>
            <ToggleSwitch v-model="settings.singleKeyShortcuts" :label="$t('settings.theme.singleKeyShortcuts')" />
          </div>
        </div>
      </section>

      <section class="setting-cluster">
        <SectionHeading :label="$t('settings.theme.groupMail')" />
        <div class="setting-group setting-group--general">
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
      </section>
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

.setting-groups {
  display: grid;
  gap: var(--space-xl);
}

.setting-cluster {
  display: grid;
  gap: var(--space-sm);
}

/* General's cards are sheets on the pane rather than grey slabs. Scoped to
   this editor, so the other settings sections keep the shared raised card;
   the doubled class outranks settingsRows.css, which loads after this block. */
.setting-group.setting-group--general {
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
}
</style>

<style scoped src="./settingsRows.css"></style>
