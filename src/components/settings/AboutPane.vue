<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
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

    <p v-if="loading" role="status">{{ $t('settings.about.loading') }}</p>
    <ErrorState
      v-else-if="loadFailed"
      :title="$t('settings.about.loadFailed')"
      @retry="loadBuild"
    />
    <dl v-else-if="build" class="about-pane__build" data-testid="about-build">
      <div>
        <dt>{{ $t('settings.about.version') }}</dt>
        <dd class="about-pane__version">{{ build.version }}</dd>
      </div>
      <div>
        <dt>{{ $t('settings.about.commit') }}</dt>
        <dd class="about-pane__commit">{{ build.commit }}</dd>
      </div>
    </dl>

    <section class="about-pane__section">
      <SectionHeading :label="$t('settings.about.updates')" />
      <ToggleSwitch
        :label="$t('app.update.nightly')"
        :model-value="updates.channel === 'nightly'"
        :disabled="!updates.supported || updates.changingChannel || updates.checking || updates.status === 'downloading'"
        :before-change="(enabled) => updates.setChannel(enabled ? 'nightly' : 'stable')"
      >
        {{ $t('app.update.nightly') }}
      </ToggleSwitch>
      <p>{{ $t('app.update.nightlyHint') }}</p>
      <p v-if="updates.preferencesLoaded && updates.supported">
        {{ $t(updates.channel === 'nightly' ? 'app.update.followingNightly' : 'app.update.followingStable') }}
      </p>
      <p v-if="updates.preferencesLoaded && !updates.supported">{{ $t('app.update.sourceBuild') }}</p>
      <div class="about-pane__actions">
        <AppButton :disabled="!updates.supported || updates.changingChannel || updates.checking || updates.visible" @click="updates.checkForUpdates()">
          {{ $t(updates.checking ? 'app.update.checking' : 'app.update.check') }}
        </AppButton>
        <AppButton
          v-if="updates.visible"
          variant="primary"
          :disabled="updates.status === 'downloading'"
          @click="updates.activate()"
        >
          {{ updateActionLabel }}
        </AppButton>
      </div>
      <p v-if="updates.checkError" role="status">{{ updates.checkError }}</p>
      <p v-else-if="updates.visible && updates.version" role="status">
        {{ $t('app.update.version', { version: updates.version }) }}
      </p>
      <p v-else-if="updates.checked" role="status">
        {{ $t('app.update.upToDate') }}
      </p>
      <p v-if="updates.error" role="alert">{{ updates.error }}</p>
    </section>

    <section class="about-pane__section">
      <SectionHeading :label="$t('settings.about.openSource')" />
      <p>{{ $t('settings.about.license') }}</p>
      <div class="about-pane__actions">
        <AppButton @click="openProject()">{{ $t('settings.about.sourceCode') }}</AppButton>
        <AppButton @click="openProject('/blob/main/LICENSE')">{{ $t('settings.about.readLicense') }}</AppButton>
        <AppButton @click="openProject('/blob/main/THIRD_PARTY_NOTICES.md')">{{ $t('settings.about.thirdParty') }}</AppButton>
      </div>
      <p v-if="linkFailed" role="alert">{{ $t('settings.about.linkFailed') }}</p>
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
  font-weight: var(--weight-semibold);
}

.about-pane p {
  margin: var(--space-xs) 0 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.about-pane__build {
  display: grid;
  gap: var(--space-md);
  margin: 0;
}

.about-pane__build > div {
  display: grid;
  gap: var(--space-xs);
}

.about-pane dt {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.about-pane dd {
  margin: 0;
  overflow-wrap: anywhere;
  user-select: text;
}

.about-pane__version {
  font-size: var(--font-size-xl);
  font-weight: var(--weight-semibold);
  font-variant-numeric: tabular-nums;
}

.about-pane__commit {
  color: var(--text-tertiary);
  font-family: var(--font-mono);
  font-size: var(--font-size-xs);
}

.about-pane__section {
  display: grid;
  gap: var(--space-sm);
  padding-top: var(--space-lg);
  border-top: 1px solid var(--border-color-muted);
}

.about-pane__actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-sm);
}
</style>
