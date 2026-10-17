<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import type { Participant, SmimeDisplayState } from '@/types/mail';
import {
  type AuthenticationVerdictKey,
  authenticationVerdicts,
  hasAuthenticationResults as hasVerdicts,
  summarizeAuthentication,
} from '@/utils/messageAuthentication';
import { useAccountsStore } from '@/stores/accounts';
import { useContactsStore } from '@/stores/contacts';
import { participantDisplayName } from '@/utils/mailIdentity';
import Avatar from '@/components/common/Avatar.vue';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import IconButton from '@/components/ui/IconButton.vue';
import PersonProfileSheet from '@/components/contacts/PersonProfileSheet.vue';

const props = withDefaults(defineProps<{
  from: Participant;
  to: Participant[];
  cc?: Participant[];
  subject: string;
  timestamp: string;
  replyTo?: string;
  authSpf?: string | null;
  authDkim?: string | null;
  authDmarc?: string | null;
  smimeState?: SmimeDisplayState | null;
  smimeSignerEmail?: string;
  smimeSignerCn?: string;
}>(), {
  cc: () => [],
  replyTo: '',
  authSpf: null,
  authDkim: null,
  authDmarc: null,
  smimeState: null,
  smimeSignerEmail: '',
  smimeSignerCn: '',
});

const accounts = useAccountsStore();
const contacts = useContactsStore();
const { locale, t } = useI18n();
const open = ref(false);
const profilePerson = ref<Participant | null>(null);

function personLabel(person: Participant): string {
  const explicitName = person.name.trim();
  if (explicitName && explicitName.toLowerCase() !== person.email.trim().toLowerCase()) {
    return explicitName;
  }
  return contacts.findByEmail(person.email)?.display_name?.trim()
    || participantDisplayName(accounts.accounts, person);
}

const senderName = computed(() => personLabel(props.from));
const exactTimestamp = computed(() => new Intl.DateTimeFormat(locale.value, {
  dateStyle: 'full',
  timeStyle: 'short',
}).format(new Date(props.timestamp)));
const distinctReplyTo = computed(() => {
  const replyTo = props.replyTo.trim();
  return replyTo && replyTo.toLowerCase() !== props.from.email.trim().toLowerCase()
    ? replyTo
    : '';
});
const summary = computed(() => {
  const recipients = props.to.length > 0 ? props.to : props.cc;
  if (recipients.length === 0) return '';
  const prefix = props.to.length > 0 ? t('common.to') : t('common.cc');
  const remaining = recipients.length - 1;
  const primary = recipients[0];
  if (!primary) return `${prefix}: —`;
  return `${prefix}: ${personLabel(primary)}${remaining > 0 ? ` +${remaining}` : ''}`;
});

// Static keys keep the catalogue check able to see them.
const VERDICT_LABELS: Record<Exclude<AuthenticationVerdictKey, 'providerResult'>, string> = {
  pass: 'mail.authentication.pass',
  fail: 'mail.authentication.fail',
  policy: 'mail.authentication.policy',
  permerror: 'mail.authentication.permerror',
  softfail: 'mail.authentication.softfail',
  neutral: 'mail.authentication.neutral',
  temperror: 'mail.authentication.temperror',
  none: 'mail.authentication.none',
  unavailable: 'mail.authentication.unavailable',
};

const authenticationResults = computed(() => ({
  spf: props.authSpf,
  dkim: props.authDkim,
  dmarc: props.authDmarc,
}));
const hasAuthenticationResults = computed(() => hasVerdicts(authenticationResults.value));
const hasSecurityDetails = computed(() => Boolean(props.smimeState) || hasAuthenticationResults.value);
const authenticationTone = computed(() => summarizeAuthentication(authenticationResults.value));
const authenticationRows = computed(() =>
  authenticationVerdicts(authenticationResults.value).map((verdict) => ({
    ...verdict,
    label: verdict.key === 'providerResult'
      ? t('mail.authentication.providerResult', { result: verdict.raw })
      : t(VERDICT_LABELS[verdict.key]),
  })),
);

