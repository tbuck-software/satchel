<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { backendErrorKey, translateBackendError } from '@/utils/backendErrors';

export type PgpState =
  | 'encrypted_no_key'
  | 'key_available_locked'
  | 'decrypting'
  | 'decrypted'
  | 'signature_valid'
  | 'signature_partial'
  | 'signature_identity_mismatch'
  | 'signature_invalid'
  | 'signature_unknown_key'
  | 'error';

const props = defineProps<{
  state: PgpState;
  errorDetail?: string;
  signerName?: string;
  signerFingerprint?: string;
  /** The mailbox the signature is measured against. */
  fromAddress?: string;
}>();

const emit = defineEmits<{
  unlock: [];
  retry: [];
  importKey: [];
  viewDetails: [];
  dismiss: [];
}>();

const { t } = useI18n();

const localizedErrorDetail = computed(
  () => translateBackendError(props.errorDetail) ?? t('mail.pgp.decryptFailed'),
);

// The same structure checked again still does not tie its signature to the
// content; only a new copy of the message could.
const retryable = computed(
  () => backendErrorKey(props.errorDetail) !== 'backend.errors.signedMessageMalformed',
);

interface StateConfig {
  title: string;
  description: string;
  primaryLabel?: string;
  primaryAction?: () => void;
  secondaryLabel?: string;
  secondaryAction?: () => void;
  tone: 'neutral' | 'success' | 'warning' | 'danger';
  animate?: boolean;
}

// State is a closed TypeScript union; the switch covers every member.
// oxlint-disable-next-line vue/return-in-computed-property
const config = computed<StateConfig>(() => {
  switch (props.state) {
    case 'encrypted_no_key':
      return {
        title: t('mail.pgp.noKey'),
        description: t('mail.pgp.noKeyDescription'),
        primaryLabel: t('mail.pgp.importKey'),
        primaryAction: () => emit('importKey'),
        secondaryLabel: t('mail.pgp.details'),
        secondaryAction: () => emit('viewDetails'),
        tone: 'warning',
      };
    case 'key_available_locked':
      return {
        title: t('mail.pgp.unlockTitle'),
        description: t('mail.pgp.unlockTitleDescription'),
        primaryLabel: t('mail.pgp.unlock'),
        primaryAction: () => emit('unlock'),
        secondaryLabel: t('mail.pgp.details'),
        secondaryAction: () => emit('viewDetails'),
        tone: 'neutral',
      };
    case 'decrypting':
      return {
        title: t('mail.pgp.decrypting'),
        description: t('mail.pgp.decryptingDescription'),
        tone: 'neutral',
        animate: true,
      };
    case 'decrypted':
      return {
        title: t('mail.pgp.decrypted'),
        description: t('mail.pgp.decryptedDescription'),
        secondaryLabel: t('mail.pgp.details'),
        secondaryAction: () => emit('viewDetails'),
        tone: 'success',
      };
    case 'signature_partial':
      return { title: t('mail.pgp.signaturePartial'), description: t('mail.pgp.signaturePartialDescription'), tone: 'warning' };
    case 'signature_valid':
      return {
        title: t('mail.pgp.signatureValid'),
        description: props.signerName
          ? t('mail.pgp.signedBy', { name: props.signerName })
          : t('mail.pgp.signatureValidDescription'),
        secondaryLabel: t('mail.pgp.details'),
        secondaryAction: () => emit('viewDetails'),
        tone: 'success',
      };
    case 'signature_identity_mismatch':
      return {
        title: t('mail.pgp.signatureIdentityMismatch'),
        description: t('mail.pgp.signatureIdentityMismatchDescription', {
          signer: props.signerName || props.signerFingerprint || t('mail.pgp.unknownSigner'),
          from: props.fromAddress || t('mail.pgp.unknownSigner'),
        }),
        secondaryLabel: t('mail.pgp.details'),
        secondaryAction: () => emit('viewDetails'),
        tone: 'warning',
      };
    case 'signature_invalid':
      return {
        title: t('mail.pgp.signatureInvalid'),
        description: t('mail.pgp.signatureInvalidDescription'),
        primaryLabel: t('mail.pgp.recheck'),
        primaryAction: () => emit('retry'),
        secondaryLabel: t('mail.pgp.details'),
        secondaryAction: () => emit('viewDetails'),
        tone: 'danger',
      };
    case 'signature_unknown_key':
      return {
        title: t('mail.pgp.signatureUnknownKey'),
        description: t('mail.pgp.signatureUnknownKeyDescription'),
        primaryLabel: t('mail.pgp.importKey'),
        primaryAction: () => emit('importKey'),
        secondaryLabel: t('mail.pgp.details'),
        secondaryAction: () => emit('viewDetails'),
        tone: 'warning',
      };
    case 'error':
      return {
        title: t('mail.pgp.errorTitle'),
        description: localizedErrorDetail.value,
        ...(retryable.value
          ? { primaryLabel: t('common.retry'), primaryAction: () => emit('retry') }
          : {}),
        secondaryLabel: t('common.close'),
        secondaryAction: () => emit('dismiss'),
        tone: 'danger',
      };
  }
});
</script>

