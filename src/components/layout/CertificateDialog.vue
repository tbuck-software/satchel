<script setup lang="ts">
/**
 * What a rejected server certificate said, and the one way around it.
 *
 * Opened from the banner and from the account's marker in the sidebar. On a
 * phone `ModalShell` makes it a page of its own, with the actions at the
 * bottom where a thumb is.
 *
 * The seven-day exception is offered only when the backend says expiry is the
 * certificate's only fault, and it takes a second, explicit step that names
 * the fingerprint and the end date.
 */
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import ModalShell from '@/components/ui/ModalShell.vue';
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { useDismissableOnTop } from '@/composables/useDismissable';
import { useToast } from '@/composables/useToast';
import { useAccountsStore } from '@/stores/accounts';
import { useCertificateTrustStore } from '@/stores/certificateTrust';
import { useConnectionStore } from '@/stores/connection';
import { useUiStore } from '@/stores/ui';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import {
  certificateDetail,
  certificateNote,
  formatCertificateMoment,
} from '@/utils/certificateLabels';

const { t } = useI18n();
const trust = useCertificateTrustStore();
const accounts = useAccountsStore();
const connection = useConnectionStore();
const toast = useToast();
const ui = useUiStore();

const accountId = computed(() => trust.detailsAccountId);
const account = computed(() => accounts.accounts.find((entry) => entry.id === accountId.value) ?? null);
const accountName = computed(() => account.value?.displayName || account.value?.email || '');
const problem = computed(() => (accountId.value ? trust.problemFor(accountId.value) : null));
const rejection = computed(() => problem.value?.rejection ?? null);
const exception = computed(() => (accountId.value ? trust.exceptionFor(accountId.value) : null));
const confirming = ref(false);
const busy = ref(false);
const copyFailed = ref(false);

watch(accountId, () => {
  confirming.value = false;
  copyFailed.value = false;
});

const note = computed(() => (rejection.value ? certificateNote(rejection.value) : ''));
const explanation = computed(() => {
  if (!rejection.value) return '';
  return rejection.value.reason === 'expired'
    ? t('certificates.dialog.explainExpired')
    : certificateDetail(rejection.value);
});
const lastSuccess = computed(() => (account.value?.lastSyncAt
  ? formatCertificateMoment(account.value.lastSyncAt)
  : t('certificates.dialog.never')));
const trustUntil = computed(() => formatCertificateMoment(
  new Date(Date.now() + 7 * 86_400_000).toISOString(),
));

function close() {
  trust.closeDetails();
}

useDismissableOnTop(() => accountId.value !== null, () => {
  if (confirming.value) confirming.value = false;
  else close();
});

async function copyNote() {
  copyFailed.value = false;
  if (!navigator.clipboard) {
    copyFailed.value = true;
    return;
  }
  try {
    await navigator.clipboard.writeText(note.value);
    toast.success(t('common.copied'));
  } catch {
    copyFailed.value = true;
  }
}

async function confirmTrust() {
  if (!accountId.value || busy.value) return;
  const id = accountId.value;
  const name = accountName.value;
  busy.value = true;
  try {
    const granted = await trust.trust(id);
    close();
    toast.success(t('certificates.dialog.trusted', {
      account: name,
      until: formatCertificateMoment(granted.expires_at),
    }), {
      action: {
        label: t('common.undo'),
        handler: () => trust.revoke(id, granted.host),
      },
    });
  } catch (error) {
    toast.error(t('certificates.dialog.trustFailed', { error: backendErrorLabel(error) }));
    confirming.value = false;
  } finally {
    busy.value = false;
  }
}

async function revokeException() {
  if (!accountId.value || !exception.value || busy.value) return;
  busy.value = true;
  try {
    await trust.revoke(accountId.value, exception.value.host);
    toast.info(t('certificates.dialog.revoked'));
    close();
  } catch (error) {
    toast.error(t('certificates.dialog.revokeFailed', { error: backendErrorLabel(error) }));
  } finally {
    busy.value = false;
  }
}

