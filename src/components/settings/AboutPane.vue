<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import CardButton from '@/components/ui/CardButton.vue';
import ErrorState from '@/components/ui/ErrorState.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { useAppUpdateStore } from '@/stores/appUpdate';
import {
  backendAppBuildInfo,
  backendCommandsAvailable,
  backendOpenExternalUrl,
  type BackendBuildInfo,
} from '@/services/backend';

const { t } = useI18n();
const build = ref<BackendBuildInfo | null>(null);
const loading = ref(false);
const loadFailed = ref(false);
const linkFailed = ref(false);
const updates = useAppUpdateStore();
const updateActionLabel = computed(() => {
  if (updates.status === 'downloading') return t('app.update.downloading', { progress: updates.progress });
  if (updates.status === 'ready') return t('app.update.restart');
  if (updates.status === 'failed') return t('app.update.failedRetry');
  return t('app.update.available');
});

/** The update row: which channel is followed, and what the last check found. */
const updateStatusLabel = computed(() => {
  if (updates.preferencesLoaded && !updates.supported) return t('app.update.sourceBuild');
  return t(updates.channel === 'nightly' ? 'app.update.followingNightly' : 'app.update.followingStable');
});
const updateStatusHint = computed(() => {
  if (updates.checkError) return updates.checkError;
  if (updates.visible && updates.version) return t('app.update.version', { version: updates.version });
  if (updates.checked) return t('app.update.upToDate');
  return '';
});

async function loadBuild() {
  loading.value = true;
  loadFailed.value = false;
  try {
    if (backendCommandsAvailable()) {
      build.value = await backendAppBuildInfo();
    } else {
      loadFailed.value = true;
    }
  } catch {
    loadFailed.value = true;
  } finally {
    loading.value = false;
  }
}

async function openProject(path = '') {
  linkFailed.value = false;
  try {
    await backendOpenExternalUrl(`https://github.com/tbuck-software/satchel${path}`);
  } catch {
    linkFailed.value = true;
  }
}

onMounted(() => { void loadBuild(); void updates.loadPreferences(); });
</script>

<template>
  <section class="about-pane">
    <header class="about-pane__identity">
      <img src="/branding/satchel-smile-check.svg" width="64" height="64" alt="" />
      <div>
        <h2>{{ $t('settings.about.name') }}</h2>
        <p>{{ $t('settings.about.description') }}</p>
      </div>
    </header>

    <!-- Grouped rows like every other section: label left, value or control
         right. Free text and a switch before its label read as another app. -->
    <p v-if="loading" role="status">{{ $t('settings.about.loading') }}</p>
    <ErrorState
      v-else-if="loadFailed"
      :title="$t('settings.about.loadFailed')"
      @retry="loadBuild"
    />
    <div v-else-if="build" class="setting-group" data-testid="about-build">
      <div class="setting-row">
        <span class="setting-label">{{ $t('settings.about.version') }}</span>
        <span class="about-pane__value about-pane__version">{{ build.version }}</span>
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ $t('settings.about.commit') }}</span>
        <span class="about-pane__value about-pane__commit">{{ build.commit }}</span>
      </div>
    </div>

    <section class="about-pane__section">
      <SectionHeading :label="$t('settings.about.updates')" />
      <div class="setting-group">
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">{{ $t('app.update.nightly') }}</span>
            <span class="setting-hint">{{ $t('app.update.nightlyHint') }}</span>
          </div>
          <ToggleSwitch
            :label="$t('app.update.nightly')"
            :model-value="updates.channel === 'nightly'"
            :disabled="!updates.supported || updates.changingChannel || updates.checking || updates.status === 'downloading'"
            :before-change="(enabled) => updates.setChannel(enabled ? 'nightly' : 'stable')"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info" role="status">
            <span class="setting-label">{{ updateStatusLabel }}</span>
            <span v-if="updateStatusHint" class="setting-hint">{{ updateStatusHint }}</span>
          </div>
          <div class="about-pane__actions">
            <AppButton
              variant="secondary"
              size="sm"
              :disabled="!updates.supported || updates.changingChannel || updates.checking || updates.visible"
              @click="updates.checkForUpdates()"
            >
              {{ $t(updates.checking ? 'app.update.checking' : 'app.update.check') }}
            </AppButton>
            <AppButton
              v-if="updates.visible"
              variant="primary"
              size="sm"
              :disabled="updates.status === 'downloading'"
              @click="updates.activate()"
            >
              {{ updateActionLabel }}
            </AppButton>
          </div>
        </div>
      </div>
      <p v-if="updates.error" class="about-pane__error" role="alert">{{ updates.error }}</p>
    </section>

    <section class="about-pane__section">
      <SectionHeading :label="$t('settings.about.openSource')" />
      <div class="setting-group">
        <div class="setting-row about-pane__note">
          <span class="setting-hint">{{ $t('settings.about.license') }}</span>
        </div>
        <CardButton class="setting-row about-pane__link" @click="openProject()">
          <span class="setting-label">{{ $t('settings.about.sourceCode') }}</span>
          <AppIcon class="about-pane__out"><path d="M6 3.5h6.5V10M12.5 3.5 4 12" /></AppIcon>
        </CardButton>
        <CardButton class="setting-row about-pane__link" @click="openProject('/blob/main/LICENSE')">
          <span class="setting-label">{{ $t('settings.about.readLicense') }}</span>
          <AppIcon class="about-pane__out"><path d="M6 3.5h6.5V10M12.5 3.5 4 12" /></AppIcon>
        </CardButton>
        <CardButton class="setting-row about-pane__link" @click="openProject('/blob/main/THIRD_PARTY_NOTICES.md')">
          <span class="setting-label">{{ $t('settings.about.thirdParty') }}</span>
          <AppIcon class="about-pane__out"><path d="M6 3.5h6.5V10M12.5 3.5 4 12" /></AppIcon>
        </CardButton>
      </div>
      <p v-if="linkFailed" class="about-pane__error" role="alert">{{ $t('settings.about.linkFailed') }}</p>
    </section>
  </section>
</template>

<style scoped>
.about-pane {
  display: grid;
  gap: var(--space-xl);
  color: var(--text-primary);
}

.about-pane__identity {
  display: flex;
  align-items: center;
  gap: var(--space-lg);
}

.about-pane__identity img {
  flex-shrink: 0;
}

.about-pane h2 {
  margin: 0;
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
}

.about-pane__identity p,
.about-pane > p,
.about-pane__error {
  margin: var(--space-xs) 0 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.about-pane__value {
  color: var(--text-secondary);
  overflow-wrap: anywhere;
  user-select: text;
}

.about-pane__version {
  font-variant-numeric: tabular-nums;
}

.about-pane__commit {
  font-family: var(--font-mono);
  font-size: var(--font-size-sm);
}

.about-pane__section {
  display: grid;
  gap: var(--space-sm);
}

.about-pane__actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-sm);
}

.setting-row.about-pane__note {
  grid-template-columns: minmax(0, 1fr);
}

.setting-row.about-pane__note > :last-child {
  justify-self: start;
}

.about-pane__link:hover {
  background: var(--bg-hover);
}

.about-pane__out {
  color: var(--text-tertiary);
}
</style>

<style scoped src="./settingsRows.css"></style>
