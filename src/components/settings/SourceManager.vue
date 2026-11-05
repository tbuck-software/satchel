<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useConfirm } from "@/composables/useConfirm";
import {
  backendCreateSource,
  backendDeleteSource,
  backendListSources,
  backendImportOpml,
  backendPollSource,
  type BackendSource,
} from "@/services/backend";
import { useAccountsStore } from "@/stores/accounts";
import { useThreadsStore } from "@/stores/threads";
import AppButton from "@/components/ui/AppButton.vue";
import AppIcon from "@/components/ui/AppIcon.vue";
import ToggleSwitch from "@/components/ui/ToggleSwitch.vue";

type SourceKind = "rss";

const { t } = useI18n();
const { ask } = useConfirm();
const accounts = useAccountsStore();
const threads = useThreadsStore();
const sources = ref<BackendSource[]>([]);
const kind = ref<SourceKind>("rss");
const name = ref("");
const identity = ref("");
// A source's identity colour is data on its way to the backend, not styling.
const color = ref("#4cd68a");
const pollMinutes = ref(30);
const notify = ref(false);
const status = ref("");
const busy = ref(false);
/** The create form opens on demand; the list and its Add row come first. */
const formOpen = ref(false);
const opmlInput = ref<HTMLInputElement | null>(null);

const identityPlaceholder = computed(() => t("settings.sources.feedUrlPlaceholder"));

function sourceConfig(source: BackendSource): Record<string, unknown> {
  try {
    return JSON.parse(source.config) as Record<string, unknown>;
  } catch {
    return {};
  }
}

async function load() {
  sources.value = await backendListSources();
}

async function createSource() {
  status.value = "";
  busy.value = true;
  try {
    await backendCreateSource({
      name: name.value,
      kind: kind.value,
      identity: identity.value,
      color: color.value,
      config: { notify: notify.value, poll_minutes: pollMinutes.value },
    });
    name.value = "";
    identity.value = "";
    await Promise.all([load(), accounts.load(), threads.load()]);
    status.value = t("settings.sources.created");
    formOpen.value = false;
  } catch (error) {
    status.value = String(error);
  } finally {
    busy.value = false;
  }
}

async function removeSource(id: string) {
  const confirmed = await ask({
    title: t("settings.sources.removeConfirm"),
    confirmLabel: t("settings.sources.removeConfirmAction"),
    tone: "danger",
  });
  if (!confirmed) return;
  try {
    await backendDeleteSource(id);
    await Promise.all([load(), accounts.load(), threads.load()]);
  } catch (error) {
    status.value = String(error);
  }
}

async function pollSource(id: string) {
  busy.value = true;
  try {
    const imported = await backendPollSource(id);
    status.value = t("settings.sources.polled", { count: imported }, imported);
    await Promise.all([accounts.load(), threads.load()]);
  } catch (error) {
    status.value = String(error);
  } finally {
    busy.value = false;
  }
}

async function importOpmlFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  busy.value = true;
  try {
    const result = await backendImportOpml({
      opml: await file.text(),
      pollMinutes: pollMinutes.value,
      notify: notify.value,
    });
    await Promise.all([load(), accounts.load(), threads.load()]);
    status.value = result.errors.length
      ? t("settings.sources.opmlImportedWithErrors", {
          created: result.created,
          skipped: result.skipped,
          errors: result.errors.length,
        })
      : t("settings.sources.opmlImported", {
          created: result.created,
          skipped: result.skipped,
        });
  } catch (error) {
    status.value = String(error);
  } finally {
    busy.value = false;
    input.value = "";
  }
}

onMounted(() => void load());
</script>