function retry() {
  connection.requestRetry();
  close();
}
</script>

<template>
  <Teleport to="body">
    <ModalShell
      v-if="accountId && (rejection || exception)"
      :label="t('certificates.dialog.title', { account: accountName })"
      width="560px"
      height="auto"
      @dismiss="close"
    >
      <div class="certificate" :class="{ 'certificate--page': ui.isSinglePane }">
        <header class="certificate__head">
          <h2 class="certificate__title">{{ t('certificates.dialog.title', { account: accountName }) }}</h2>
          <IconButton :title="t('common.close')" @click="close">
            <AppIcon><path d="m4 4 8 8m0-8-8 8" /></AppIcon>
          </IconButton>
        </header>

        <div class="certificate__body">
          <template v-if="rejection">
            <p class="certificate__explain">{{ explanation }}</p>
            <dl class="certificate__facts">
              <dt>{{ t('certificates.dialog.server') }}</dt>
              <dd class="certificate__mono">{{ rejection.host }}{{ rejection.port ? `:${rejection.port}` : '' }}</dd>
              <dt>{{ t('certificates.dialog.subject') }}</dt>
              <dd class="certificate__mono">{{ rejection.subject ?? t('certificates.dialog.unknown') }}</dd>
              <dt>{{ t('certificates.dialog.issuer') }}</dt>
              <dd>{{ rejection.issuer ?? t('certificates.dialog.unknown') }}</dd>
              <template v-if="rejection.reason === 'notYetValid'">
                <dt>{{ t('certificates.dialog.validFrom') }}</dt>
                <dd class="certificate__bad">{{ formatCertificateMoment(rejection.notBefore) }}</dd>
              </template>
              <dt>{{ t('certificates.dialog.validUntil') }}</dt>
              <dd :class="{ certificate__bad: rejection.reason === 'expired' }">{{ formatCertificateMoment(rejection.notAfter) }}</dd>
              <dt>{{ t('certificates.dialog.fingerprint') }}</dt>
              <dd class="certificate__mono certificate__fingerprint">{{ rejection.fingerprint }}</dd>
              <dt>{{ t('certificates.dialog.lastSuccess') }}</dt>
              <dd>{{ lastSuccess }}</dd>
            </dl>

            <details class="certificate__note">
              <summary>{{ t('certificates.dialog.note') }}</summary>
              <pre class="certificate__note-text">{{ note }}</pre>
              <p v-if="copyFailed" class="certificate__hint">{{ t('certificates.dialog.copyFailed') }}</p>
            </details>

            <section v-if="confirming" class="certificate__confirm" aria-live="polite">
              <h3 class="certificate__confirm-title">{{ t('certificates.dialog.confirmTitle') }}</h3>
              <p class="certificate__confirm-body">
                {{ t('certificates.dialog.confirmBody', { fingerprint: rejection.fingerprint, account: accountName, until: trustUntil }) }}
              </p>
              <div class="certificate__confirm-actions">
                <AppButton variant="ghost" :disabled="busy" @click="confirming = false">{{ t('common.cancel') }}</AppButton>
                <AppButton variant="primary" :disabled="busy" @click="confirmTrust">{{ t('certificates.dialog.confirm') }}</AppButton>
              </div>
            </section>
          </template>

          <template v-else-if="exception">
            <h3 class="certificate__confirm-title">
              {{ t('certificates.dialog.exceptionTitle', { until: formatCertificateMoment(exception.expires_at) }) }}
            </h3>
            <p class="certificate__explain">
              {{ t('certificates.dialog.exceptionBody', { account: accountName, host: exception.host }) }}
            </p>
            <dl class="certificate__facts">
              <dt>{{ t('certificates.dialog.server') }}</dt>
              <dd class="certificate__mono">{{ exception.host }}</dd>
              <dt>{{ t('certificates.dialog.fingerprint') }}</dt>
              <dd class="certificate__mono certificate__fingerprint">{{ exception.fingerprint }}</dd>
            </dl>
          </template>
        </div>

        <footer v-if="!confirming" class="certificate__actions">
          <template v-if="rejection">
            <AppButton
              v-if="rejection.exceptionAllowed && !exception"
              class="certificate__trust"
              variant="secondary"
              @click="confirming = true"
            >
              {{ t('certificates.dialog.trust') }}
            </AppButton>
            <AppButton variant="ghost" @click="copyNote">{{ t('certificates.dialog.copy') }}</AppButton>
            <AppButton variant="primary" :disabled="connection.retrying" @click="retry">{{ t('common.retry') }}</AppButton>
          </template>
          <template v-else-if="exception">
            <AppButton variant="danger" :disabled="busy" @click="revokeException">{{ t('certificates.dialog.revoke') }}</AppButton>
            <AppButton variant="primary" @click="close">{{ t('common.close') }}</AppButton>
          </template>
        </footer>
      </div>
    </ModalShell>
  </Teleport>