const smimeDetails = computed(() => {
  switch (props.smimeState) {
    case 'signature_partial':
      return { title: t('mail.smime.signaturePartial'), description: t('mail.smime.signaturePartialDescription'), tone: 'warning' };
    case 'signature_valid':
      return {
        title: t('mail.smime.headingSignature'),
        description: t('mail.smime.detailsSignature'),
        tone: 'success',
      };
    case 'signature_unchained':
      return {
        title: t('mail.smime.headingSignatureUnchained'),
        description: t('mail.smime.detailsSignatureUnchained'),
        tone: 'warning',
      };
    case 'signature_identity_mismatch':
      return {
        title: t('mail.smime.headingSignatureIdentityMismatch'),
        description: t('mail.smime.signatureIdentityMismatchDescription', {
          signer: props.smimeSignerEmail || props.smimeSignerCn || t('mail.smime.unknownSigner'),
          from: props.from.email || t('mail.smime.unknownSigner'),
        }),
        tone: 'warning',
      };
    case 'signature_invalid':
      return {
        title: t('mail.smime.signatureInvalid'),
        description: t('mail.smime.signatureInvalidDescription'),
        tone: 'danger',
      };
    case 'signature_unverified':
      return {
        title: t('mail.smime.headingSignatureDetected'),
        description: t('mail.smime.detailsSignatureDetected'),
        tone: 'warning',
      };
    case 'decrypted':
      return {
        title: t('mail.smime.headingDecrypted'),
        description: t('mail.smime.detailsDecrypted'),
        tone: 'success',
      };
    default:
      return null;
  }
});

function openDetails() {
  open.value = true;
}

defineExpose({ openDetails });

function openProfile(person: Participant) {
  profilePerson.value = person;
}

function closeAll() {
  profilePerson.value = null;
  open.value = false;
}

watch(() => props.from.email, () => {
  profilePerson.value = null;
  open.value = false;
});
</script>

