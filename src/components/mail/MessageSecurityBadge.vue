<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { PgpIndicatorState, SmimeIndicatorState } from '@/types/mail';
import type { AuthenticationTone } from '@/utils/messageAuthentication';

/**
 * One glyph next to the sender name. Colour is the signal; the sheet behind it
 * carries the breakdown. Worst state wins, S/MIME outranks header auth.
 */
const props = withDefaults(defineProps<{
  smimeState?: SmimeIndicatorState | null;
  pgpState?: PgpIndicatorState | null;
  authentication?: AuthenticationTone;
  /** The mailbox the signature is measured against. */
  fromAddress?: string;
  smimeSigner?: string | null;
  pgpSigner?: string | null;
  pgpSignerFingerprint?: string | null;
}>(), {
  smimeState: null,
  pgpState: null,
  authentication: 'none',
  fromAddress: '',
  smimeSigner: null,
  pgpSigner: null,
  pgpSignerFingerprint: null,
});

const emit = defineEmits<{ showDetails: [] }>();

const { t } = useI18n();

type Glyph = 'seal' | 'lock' | 'shield';
type Mark = 'check' | 'cross' | 'question';
type Tone = 'success' | 'warning' | 'danger' | 'muted';

const badge = computed<{ glyph: Glyph; mark: Mark; tone: Tone; label: string } | null>(() => {
  if (props.smimeState === 'signature_invalid') {
    return { glyph: 'seal', mark: 'cross', tone: 'danger', label: t('mail.smime.shortSignatureInvalid') };
  }
  if (props.authentication === 'fail') {
    return { glyph: 'shield', mark: 'cross', tone: 'danger', label: t('mail.authentication.summaryFailed') };
  }
  // A signature that is correct but not the sender's must not read like one
  // that is, so it gets its own tone and names both mailboxes.
  if (props.smimeState === 'signature_identity_mismatch') {
    return {
      glyph: 'seal',
      mark: 'cross',
      tone: 'warning',
      label: t('mail.smime.shortSignatureIdentityMismatch', {
        signer: props.smimeSigner || t('mail.smime.unknownSigner'),
        from: props.fromAddress,
      }),
    };
  }
  if (props.pgpState === 'signature_identity_mismatch') {
    return {
      glyph: 'seal',
      mark: 'cross',
      tone: 'warning',
      label: t('mail.pgp.shortSignatureIdentityMismatch', {
        signer: props.pgpSigner || t('mail.pgp.unknownSigner'),
        from: props.fromAddress,
        fingerprint: props.pgpSignerFingerprint ?? '',
      }),
    };
  }
  if (props.smimeState === 'signature_partial' || props.pgpState === 'signature_partial') {
    return { glyph: 'seal', mark: 'question', tone: 'warning', label: t('mail.smime.signaturePartial') };
  }
  if (props.smimeState === 'signature_valid') {
    return { glyph: 'seal', mark: 'check', tone: 'success', label: t('mail.smime.shortSignature') };
  }
  if (props.smimeState === 'signature_unchained') {
    return { glyph: 'seal', mark: 'question', tone: 'warning', label: t('mail.smime.shortSignatureUnchained') };
  }
  if (props.smimeState === 'decrypted') {
    return { glyph: 'lock', mark: 'check', tone: 'success', label: t('mail.smime.shortDecrypted') };
  }
  if (props.smimeState === 'signature_unverified') {
    return { glyph: 'seal', mark: 'question', tone: 'warning', label: t('mail.smime.shortSignatureDetected') };
  }
  if (props.authentication === 'warn') {
    return { glyph: 'shield', mark: 'question', tone: 'warning', label: t('mail.authentication.summaryWarning') };
  }
  if (props.authentication === 'pass') {
    return { glyph: 'shield', mark: 'check', tone: 'muted', label: t('mail.authentication.summaryPassed') };
  }
  return null;
});
</script>

<template>
  <button
    v-if="badge"
    type="button"
    class="security-badge"
    :class="`security-badge--${badge.tone}`"
    aria-haspopup="dialog"
    :aria-label="t('mail.smime.showDetails', { label: badge.label })"
    :title="badge.label"
    @click.stop="emit('showDetails')"
  >
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path v-if="badge.glyph === 'seal'" d="M10 1.5 12 4l3.2.3-.3 3.2 2 2.5-2 2.5.3 3.2-3.2.3-2 2.5-2-2.5-3.2-.3.3-3.2-2-2.5 2-2.5-.3-3.2L8 4l2-2.5Z" />
      <template v-else-if="badge.glyph === 'lock'">
        <rect x="3" y="8" width="14" height="9" rx="2" />
        <path d="M6 8V5.8a4 4 0 0 1 8 0V8" />
      </template>
      <path v-else d="M10 2.1 16 4.4V9c0 3.8-2.3 6.7-6 8.2C6.3 15.7 4 12.8 4 9V4.4z" />
      <path v-if="badge.mark === 'check'" :d="badge.glyph === 'lock' ? 'm7.5 12.4 1.7 1.7 3.5-3.6' : 'm6.8 10 2 2 4.3-4.5'" />
      <path v-else-if="badge.mark === 'cross'" d="m7.2 7.2 5.6 5.6m0-5.6-5.6 5.6" />
      <path v-else d="M10 7.1v4m0 2.2h.01" />
    </svg>
  </button>
</template>

<style scoped>
.security-badge {
  display: inline-grid;
  width: 20px;
  height: 20px;
  flex: 0 0 auto;
  place-items: center;
  border: 0;
  border-radius: var(--radius-full);
  padding: 0;
  background: transparent;
  color: var(--badge-color);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.security-badge svg {
  width: 15px;
  height: 15px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.7;
}

.security-badge:hover,
.security-badge:focus-visible {
  background: color-mix(in srgb, var(--badge-color) 14%, transparent);
}

.security-badge--success { --badge-color: var(--accent-green); }
.security-badge--warning { --badge-color: var(--accent-amber); }
.security-badge--danger { --badge-color: var(--accent-red); }
.security-badge--muted { --badge-color: var(--text-tertiary); }
</style>