</template>

<style scoped>
.certificate {
  display: flex;
  flex-direction: column;
  min-height: 0;
  max-height: min(720px, calc(100dvh - 2 * var(--space-xl)));
}

.certificate__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-sm);
  padding: var(--space-lg) var(--space-lg) var(--space-sm) var(--space-xl);
}

.certificate__title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  text-wrap: balance;
}

/* The facts and the message can outgrow a small window; the actions stay put. */
.certificate__body {
  display: grid;
  gap: var(--space-md);
  padding: 0 var(--space-xl) var(--space-md);
  overflow-y: auto;
  min-height: 0;
}

.certificate__explain {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.certificate__facts {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: var(--space-2xs) var(--space-md);
  margin: 0;
  font-size: var(--font-size-sm);
}

.certificate__facts dt {
  color: var(--text-tertiary);
}

.certificate__facts dd {
  margin: 0;
  overflow-wrap: anywhere;
}

.certificate__mono {
  font-family: var(--font-mono);
  font-size: var(--font-size-xs);
}

.certificate__fingerprint {
  color: var(--text-secondary);
}

.certificate__bad {
  color: var(--accent-red);
}

.certificate__note {
  font-size: var(--font-size-sm);
}

.certificate__note summary {
  cursor: pointer;
  font-weight: var(--weight-semibold);
}

.certificate__note-text {
  margin: var(--space-xs) 0 0;
  padding: var(--space-sm) var(--space-md);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font-family: var(--font-mono);
  font-size: var(--font-size-xs);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  user-select: text;
}

.certificate__hint {
  margin: var(--space-2xs) 0 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.certificate__confirm {
  display: grid;
  gap: var(--space-xs);
  padding: var(--space-md);
  border: 1px solid color-mix(in srgb, var(--accent-orange) 45%, transparent);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--accent-orange) 10%, transparent);
}

.certificate__confirm-title {
  margin: 0;
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
}

.certificate__confirm-body {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  overflow-wrap: anywhere;
}

.certificate__confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
  flex-wrap: wrap;
}

.certificate__actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
  flex-wrap: wrap;
  padding: var(--space-md) var(--space-xl) var(--space-lg);
  border-top: var(--border-muted);
}

/* Set apart from the everyday actions on the right. */
.certificate__trust {
  margin-right: auto;
}

/* Single pane, the shell is the whole screen (the same breakpoint ModalShell
   uses): actions stack at the bottom, at thumb height. */
.certificate--page {
  max-height: none;
  height: 100%;
}

.certificate--page .certificate__body {
  flex: 1;
}

.certificate--page .certificate__actions,
.certificate--page .certificate__confirm-actions {
  flex-direction: column-reverse;
  align-items: stretch;
}

.certificate--page .certificate__actions {
  padding-bottom: calc(var(--space-lg) + var(--satchel-safe-area-bottom));
}

.certificate--page .certificate__trust {
  margin-right: 0;
}
</style>
