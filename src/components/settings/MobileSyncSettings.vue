<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import {
  backendBackgroundSyncStatus,
  backendListAccounts,
  backendOpenBackgroundSyncSettings,
  type BackendBackgroundSyncStatus,
} from "@/services/backend";
import AppButton from "@/components/ui/AppButton.vue";

const { locale, t } = useI18n();
const error = ref("");
const backgroundStatus = ref<BackendBackgroundSyncStatus | null>(null);
const lastSyncAt = ref<string | null>(null);

const lastSyncLabel = computed(() => {
  if (!lastSyncAt.value) return t("settings.mobileSync.neverSynced");
  const date = new Date(lastSyncAt.value);
  if (Number.isNaN(date.getTime())) return t("settings.mobileSync.unknownTime");
  return date.toLocaleString(locale.value, { dateStyle: "medium", timeStyle: "short" });
});

async function refreshBackgroundStatus() {
  try {
    const [nativeStatus, accounts] = await Promise.all([
      backendBackgroundSyncStatus(),
      backendListAccounts(),
    ]);
    backgroundStatus.value = nativeStatus;
    lastSyncAt.value = accounts
      .filter((account) => account.kind === "email" && account.last_sync_at)
      .map((account) => account.last_sync_at as string)
      .sort((left, right) => Date.parse(right) - Date.parse(left))[0] ?? null;
    error.value = "";
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  }
}

async function openAndroidSettings() {
  try {
    await backendOpenBackgroundSyncSettings();
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  }
}

function refreshOnFocus() {
  void refreshBackgroundStatus();
}

onMounted(async () => {
  await refreshBackgroundStatus();
  window.addEventListener("focus", refreshOnFocus);
});

onBeforeUnmount(() => window.removeEventListener("focus", refreshOnFocus));
</script>
<template>
  <section class="mobile-sync">
    <header>
      <h2>{{ t("settings.mobileSync.androidTitle") }}</h2>
      <p>{{ t("settings.mobileSync.androidDescription") }}</p>
    </header>
    <div class="health-card">
      <div class="health-row">
        <span>{{ t("settings.mobileSync.service") }}</span>
        <strong :class="backgroundStatus?.running ? 'healthy' : 'warning'">
          {{
            backgroundStatus?.running
              ? t("settings.mobileSync.serviceActive")
              : t("settings.mobileSync.serviceInactive")
          }}
        </strong>
      </div>
      <div class="health-row">
        <span>{{ t("settings.mobileSync.lastSync") }}</span>
        <strong>{{ lastSyncLabel }}</strong>
      </div>
      <div class="health-row">
        <span>{{ t("settings.mobileSync.battery") }}</span>
        <strong :class="backgroundStatus?.batteryOptimizationExempt ? 'healthy' : 'warning'">
          {{
            backgroundStatus?.batteryOptimizationExempt
              ? t("settings.mobileSync.batteryUnrestricted")
              : t("settings.mobileSync.batteryOptimized")
          }}
        </strong>
      </div>
    </div>
    <p v-if="backgroundStatus && !backgroundStatus.running" class="message failure">
      {{ t("settings.mobileSync.serviceDownNote") }}
    </p>
    <p v-if="backgroundStatus && !backgroundStatus.batteryOptimizationExempt" class="message warning-note">
      {{ t("settings.mobileSync.batteryNote") }}
    </p>
    <p v-if="error" class="message failure">{{ error }}</p>
    <div class="actions">
      <AppButton variant="primary" @click="openAndroidSettings">
        {{ t("settings.mobileSync.openAndroidSettings") }}
      </AppButton>
      <AppButton variant="soft" @click="refreshBackgroundStatus">
        {{ t("settings.mobileSync.refreshStatus") }}
      </AppButton>
    </div>
  </section>
</template>
<style scoped>
.mobile-sync {
  max-width: 620px;
}
header {
  margin-bottom: 24px;
}
h2 {
  margin: 0 0 8px;
  font-size: 20px;
}
p {
  color: var(--text-secondary);
  line-height: 1.5;
}
.health-card {
  overflow: hidden;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-secondary);
}
.health-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
  padding: 14px 16px;
}
.health-row + .health-row {
  border-top: 1px solid var(--border-color);
}
.health-row span {
  color: var(--text-secondary);
}
.health-row strong {
  color: var(--text-primary);
  text-align: right;
}
.health-row .healthy {
  color: var(--accent-green);
}
.health-row .warning {
  color: var(--accent-amber);
}
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  padding-top: 4px;
}
.message {
  margin: 0;
  padding: 10px 12px;
  border-radius: var(--radius-sm);
  font-size: var(--font-size-sm);
}
.failure {
  color: var(--accent-red);
  background: color-mix(in srgb, var(--accent-red) 12%, transparent);
}
.warning-note {
  color: var(--accent-amber);
  background: color-mix(in srgb, var(--accent-amber) 12%, transparent);
}
</style>
