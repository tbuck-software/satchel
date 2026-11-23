<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { BadgeCheck, Lock, TriangleAlert, type IconNode } from 'lucide';
import type { SmimeMessageState } from '@/types/mail';
import { translateBackendError } from '@/utils/backendErrors';
import AppIcon from '@/components/ui/AppIcon.vue';

const { t } = useI18n();

const props = defineProps<{
  state: SmimeMessageState;
  errorDetail?: string;
  signerEmail?: string;
  signerCn?: string;
  /** The mailbox the signature is measured against. */
  fromAddress?: string;
}>();

const emit = defineEmits<{
  unlock: [];
  importCert: [];
  dismiss: [];
}>();

// Only a message key is shown. The engine's own wording is neither translated
// nor written for readers, so the card's text stands in for it.
const errorDetail = computed(() => translateBackendError(props.errorDetail) || undefined);

// State is a closed TypeScript union; the switch covers every member.
// oxlint-disable-next-line vue/return-in-computed-property
const config = computed(() => {
  switch (props.state) {
    case 'encrypted_no_cert':
      return {
        title: t('mail.smime.encryptedNoCert'),
        description: t('mail.smime.encryptedNoCertDescription'),
        tone: 'warning',
        primary: t('mail.smime.importCert'),
        action: () => emit('importCert'),
      };
    case 'cert_available_locked':
      return {
        title: t('mail.smime.certLocked'),
        description: t('mail.smime.certLockedDescription'),
        tone: 'neutral',
        primary: t('mail.smime.unlock'),
        action: () => emit('unlock'),
      };
    case 'decrypting':
      return {
        title: t('mail.smime.decrypting'),
        description: t('mail.smime.decryptingDescription'),
        tone: 'neutral',
      };
    case 'decrypted':
      return {
        title: t('mail.smime.decrypted'),
        description: t('mail.smime.decryptedDescription'),
        tone: 'success',
      };
    case 'signature_partial':
      return { title: t('mail.smime.signaturePartial'), description: t('mail.smime.signaturePartialDescription'), tone: 'warning' };
    case 'signature_valid':
      return {
        title: t('mail.smime.signatureValid'),
        description: props.signerCn || props.signerEmail
          ? t('mail.smime.signedBy', { name: props.signerCn || props.signerEmail })
          : t('mail.smime.signatureValidDescription'),
        tone: 'success',
      };
    case 'signature_unchained':
      return {
        title: t('mail.smime.signatureUnchained'),
        description: t('mail.smime.signatureUnchainedDescription'),
        tone: 'warning',
      };
    case 'signature_identity_mismatch':
      return {
        title: t('mail.smime.signatureIdentityMismatch'),
        description: t('mail.smime.signatureIdentityMismatchDescription', {
          signer: props.signerEmail || props.signerCn || t('mail.smime.unknownSigner'),
          from: props.fromAddress || t('mail.smime.unknownSigner'),
        }),
        tone: 'warning',
      };
    case 'signature_invalid':
      return {
        title: t('mail.smime.signatureInvalid'),
        description: errorDetail.value || t('mail.smime.signatureInvalidDescription'),
        tone: 'danger',
      };
    case 'error':
      return {
        title: t('mail.smime.errorTitle'),
        description: errorDetail.value || t('mail.smime.messageFailed'),
        tone: 'danger',
        primary: t('mail.smime.close'),
        action: () => emit('dismiss'),
      };
  }
});

const stateIcon = computed<IconNode>(() => {
  if (props.state === 'signature_partial') return TriangleAlert;
  if (props.state === 'signature_valid') return BadgeCheck;
  return Lock;
});
</script>

<template>
  <div class="smime-card" :class="`tone-${config.tone}`" role="status">
    <div class="accent" />
    <div class="icon" aria-hidden="true">
      <AppIcon :icon="stateIcon" size="lg" />
    </div>
    <div class="content">
      <strong>{{ config.title }}</strong>
      <span>{{ config.description }}</span>
      <code v-if="signerEmail">{{ signerEmail }}</code>
    </div>
    <AppButton v-if="config.primary" class="action" @click="config.action?.()" variant="primary" size="sm">
      {{ config.primary }}
    </AppButton>
  </div>
</template>

<style scoped>
.smime-card {
  position: relative;
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 14px 14px 14px 17px;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  overflow: hidden;
}
.accent {
  position: absolute;
  inset: 0 auto 0 0;
  width: 3px;
}
.tone-neutral .accent { background: var(--accent-primary); }
.tone-success .accent { background: var(--accent-green); }
.tone-warning .accent { background: var(--accent-amber); }
.tone-danger .accent { background: var(--accent-red); }
.tone-neutral .icon { color: var(--accent-primary); }
.tone-success .icon { color: var(--accent-green); }
.tone-warning .icon { color: var(--accent-amber); }
.tone-danger .icon { color: var(--accent-red); }
.icon {
  flex: 0 0 auto;
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, currentColor 12%, transparent);
}
.content {
  display: grid;
  gap: 3px;
  min-width: 0;
}
.content strong {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}
.content span,
.content code {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}
.action { margin-left:auto; flex:0 0 auto; }
</style>