<template>
  <div
    class="pgp-card"
    :class="[`tone-${config.tone}`, { 'is-animating': config.animate }]"
    role="status"
    :aria-label="config.title"
  >
    <div class="pgp-accent" />

    <div class="pgp-icon-area">
      <!-- encrypted_no_key: shield with keyhole -->
      <svg v-if="state === 'encrypted_no_key'" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M10 1.5L3 5v4.5c0 4.7 3 8.5 7 10 4-1.5 7-5.3 7-10V5l-7-3.5Z" />
        <circle cx="10" cy="9" r="1.5" />
        <path d="M10 10.5v2" />
      </svg>

      <!-- key_available_locked: key -->
      <svg v-else-if="state === 'key_available_locked'" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="7.5" cy="12.5" r="4" />
        <path d="M10.5 9.5L17 3" />
        <path d="M14 3h3v3" />
        <path d="M13 7l2 -2" />
      </svg>

      <!-- decrypting: rotating spinner -->
      <svg v-else-if="state === 'decrypting'" class="spinner" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
        <path d="M10 2a8 8 0 0 1 8 8" />
      </svg>

      <!-- decrypted: open shield with check -->
      <svg v-else-if="state === 'decrypted'" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M10 1.5L3 5v4.5c0 4.7 3 8.5 7 10 4-1.5 7-5.3 7-10V5l-7-3.5Z" />
        <path d="M7.5 10l2 2 3.5-4" />
      </svg>

      <!-- signature_valid: badge with check -->
      <svg v-else-if="state === 'signature_valid'" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M10 1l2.2 3.2 3.8.8-2 3.4.5 3.9L10 10.6l-4.5 1.7.5-3.9L4 5l3.8-.8L10 1Z" />
        <path d="M7.5 9l2 2 3-3.5" />
      </svg>

      <!-- signature_invalid: warning triangle -->
      <svg v-else-if="state === 'signature_partial' || state === 'signature_invalid' || state === 'signature_unknown_key' || state === 'signature_identity_mismatch'" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M10 2L1.5 17h17L10 2Z" />
        <path d="M10 8v4" />
        <circle cx="10" cy="14.5" r="0.5" fill="currentColor" stroke="none" />
      </svg>

      <!-- error: circle with X -->
      <svg v-else-if="state === 'error'" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="10" cy="10" r="8" />
        <path d="M7.5 7.5l5 5M12.5 7.5l-5 5" />
      </svg>
    </div>

    <div class="pgp-content">
      <div class="pgp-text">
        <span class="pgp-title">{{ config.title }}</span>
        <span class="pgp-description">{{ config.description }}</span>
        <span v-if="signerFingerprint && (state === 'signature_valid' || state === 'signature_invalid')" class="pgp-fingerprint">
          {{ signerFingerprint }}
        </span>
      </div>

      <div v-if="config.primaryLabel || config.secondaryLabel" class="pgp-actions">
        <AppButton
          v-if="config.primaryLabel"
          class="pgp-btn pgp-btn-primary"
          @click="config.primaryAction?.()"
          variant="primary"
          size="sm"
        >
          {{ config.primaryLabel }}
        </AppButton>
        <AppButton
          v-if="config.secondaryLabel"
          class="pgp-btn pgp-btn-secondary"
          @click="config.secondaryAction?.()"
          variant="secondary"
          size="sm"
        >
          {{ config.secondaryLabel }}
        </AppButton>
      </div>
    </div>
  </div>
</template>

<style scoped>
.pgp-card {
  position: relative;
  display: flex;
  align-items: flex-start;
  gap: var(--space-md);
  padding: var(--space-lg) var(--space-lg) var(--space-lg) calc(var(--space-lg) + 3px);
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  overflow: hidden;
  transition: background var(--duration-normal) var(--ease-out);
}

.pgp-card:hover {
  background: var(--bg-hover);
}

/* Accent left border — color set per tone */
.pgp-accent {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  border-radius: 0 2px 2px 0;
  transition: background var(--duration-normal) var(--ease-out);
}

/* ─── Tone colors ─── */
.tone-neutral .pgp-accent { background: var(--accent-primary); }
.tone-success .pgp-accent { background: var(--accent-green); }
.tone-warning .pgp-accent { background: var(--accent-amber); }
.tone-danger  .pgp-accent { background: var(--accent-red); }

.tone-neutral .pgp-icon-area { color: var(--accent-primary); }
.tone-success .pgp-icon-area { color: var(--accent-green); }
.tone-warning .pgp-icon-area { color: var(--accent-amber); }
.tone-danger  .pgp-icon-area { color: var(--accent-red); }

/* ─── Icon area ─── */
.pgp-icon-area {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, currentColor 8%, transparent);
  transition: background var(--duration-normal) var(--ease-out);
}

.pgp-card:hover .pgp-icon-area {
  background: color-mix(in srgb, currentColor 12%, transparent);
}

/* Spinner animation */
@keyframes pgp-spin {
  to { transform: rotate(360deg); }
}

.spinner {
  animation: pgp-spin 0.8s linear infinite;
}

/* ─── Content ─── */
.pgp-content {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-md);
}

.pgp-text {
  display: flex;
  flex-direction: column;
  gap: var(--space-2xs);
}

.pgp-title {
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  line-height: 1.3;
  letter-spacing: -0.01em;
}

.pgp-description {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  line-height: 1.5;
}

.pgp-fingerprint {
  font-family: var(--font-mono);
  font-size: var(--font-size-2xs);
  color: var(--text-tertiary);
  letter-spacing: 0.04em;
  margin-top: var(--space-2xs);
}

/* ─── Animating (decrypting) state ─── */
.is-animating .pgp-title {
  animation: pgp-pulse 2s ease-in-out infinite;
}

@keyframes pgp-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.55; }
}

/* ─── Actions ─── */
.pgp-actions {
  display: flex;
  gap: var(--space-sm);
}

/* ─── Responsive ─── */
@media (max-width: 900px) {
  .pgp-card {
    flex-direction: column;
    gap: var(--space-sm);
  }

  .pgp-icon-area {
    width: 32px;
    height: 32px;
  }

  .pgp-icon-area svg {
    width: 18px;
    height: 18px;
  }

  .pgp-actions {
    flex-direction: column;
  }
}
</style>
