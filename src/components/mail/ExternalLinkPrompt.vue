<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  linkRegistrableDomain,
  type ExternalLinkAssessment,
  type ExternalLinkRisk,
} from '@/utils/externalLinks';
import { useSettingsStore } from '@/stores/settings';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

/**
 * The interactive core of the external-link confirmation, shared by the
 * desktop alert dialog and the phone bottom sheet. Trust persists per
 * registrable domain here so both containers behave identically.
 */
const props = defineProps<{
  assessment: ExternalLinkAssessment;
}>();

const emit = defineEmits<{
  confirm: [];
  cancel: [];
}>();

const { t } = useI18n();
const settings = useSettingsStore();
const confirmButton = ref<HTMLButtonElement | null>(null);
const trustHost = ref(false);

// A switch rather than a lookup table: the subset builder only sees literal
// keys, and a table indexed by risk would hide them behind a variable.
function riskLabel(risk: ExternalLinkRisk): string {
  switch (risk) {
    case 'display_host_mismatch':
      return t('mail.externalLink.risk.displayHostMismatch');
    case 'insecure_http':
      return t('mail.externalLink.risk.insecureHttp');
    case 'embedded_credentials':
      return t('mail.externalLink.risk.embeddedCredentials');
    case 'ip_address':
      return t('mail.externalLink.risk.ipAddress');
    case 'punycode_hostname':
      return t('mail.externalLink.risk.punycodeHostname');
    case 'unusual_port':
      return t('mail.externalLink.risk.unusualPort');
    case 'local_hostname':
      return t('mail.externalLink.risk.localHostname');
  }
}

const reasons = computed(() => props.assessment.risks.map(riskLabel));
/**
 * Tone follows the same line the assessment draws for desktop: plain HTTP is
 * listed but does not turn the prompt into a warning, so a mobile sheet for
 * an http link reads like the same destination on the desktop.
 */
const risky = computed(() => props.assessment.requiresConfirmation);
/** What trusting stores; the label names it, not the subdomain at hand. */
const trustDomain = computed(() => linkRegistrableDomain(props.assessment.hostname));
// Trust only skips the plain question, so a warning offers none.
const offerTrust = computed(() => !risky.value && trustDomain.value !== '');
/** A display name equal to the destination carries no information. */
const displayMismatch = computed(
  () => props.assessment.displayedHostname !== undefined
    && props.assessment.displayedHostname !== props.assessment.hostname,
);

watch(
  () => props.assessment,
  () => {
    trustHost.value = false;
    void nextTick(() => confirmButton.value?.focus());
  },
  { immediate: true },
);

function confirm() {
  if (trustHost.value && offerTrust.value) {
    settings.trustLinkHost(props.assessment.hostname);
  }
  emit('confirm');
}
</script>

<template>
  <div class="link-prompt">
    <div :class="['link-prompt-icon', { 'link-prompt-icon--risky': risky }]" aria-hidden="true">↗</div>
    <p :class="['link-prompt-kicker', { 'link-prompt-kicker--risky': risky }]">
      {{ risky ? $t('mail.externalLink.kickerRisky') : $t('mail.externalLink.kicker') }}
    </p>
    <h2 class="link-prompt-title">
      {{ risky ? $t('mail.externalLink.titleRisky') : $t('mail.externalLink.title') }}
    </h2>
    <p class="link-prompt-description">
      {{
        risky
          ? $t('mail.externalLink.descriptionRisky')
          : $t('mail.externalLink.description')
      }}
    </p>

    <dl class="link-targets">
      <template v-if="displayMismatch">
        <dt>{{ $t('mail.externalLink.displayed') }}</dt>
        <dd>{{ assessment.displayedHostname }}</dd>
      </template>
      <dt>{{ $t('mail.externalLink.actualTarget') }}</dt>
      <dd class="actual-target">{{ assessment.hostname }}</dd>
    </dl>

    <ul v-if="reasons.length > 0" class="link-risks">
      <li v-for="reason in reasons" :key="reason">{{ reason }}</li>
    </ul>

    <p class="link-address">{{ assessment.href }}</p>

    <ToggleSwitch v-if="offerTrust" v-model="trustHost" class="link-trust-toggle">
      {{ $t('mail.externalLink.trustDomain', { domain: trustDomain }) }}
    </ToggleSwitch>

    <div class="link-prompt-actions">
      <button type="button" class="dialog-button dialog-button--cancel" @click="emit('cancel')">
        {{ $t('common.cancel') }}
      </button>
      <button
        ref="confirmButton"
        type="button"
        :class="['dialog-button', risky ? 'dialog-button--confirm-risky' : 'dialog-button--confirm']"
        @click="confirm"
      >
        {{ risky ? $t('mail.externalLink.openAnyway') : $t('mail.externalLink.open') }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.link-prompt {
  display: grid;
  justify-items: start;
}

.link-prompt-icon {
  display: grid;
  place-items: center;
  width: 42px;
  height: 42px;
  margin-bottom: 12px;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--accent-primary) 14%, transparent);
  color: var(--accent-primary);
  font-size: 21px;
  font-weight: var(--weight-bold);
}

.link-prompt-icon--risky {
  background: color-mix(in srgb, var(--accent-amber) 16%, transparent);
  color: var(--accent-amber);
}

.link-prompt-kicker {
  margin: 0 0 4px;
  color: var(--accent-primary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-bold);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.link-prompt-kicker--risky {
  color: var(--accent-amber);
}

.link-prompt-title {
  margin: 0;
  color: var(--text-primary);
  font-size: 19px;
  letter-spacing: -0.02em;
}

.link-prompt-description {
  margin: 6px 0 16px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  line-height: 1.5;
}

.link-targets {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 7px 12px;
  width: 100%;
  margin: 0;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-inset) 70%, transparent);
  padding: 11px 12px;
  font-size: var(--font-size-sm);
}

.link-targets dt { color: var(--text-tertiary); }
.link-targets dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
  color: var(--text-secondary);
  font-family: var(--font-mono);
}
.link-targets .actual-target { color: var(--text-primary); }

.link-risks {
  margin: 13px 0 0;
  padding-left: 19px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  line-height: 1.5;
}

.link-address {
  max-height: 64px;
  margin: 13px 0 15px;
  overflow: auto;
  color: var(--text-tertiary);
  font-family: var(--font-mono);
  font-size: var(--font-size-2xs);
  overflow-wrap: anywhere;
}

.link-trust-toggle {
  margin-bottom: 16px;
}

.link-trust-toggle :deep(.toggle-switch__label) {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  overflow-wrap: anywhere;
}

.link-prompt-actions {
  display: flex;
  width: 100%;
  gap: 9px;
}

.dialog-button {
  flex: 1;
  min-height: 44px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  padding: 0 14px;
  font: inherit;
  font-size: var(--font-size-sm);
  font-weight: var(--weight-strong);
  cursor: pointer;
}

.dialog-button:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

.dialog-button--cancel {
  border-color: var(--border-color-muted);
  background: var(--bg-raised);
  color: var(--text-secondary);
}

.dialog-button--confirm {
  background: var(--accent-primary);
  color: var(--text-on-accent);
}

.dialog-button--confirm-risky {
  background: var(--accent-amber);
  color: var(--text-on-amber);
}
</style>