<template>
  <section class="sources">
    <header>
      <h3 class="settings-page-title">{{ $t('settings.sections.sources.label') }}</h3>
      <p>{{ $t("settings.sources.description") }}</p>
    </header>

    <!-- Feeds as a list with an Add row, like the other collections. -->
    <div class="setting-group">
      <div v-for="source in sources" :key="source.id" class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ source.display_name }}</span>
          <span class="setting-hint" :title="source.identity">
            {{ source.identity }} ·
            <template v-if="source.kind !== 'rss'">{{ $t("settings.sources.pushSourceUnsupported") }}</template>
            <template v-else>
              {{
                $t(
                  sourceConfig(source).notify
                    ? "settings.sources.pollWithNotify"
                    : "settings.sources.pollWithoutNotify",
                  { minutes: Number(sourceConfig(source).poll_minutes || 30) },
                )
              }}
            </template>
          </span>
        </div>
        <div class="card-actions">
          <AppButton
            v-if="source.kind === 'rss'"
            variant="secondary"
            size="sm"
            :disabled="busy"
            @click="pollSource(source.id)"
          >
            {{ $t("settings.sources.pollNow") }}
          </AppButton>
          <AppButton variant="ghost" size="sm" @click="removeSource(source.id)">
            {{ $t("common.remove") }}
          </AppButton>
        </div>
      </div>
      <div v-if="!sources.length" class="setting-row setting-row--single">
        <span class="setting-hint">{{ $t("settings.sources.empty") }}</span>
      </div>
      <div v-if="!formOpen" class="setting-row setting-row--single">
        <div class="card-actions">
          <AppButton variant="secondary" size="sm" @click="formOpen = true">
            <template #icon><AppIcon><path d="M8 3v10M3 8h10" /></AppIcon></template>
            {{ $t("settings.sources.newFeed") }}
          </AppButton>
          <AppButton variant="ghost" size="sm" :disabled="busy" @click="opmlInput?.click()">
            {{ $t("settings.sources.importOpml") }}
          </AppButton>
          <input
            ref="opmlInput"
            class="file-input"
            type="file"
            accept=".opml,.xml,text/xml,application/xml"
            @change="importOpmlFile"
          />
        </div>
      </div>
    </div>

    <form v-if="formOpen" class="source-form" @submit.prevent="createSource">
      <div class="form-grid">
        <label>
          <span>{{ $t("settings.sources.type") }}</span>
          <select v-model="kind">
            <option value="rss">{{ $t("settings.sources.typeRss") }}</option>
          </select>
        </label>
        <label>
          <span>{{ $t("common.name") }}</span>
          <input
            v-model="name"
            required
            :placeholder="$t('settings.sources.namePlaceholder')"
          />
        </label>
        <label class="span-full">
          <span>{{ $t("settings.sources.feedUrl") }}</span>
          <input v-model="identity" required :placeholder="identityPlaceholder" />
        </label>
        <label>
          <span>{{ $t("settings.sources.color") }}</span>
          <input v-model="color" type="color" :aria-label="$t('settings.sources.colorAriaLabel')" />
        </label>
        <label>
          <span>{{ $t("settings.sources.interval") }}</span>
          <select v-model.number="pollMinutes">
            <option :value="5">{{ $t("settings.sources.intervalMinutes", { count: 5 }, 5) }}</option>
            <option :value="15">{{ $t("settings.sources.intervalMinutes", { count: 15 }, 15) }}</option>
            <option :value="30">{{ $t("settings.sources.intervalMinutes", { count: 30 }, 30) }}</option>
            <option :value="60">{{ $t("settings.sources.intervalHours", { count: 1 }, 1) }}</option>
            <option :value="360">{{ $t("settings.sources.intervalHours", { count: 6 }, 6) }}</option>
          </select>
        </label>
      </div>
      <div class="form-footer">
        <ToggleSwitch v-model="notify">{{ $t("settings.sources.notify") }}</ToggleSwitch>
        <span class="form-footer__spacer" />
        <AppButton variant="ghost" size="sm" @click="formOpen = false">{{ $t("common.cancel") }}</AppButton>
        <AppButton variant="primary" size="sm" type="submit" :disabled="busy">
          {{ $t("settings.sources.createSource") }}
        </AppButton>
      </div>
    </form>

    <p v-if="status" class="status" role="status">{{ status }}</p>

  </section>
</template>

<style scoped>
.sources {
  display: grid;
  gap: 18px;
}
.sources h3,
.sources p {
  margin: 0;
}
.sources header p,
.empty {
  margin-top: 4px;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}
.source-form {
  display: grid;
  gap: 12px;
  padding: 14px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
}
.form-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}
.span-full {
  grid-column: 1 / -1;
}
.form-footer {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
}
label {
  display: grid;
  gap: 5px;
  min-width: 0;
}
label span {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
}
input,
select {
  min-width: 0;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
}
input[type="color"] {
  width: 100%;
  min-height: 35px;
  padding: 3px;
}
.status {
  color: var(--accent-green);
}
</style>

<style scoped>
.file-input { display: none; }
.form-footer__spacer { flex: 1; }
.card-actions { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-sm); }
</style>

<style scoped src="./settingsRows.css"></style>