<template>
  <button
    v-if="summary"
    type="button"
    class="recipient-summary"
    aria-haspopup="dialog"
    :aria-expanded="open"
    @click="openDetails"
  >
    <span>{{ summary }}</span>
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m5 6 3 3 3-3" /></svg>
  </button>

  <Teleport to="body">
    <BottomSheet
      :label="$t('mail.recipients.detailsLabel')"
      :open="open"
      desktop-width="min(520px, calc(100vw - 80px))"
      @dismiss="open = false"
    >
      <section class="recipient-details">
        <header class="recipient-details__header">
          <div class="recipient-details__title">
            <h2>{{ props.subject || $t('mail.recipients.noSubject') }}</h2>
            <time :datetime="props.timestamp">{{ exactTimestamp }}</time>
          </div>
          <IconButton :title="$t('common.close')" size="lg" @click="open = false">
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m4 4 8 8M12 4l-8 8" /></svg>
          </IconButton>
        </header>

        <div class="recipient-details__groups">
          <section v-if="hasSecurityDetails" class="recipient-details__group message-security">
            <h3>{{ $t('mail.security.title') }}</h3>
            <ul class="security-rows">
              <li
                v-if="smimeDetails"
                class="security-row security-row--smime"
                :class="`security-row--${smimeDetails.tone}`"
              >
                <span class="security-row__icon" aria-hidden="true">
                  <svg viewBox="0 0 20 20">
                    <template v-if="props.smimeState === 'decrypted'">
                      <rect x="3" y="8" width="14" height="9" rx="2" />
                      <path d="M6 8V5.8a4 4 0 0 1 8 0V8" />
                      <path d="m7.5 12.4 1.7 1.7 3.5-3.6" />
                    </template>
                    <template v-else>
                      <path d="M10 1.5 12 4l3.2.3-.3 3.2 2 2.5-2 2.5.3 3.2-3.2.3-2 2.5-2-2.5-3.2-.3.3-3.2-2-2.5 2-2.5-.3-3.2L8 4l2-2.5Z" />
                      <path v-if="props.smimeState === 'signature_valid'" d="m6.8 10 2 2 4.3-4.5" />
                      <path v-else-if="props.smimeState === 'signature_invalid'" d="m7.2 7.2 5.6 5.6m0-5.6-5.6 5.6" />
                      <path v-else d="M10 7.1v4m0 2.2h.01" />
                    </template>
                  </svg>
                </span>
                <div class="security-row__body">
                  <strong>{{ smimeDetails.title }}</strong>
                  <p>{{ smimeDetails.description }}</p>
                  <span v-if="props.smimeSignerCn || props.smimeSignerEmail" class="security-row__signer">
                    <span>{{ $t('mail.security.signedBy') }}</span>
                    <strong v-if="props.smimeSignerCn">{{ props.smimeSignerCn }}</strong>
                    <code v-if="props.smimeSignerEmail">{{ props.smimeSignerEmail }}</code>
                  </span>
                </div>
              </li>

              <li
                v-if="hasAuthenticationResults"
                class="security-row security-row--authentication"
                :class="`security-row--${authenticationTone}`"
              >
                <span class="security-row__icon" aria-hidden="true">
                  <svg viewBox="0 0 20 20">
                    <path d="M10 2.1 16 4.4V9c0 3.8-2.3 6.7-6 8.2C6.3 15.7 4 12.8 4 9V4.4z" />
                    <path v-if="authenticationTone === 'pass'" d="m6.8 10 2 2 4.3-4.5" />
                    <path v-else-if="authenticationTone === 'fail'" d="m7.2 7.2 5.6 5.6m0-5.6-5.6 5.6" />
                    <path v-else d="M10 7.1v4m0 2.2h.01" />
                  </svg>
                </span>
                <div class="security-row__body">
                  <strong>{{ $t('mail.authentication.title') }}</strong>
                  <ul class="auth-verdicts">
                    <li
                      v-for="row in authenticationRows"
                      :key="row.mechanism"
                      class="auth-verdict"
                      :class="`auth-verdict--${row.tone}`"
                      :title="row.label"
                    >
                      <svg viewBox="0 0 12 12" aria-hidden="true">
                        <path v-if="row.tone === 'pass'" d="m2.5 6 2.2 2.2 4.8-4.8" />
                        <path v-else-if="row.tone === 'fail'" d="m3 3 6 6m0-6-6 6" />
                        <path v-else-if="row.tone === 'warn'" d="M6 2.6v3.6m0 2.4h.01" />
                        <path v-else d="M3.2 6h5.6" />
                      </svg>
                      <span>{{ row.mechanism }}</span>
                      <span class="sr-only">{{ row.label }}</span>
                    </li>
                  </ul>
                  <p>{{ $t('mail.authentication.description') }}</p>
                </div>
              </li>
            </ul>
          </section>

          <section class="recipient-details__group message-route">
            <h3>{{ $t('mail.security.route') }}</h3>
            <div class="message-route__row">
              <span class="message-route__label">{{ $t('common.from') }}</span>
              <button type="button" class="recipient-person recipient-person--interactive" @click="openProfile(props.from)">
                <Avatar :name="senderName" :email="props.from.email" :size="36" />
                <div><strong>{{ senderName }}</strong><span>{{ props.from.email }}</span></div>
                <svg class="recipient-person__chevron" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 3 5 5-5 5" /></svg>
              </button>
            </div>
            <div v-if="distinctReplyTo" class="message-route__row">
              <span class="message-route__label">{{ $t('mail.recipients.replyTo') }}</span>
              <div class="recipient-address">{{ distinctReplyTo }}</div>
            </div>
            <div v-if="props.to.length > 0" class="message-route__row">
              <span class="message-route__label">{{ $t('common.to') }}</span>
              <div class="message-route__people">
                <button
                  v-for="(recipient, index) in props.to"
                  :key="`to-${recipient.email}-${index}`"
                  type="button"
                  class="recipient-person recipient-person--interactive"
                  @click="openProfile(recipient)"
                >
                  <Avatar :name="personLabel(recipient)" :email="recipient.email" :size="36" />
                  <div><strong>{{ personLabel(recipient) }}</strong><span>{{ recipient.email }}</span></div>
                  <svg class="recipient-person__chevron" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 3 5 5-5 5" /></svg>
                </button>
              </div>
            </div>
            <div v-if="props.cc.length > 0" class="message-route__row">
              <span class="message-route__label">{{ $t('common.cc') }}</span>
              <div class="message-route__people">
                <button
                  v-for="(recipient, index) in props.cc"
                  :key="`cc-${recipient.email}-${index}`"
                  type="button"
                  class="recipient-person recipient-person--interactive"
                  @click="openProfile(recipient)"
                >
                  <Avatar :name="personLabel(recipient)" :email="recipient.email" :size="36" />
                  <div><strong>{{ personLabel(recipient) }}</strong><span>{{ recipient.email }}</span></div>
                  <svg class="recipient-person__chevron" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 3 5 5-5 5" /></svg>
                </button>
              </div>
            </div>
          </section>
        </div>
      </section>
    </BottomSheet>
  </Teleport>

  <PersonProfileSheet
    v-if="profilePerson"
    :open="Boolean(profilePerson)"
    :person="profilePerson"
    @dismiss="profilePerson = null"
    @navigate="closeAll"
  />
</template>

<style scoped>
.recipient-summary {
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 2px;
  border: 0;
  padding: 0;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  cursor: pointer;
}

.recipient-summary span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.recipient-summary svg {
  width: 13px;
  height: 13px;
  flex: 0 0 auto;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.5;
}

.recipient-summary:hover,
.recipient-summary:focus-visible { color: var(--text-primary); }

.recipient-summary:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
  border-radius: var(--radius-xs);
}

.recipient-details {
  width: min(520px, 100vw);
  padding: var(--space-xl);
}

.recipient-details__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-lg);
  padding: 0 0 var(--space-lg);
  border-bottom: 1px solid var(--border-color-muted);
}

.recipient-details__title {
  display: grid;
  min-width: 0;
  gap: var(--space-xs);
}

