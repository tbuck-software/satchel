<script setup lang="ts">
import IconButton from '@/components/ui/IconButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  BadgeCheck, BadgeQuestionMark, BadgeX, Lock, ShieldCheck, ShieldQuestionMark, ShieldX,
  type IconNode,
} from 'lucide';
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

const ICONS: Record<Glyph, Record<Mark, IconNode>> = {
  seal: { check: BadgeCheck, cross: BadgeX, question: BadgeQuestionMark },
  lock: { check: Lock, cross: Lock, question: Lock },
  shield: { check: ShieldCheck, cross: ShieldX, question: ShieldQuestionMark },
};
const badgeIcon = computed(() => (badge.value ? ICONS[badge.value.glyph][badge.value.mark] : null));
</script>

<template>
  <IconButton
    v-if="badge"
    class="security-badge"
    :class="`security-badge--${badge.tone}`"
    aria-haspopup="dialog"
    :aria-label="t('mail.smime.showDetails', { label: badge.label })"
    :title="badge.label"
    @click.stop="emit('showDetails')"
  >
    <AppIcon :icon="badgeIcon" />
  </IconButton>
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

.security-badge:hover,
.security-badge:focus-visible {
  background: color-mix(in srgb, var(--badge-color) 14%, transparent);
}

.security-badge--success { --badge-color: var(--accent-green); }
.security-badge--warning { --badge-color: var(--accent-amber); }
.security-badge--danger { --badge-color: var(--accent-red); }
.security-badge--muted { --badge-color: var(--text-tertiary); }
</style>