.recipient-details__header h2 {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-xl);
  line-height: 1.2;
  overflow-wrap: anywhere;
}

.recipient-details__header time {
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

.recipient-details__header :deep(.icon-button) {
  border-radius: var(--radius-full);
  background: var(--bg-raised);
}

.recipient-details__header :deep(.icon-button:hover) {
  background: var(--bg-overlay);
}

.recipient-details__header svg {
  width: 15px;
  height: 15px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-width: 1.6;
}

.recipient-details__groups {
  display: grid;
  gap: var(--space-xl);
  padding-top: var(--space-lg);
}

.recipient-details__group {
  display: grid;
  gap: var(--space-sm);
}

.recipient-details__group h3 {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-bold);
  letter-spacing: 0.07em;
  text-transform: uppercase;
}

.security-rows {
  display: grid;
  margin: 0;
  padding: 0;
  list-style: none;
}

.security-row {
  --security-color: var(--text-tertiary);
  display: grid;
  grid-template-columns: 32px minmax(0, 1fr);
  gap: var(--space-md);
  padding: var(--space-md) 0;
  border-bottom: var(--border-muted);
}

.security-row:first-child {
  padding-top: var(--space-xs);
}

.security-row:last-child {
  padding-bottom: 0;
  border-bottom: 0;
}

.security-row--success,
.security-row--pass { --security-color: var(--accent-green); }
.security-row--warning,
.security-row--warn { --security-color: var(--accent-amber); }
.security-row--danger,
.security-row--fail { --security-color: var(--accent-red); }

.security-row__icon {
  display: grid;
  width: 32px;
  height: 32px;
  place-items: center;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--security-color) 14%, transparent);
  color: var(--security-color);
}

.security-row__icon svg {
  width: 18px;
  height: 18px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.6;
}

.security-row__body {
  display: grid;
  min-width: 0;
  gap: var(--space-xs);
  padding-top: 5px;
}

.security-row__body > strong {
  color: var(--text-primary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
  line-height: 1.3;
}

.security-row__body > p {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  line-height: 1.45;
}

.security-row__signer {
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-xs) var(--space-sm);
  margin-top: var(--space-2xs);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.security-row__signer strong {
  color: var(--text-secondary);
  font-weight: var(--weight-semibold);
}

.security-row__signer code {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono);
}

.auth-verdicts {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-xs);
  margin: var(--space-2xs) 0 0;
  padding: 0;
  list-style: none;
}

.auth-verdict {
  --verdict-color: var(--text-tertiary);
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border-radius: var(--radius-full);
  padding: 3px 9px 3px 7px;
  background: color-mix(in srgb, var(--verdict-color) 12%, transparent);
  color: var(--verdict-color);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-strong);
  letter-spacing: 0.03em;
  line-height: 1.2;
}

.auth-verdict svg {
  width: 11px;
  height: 11px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.9;
}

.auth-verdict--pass { --verdict-color: var(--accent-green); }
.auth-verdict--fail { --verdict-color: var(--accent-red-soft); }
.auth-verdict--warn { --verdict-color: var(--accent-amber-soft); }

.auth-verdict--none {
  background: var(--bg-raised);
}

.recipient-person {
  display: grid;
  grid-template-columns: 36px minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-md);
}

.recipient-person--interactive {
  width: 100%;
  border: 0;
  border-radius: var(--radius-sm);
  padding: var(--space-sm);
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.recipient-person--interactive:hover,
.recipient-person--interactive:focus-visible {
  background: var(--bg-hover);
}

.recipient-person__chevron {
  width: 15px;
  height: 15px;
  color: var(--text-tertiary);
}

.recipient-person > div {
  display: grid;
  min-width: 0;
  gap: 2px;
}

.recipient-person > div > strong,
.recipient-person > div > span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.recipient-person > div > strong {
  color: var(--text-primary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
}

.recipient-person > div > span,
.recipient-address {
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

.recipient-address {
  overflow-wrap: anywhere;
}

.message-route {
  gap: 0;
}

.message-route > h3 {
  margin-bottom: var(--space-xs);
}

.message-route__row {
  display: grid;
  grid-template-columns: 76px minmax(0, 1fr);
  align-items: center;
  gap: var(--space-md);
  padding: var(--space-sm) 0;
  border-bottom: var(--border-muted);
}

.message-route__row:last-child {
  border-bottom: 0;
}

.message-route__label {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-strong);
}

.message-route__people {
  display: grid;
  gap: var(--space-xs);
}

@media (max-width: 1020px) {
  .recipient-details {
    width: 100%;
    padding: var(--space-lg);
  }

  .message-route__row {
    grid-template-columns: 1fr;
    gap: var(--space-xs);
  }
}
</style>
