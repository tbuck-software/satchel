<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useSettingsStore } from '@/stores/settings';
import { useAccountsStore } from '@/stores/accounts';
import { useContactsStore } from '@/stores/contacts';
import { useUiStore } from '@/stores/ui';
import {
  backendAddAccount,
  backendCompleteOauthFlow,
  backendDeactivateAccount,
  backendDetectMailProvider,
  backendDiscoverAccount,
  backendListOpenOauthFlows,
  backendPollOauthFlow,
  backendStartOauthFlow,
  backendSupportsNativeCommands,
  backendSyncGroupware,
  backendTestImapConnection,
  backendUpsertSignature,
  translateBackendError,
  type BackendAccountDiscovery,
  type BackendDiscoveredService,
  type BackendOpenOauthFlow,
} from '@/services/backend';
import AppButton from '@/components/ui/AppButton.vue';
import { useFocusTrap } from '@/composables/useFocusTrap';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { openOAuthAuthorizationUrl } from '@/utils/mobileOAuth';
import { transportErrorLabel, transportErrorText } from '@/utils/transportLabels';
import { useI18n } from 'vue-i18n';

type Provider = 'gmail' | 'outlook' | 'imap' | 'pop3';
type LocalService = Omit<BackendDiscoveredService, 'status'> & {
  status: BackendDiscoveredService['status'] | 'checking';
};
type FlowStep = { step: number; label: string };

const settings = useSettingsStore();
const accounts = useAccountsStore();
const contacts = useContactsStore();
const ui = useUiStore();
const { t } = useI18n();
// First run and account setup alike cover the whole app; Tab stays inside.
const frameRef = ref<HTMLElement | null>(null);
const trap = useFocusTrap(frameRef);
const hasTauri = backendSupportsNativeCommands();
const step = ref(1);
const email = ref('');
const password = ref('');
const displayName = ref('');
const username = ref('');
const providerOverride = ref<Provider | null>(null);
const chosenTheme = ref<'dark' | 'light'>(settings.theme);
const chosenSingleKeys = ref(settings.singleKeyShortcuts);
const busy = ref(false);
const errorMessage = ref('');
/** The untranslated chain behind `errorMessage`, empty when it says the same. */
const errorDetail = ref('');
const services = ref<LocalService[]>([]);
const showAdvanced = ref(false);
const imapHost = ref('');
const imapPort = ref('993');
const smtpHost = ref('');
const smtpPort = ref('587');
const connectedEmail = ref('');
const connectedProvider = ref<Provider | null>(null);
const contactImportStatus = ref('');
const testStatus = ref('');
const testBusy = ref(false);
/**
 * A started authorization whose loopback callback never came back. The browser
 * still shows `?state=…&code=…` in its address bar even when it cannot reach
 * our listener, so the flow can be finished by hand from there.
 */
const manualOauth = ref<{ accountId: string; state: string; seedSignature: boolean } | null>(null);
const manualState = ref('');
const manualCode = ref('');
const resumedFlow = ref<BackendOpenOauthFlow | null>(null);
const resuming = computed(() => resumedFlow.value !== null);

const isAccountSetup = computed(() => settings.onboardingComplete && ui.accountSetupOpen);
const wizardVisible = computed(() => !settings.onboardingComplete || ui.accountSetupOpen);
const flowSteps = computed<FlowStep[]>(() => isAccountSetup.value
  ? [
      { step: 1, label: t('onboarding.steps.account') },
      { step: 2, label: t('onboarding.steps.connection') },
      { step: 4, label: t('onboarding.steps.done') },
    ]
  : [
      { step: 1, label: t('onboarding.steps.account') },
      { step: 2, label: t('onboarding.steps.connection') },
      { step: 3, label: t('onboarding.steps.preferences') },
      { step: 4, label: t('onboarding.steps.done') },
    ]);
const currentFlowIndex = computed(() => Math.max(
  flowSteps.value.findIndex((item) => item.step === step.value),
  0,
));
const progressPercent = computed(() => (
  ((currentFlowIndex.value + 1) / flowSteps.value.length) * 100
));
const normalizedEmail = computed(() => email.value.trim().toLowerCase());
const emailDomain = computed(() => normalizedEmail.value.split('@')[1] ?? '');
const validEmail = computed(() => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(normalizedEmail.value));
/** What the domain's MX records said, kept with the domain they answer for. */
const mxDetection = ref<{ domain: string; provider: Provider | null } | null>(null);
const mxProvider = computed(() =>
  mxDetection.value?.domain === emailDomain.value ? mxDetection.value.provider : null,
);
const provider = computed<Provider>(() => {
  if (providerOverride.value) return providerOverride.value;
  const byName = providerForDomain(emailDomain.value);
  return byName === 'imap' ? mxProvider.value ?? 'imap' : byName;
});
const providerDetection = computed(() => {
  if (provider.value === 'gmail') {
    return mxProvider.value === 'gmail' && !providerOverride.value
      ? t('onboarding.account.workspaceDetected', { domain: emailDomain.value })
      : t('onboarding.account.googleDetected');
  }
  if (provider.value === 'outlook') {
    return mxProvider.value === 'outlook' && !providerOverride.value
      ? t('onboarding.account.microsoft365Detected', { domain: emailDomain.value })
      : t('onboarding.account.microsoftDetected');
  }
  return t('onboarding.account.customDomain', { domain: emailDomain.value });
});
/** IMAP and POP3 both need a password up front; OAuth providers do not. */
const usesPassword = computed(() => provider.value === 'imap' || provider.value === 'pop3');
const canStart = computed(() => validEmail.value && (!usesPassword.value || password.value.length > 0));
const failedRequiredServices = computed(() =>
  services.value.filter((service) =>
    (service.kind === 'mail' || service.kind === 'smtp') && service.status !== 'connected',
  ),
);
const connectedServices = computed(() =>
  services.value.filter((service) => service.status === 'connected'),
);
/** OAuth reports no per-service list, and neither does the browser fallback. */
const completionSummary = computed(() => {
  if (connectedProvider.value !== 'imap') return t('onboarding.summary.secureSignedIn');
  const count = connectedServices.value.length;
  if (count === 0) return t('onboarding.summary.connected');
  return t('onboarding.summary.services', count);
});

function providerForDomain(domain: string): Provider {
  const value = domain.toLowerCase();
  if (value === 'gmail.com' || value === 'googlemail.com') return 'gmail';
  if (
    value === 'outlook.com' ||
    value === 'hotmail.com' ||
    value === 'live.com' ||
    value === 'office365.com'
  ) return 'outlook';
  return 'imap';
}

let mxLookup: { domain: string; done: Promise<void> } | null = null;
let mxTimer: number | undefined;

/**
 * A company domain on Google Workspace or Microsoft 365 needs OAuth, and its
 * name does not say so; its MX records do. One lookup per domain.
 */
function lookUpMailProvider(domain: string): Promise<void> {
  if (mxLookup?.domain === domain) return mxLookup.done;
  const done = backendDetectMailProvider(domain)
    .catch(() => null)
    .then((found) => {
      mxDetection.value = { domain, provider: found };
    });
  mxLookup = { domain, done };
  return done;
}

onBeforeUnmount(() => window.clearTimeout(mxTimer));

watch(emailDomain, (domain) => {
  window.clearTimeout(mxTimer);
  if (!validEmail.value || providerForDomain(domain) !== 'imap') return;
  // Typing pauses, not every keystroke, ask the resolver.
  mxTimer = window.setTimeout(() => void lookUpMailProvider(domain), 350);
});

function chooseProvider(value: Provider) {
  providerOverride.value = value;
  clearError();
}

function clearError() {
  errorMessage.value = '';
  errorDetail.value = '';
}

/**
 * Say what went wrong, not what the stack said. The class carries the next
 * step; the chain stays one click away for a bug report.
 */
function showError(error: unknown) {
  const raw = transportErrorText(error);
  const label = transportErrorLabel(error);
  errorMessage.value = label || raw;
  errorDetail.value = label === raw ? '' : raw;
}

/**
 * Try the typed-in server without creating anything.
 *
 * Discovery commits the account the moment it succeeds, so before this there
 * was no way to find out whether a hand-entered host and port work — the only
 * feedback was an account that did or did not appear.
 */
async function testConnection() {
  if (testBusy.value) return;
  testBusy.value = true;
  testStatus.value = '';
  clearError();
  try {
    const result = await backendTestImapConnection({
      email: normalizedEmail.value,
      config: {
        host: imapHost.value.trim(),
        port: parsePort(imapPort.value, provider.value === 'pop3' ? 995 : 993),
        tls: true,
        username: username.value.trim() || normalizedEmail.value,
        password: password.value,
      },
    });
    testStatus.value = t('onboarding.connection.testSucceeded', { count: result.folders });
  } catch (error) {
    showError(error);
  } finally {
    testBusy.value = false;
  }
}

function parsePort(value: string, fallback: number): number {
  const parsed = Number.parseInt(value, 10);
  return parsed > 0 && parsed <= 65535 ? parsed : fallback;
}

function next() {
  const nextStep = flowSteps.value[currentFlowIndex.value + 1];
  if (nextStep) step.value = nextStep.step;
}

function resetWizard(startStep: number) {
  step.value = startStep;
  email.value = '';
  password.value = '';
  displayName.value = '';
  username.value = '';
  providerOverride.value = null;
  mxDetection.value = null;
  mxLookup = null;
  chosenTheme.value = settings.theme;
  chosenSingleKeys.value = settings.singleKeyShortcuts;
  busy.value = false;
  clearError();
  services.value = [];
  showAdvanced.value = false;
  imapHost.value = '';
  imapPort.value = '993';
  smtpHost.value = '';
  smtpPort.value = '587';
  connectedEmail.value = '';
  connectedProvider.value = null;
  contactImportStatus.value = '';
}

function completeConnectionStep() {
  step.value = isAccountSetup.value ? 4 : 3;
}

function cancelAccountSetup() {
  if (!isAccountSetup.value || busy.value) return;
  password.value = '';
  ui.returnToAccountSettings();
}

watch(
  () => ui.accountSetupOpen,
  (open) => {
    if (open) resetWizard(1);
  },
);

function selectTheme(theme: 'dark' | 'light') {
  chosenTheme.value = theme;
  settings.theme = theme;
  document.documentElement.dataset.theme = theme;
}

function defaultServices(): LocalService[] {
  return [
    { kind: 'mail', label: t('onboarding.services.mail'), status: 'checking' },
    { kind: 'smtp', label: t('onboarding.services.smtp'), status: 'checking' },
    { kind: 'caldav', label: t('onboarding.services.calendar'), status: 'checking' },
    { kind: 'carddav', label: t('onboarding.services.contacts'), status: 'checking' },
  ];
}

function discoveredServiceLabel(service: BackendDiscoveredService): string {
  const key = service.label.startsWith('onboarding.services.')
    ? service.label
    : `onboarding.services.${service.kind === 'caldav'
      ? 'calendar'
      : service.kind === 'carddav'
        ? 'contacts'
        : service.kind}`;
  return t(key);
}

function applyDiscoveredServers(result: BackendAccountDiscovery) {
  const imap = result.services.find((service) => service.kind === 'mail');
  const smtp = result.services.find((service) => service.kind === 'smtp');
  username.value ||= result.username;
  imapHost.value ||= imap?.host ?? `imap.${emailDomain.value}`;
  imapPort.value = String(imap?.port ?? parsePort(imapPort.value, 993));
  smtpHost.value ||= smtp?.host ?? `smtp.${emailDomain.value}`;
  smtpPort.value = String(smtp?.port ?? parsePort(smtpPort.value, 587));
}

async function seedSignature(accountId: string) {
  const fallbackName =
    displayName.value.trim() || (normalizedEmail.value.split('@')[0] ?? normalizedEmail.value);
  try {
    await backendUpsertSignature({
      accountId,
      name: t('common.default'),
      bodyText: fallbackName,
      isDefault: true,
    });
  } catch {
    // Signature setup is helpful, but never blocks account setup.
  }
}

async function addConnectedAccount(result: BackendAccountDiscovery) {
  const name = displayName.value.trim() || normalizedEmail.value.split('@')[0] || t('common.email');
  const account = await backendAddAccount({
    displayName: name,
    email: normalizedEmail.value,
    provider: provider.value,
    config: result.config,
  });
  connectedEmail.value = normalizedEmail.value;
  connectedProvider.value = provider.value;
  await accounts.load();
  accounts.setActiveAccount(account.id);
  await seedSignature(account.id);
  if (result.services.some((service) =>
    (service.kind === 'caldav' || service.kind === 'carddav') && service.status === 'connected')) {
    void backendSyncGroupware(account.id).catch(() => undefined);
  }
}

/** The callback never arrived. Distinct from a refusal: the code still exists. */
class OAuthCallbackTimeout extends Error {}

async function waitForOAuth(state: string) {
  const deadline = Date.now() + 2 * 60_000;
  while (Date.now() < deadline) {
    await new Promise<void>((resolve) => window.setTimeout(resolve, 900));
    const poll = await backendPollOauthFlow(state);
    if (poll.status === 'completed') return;
    if (poll.status === 'failed') {
      // Keep the structured reason as the cause: the message is already a
      // sentence, and only the key still says which failure this was.
      throw new Error(translateBackendError(poll.error) || t('onboarding.errors.cancelled'), {
        cause: poll.error,
      });
    }
  }
  throw new OAuthCallbackTimeout(t('onboarding.errors.timeout'));
}

async function connectOAuth() {
  const selectedProvider = provider.value as 'gmail' | 'outlook';
  services.value = [
    { kind: 'mail', label: t('onboarding.services.secureSignIn'), status: 'checking' },
    { kind: 'smtp', label: t('onboarding.services.mailAndCalendar'), status: 'checking' },
  ];
  step.value = 2;
  const account = await backendAddAccount({
    displayName: displayName.value.trim() || normalizedEmail.value.split('@')[0] || t('common.email'),
    email: normalizedEmail.value,
    provider: selectedProvider,
  });
  await accounts.load();
  accounts.setActiveAccount(account.id);
  let flowState = '';
  try {
    const flow = await backendStartOauthFlow({ accountId: account.id, provider: selectedProvider });
    flowState = flow.state;
    // The backend opens the browser itself and reports whether it managed to.
    // That answer used to be ignored: when nothing opened — which is exactly
    // what a phone does, where the Rust-side opener has no desktop session to
    // hand a URL to — the user watched a spinner for two minutes and was told
    // the login had taken too long, rather than that it had never begun.
    if (!flow.browser_opened) {
      await openOAuthAuthorizationUrl(flow.authorization_url);
    }
    await waitForOAuth(flow.state);
  } catch (error) {
    if (error instanceof OAuthCallbackTimeout) {
      // Keep the account and the open flow: the panel below can still spend
      // the code. Tearing down here is what left the user with no way out.
      offerManualOauth(account.id, flowState, true);
      showError(error);
      return;
    }
    await backendDeactivateAccount(account.id).catch(() => undefined);
    await accounts.load();
    throw error;
  }
  await finishOAuthAccount(account.id, true);
}

function offerManualOauth(accountId: string, state: string, seedSignature: boolean) {
  manualOauth.value = { accountId, state, seedSignature };
  manualState.value = state;
  manualCode.value = '';
}

async function finishOAuthAccount(accountId: string, seed: boolean) {
  if (seed) await seedSignature(accountId);
  await accounts.load();
  accounts.setActiveAccount(accountId);
  connectedEmail.value = normalizedEmail.value;
  connectedProvider.value = provider.value;
  services.value = [
    { kind: 'mail', label: t('onboarding.services.secureSignIn'), status: 'connected' },
    { kind: 'smtp', label: t('onboarding.services.mailAndCalendar'), status: 'connected' },
  ];
  await new Promise<void>((resolve) => window.setTimeout(resolve, 650));
  completeConnectionStep();
}

/**
 * Spend a `state` + `code` pair by hand.
 *
 * The backend consumes the PKCE session on every attempt, right or wrong, so
 * a rejected pair ends the flow: report it and start over rather than leaving
 * a half-connected account behind.
 */
async function completeOauthManually() {
  const pending = manualOauth.value;
  if (!pending || busy.value) return;
  if (!manualState.value.trim() || !manualCode.value.trim()) return;
  busy.value = true;
  clearError();
  try {
    await backendCompleteOauthFlow({
      state: manualState.value.trim(),
      code: manualCode.value.trim(),
    });
    manualOauth.value = null;
    await finishOAuthAccount(pending.accountId, pending.seedSignature);
  } catch (error) {
    await abandonManualOauth();
    showError(error);
  } finally {
    busy.value = false;
  }
}

async function abandonManualOauth() {
  const pending = manualOauth.value;
  manualOauth.value = null;
  if (!pending) return;
  await backendDeactivateAccount(pending.accountId).catch(() => undefined);
  await accounts.load();
  step.value = 1;
}

/**
 * Pick up an authorization that outlived the process that started it.
 *
 * The account row and the PKCE verifier are both on disk, so starting a second
 * flow would only strand the first. Poll the one that is already running: the
 * backend answers it from `oauth_flow_sessions` even though this process has
 * never seen it.
 */
async function resumeOAuthFlow(flow: BackendOpenOauthFlow) {
  resumedFlow.value = flow;
  providerOverride.value = flow.provider === 'outlook' ? 'outlook' : 'gmail';
  await accounts.load();
  const account = accounts.accounts.find((entry) => entry.id === flow.account_id);
  if (account) email.value = account.email;
  services.value = [
    { kind: 'mail', label: t('onboarding.services.secureSignIn'), status: 'checking' },
    { kind: 'smtp', label: t('onboarding.services.mailAndCalendar'), status: 'checking' },
  ];
  step.value = 2;
  busy.value = true;
  try {
    await waitForOAuth(flow.state);
    await accounts.load();
    accounts.setActiveAccount(flow.account_id);
    connectedEmail.value = normalizedEmail.value;
    connectedProvider.value = provider.value;
    services.value = [
      { kind: 'mail', label: t('onboarding.services.secureSignIn'), status: 'connected' },
      { kind: 'smtp', label: t('onboarding.services.mailAndCalendar'), status: 'connected' },
    ];
    completeConnectionStep();
  } catch (error) {
    if (error instanceof OAuthCallbackTimeout) {
      offerManualOauth(flow.account_id, flow.state, false);
      showError(error);
      return;
    }
    // The same cleanup the live flow does in its catch branch: a half
    // connected account must not survive the attempt that created it.
    await backendDeactivateAccount(flow.account_id).catch(() => undefined);
    await accounts.load();
    showError(error);
    step.value = 1;
  } finally {
    busy.value = false;
    resumedFlow.value = null;
  }
}

onMounted(async () => {
  if (!hasTauri || !wizardVisible.value) return;
  try {
    const [flow] = await backendListOpenOauthFlows();
    if (flow) await resumeOAuthFlow(flow);
  } catch {
    // No resumable flow, or a shell that predates the command: start fresh.
  }
});

async function runDiscovery(useManualValues = false) {
  clearError();
  services.value = defaultServices();
  step.value = 2;
  busy.value = true;
  try {
    const result = await backendDiscoverAccount({
      email: normalizedEmail.value,
      password: password.value,
      username: username.value.trim() || null,
      protocol: provider.value === 'pop3' ? 'pop3' : 'imap',
      imap: provider.value !== 'pop3' && useManualValues && imapHost.value.trim()
        ? { host: imapHost.value.trim(), port: parsePort(imapPort.value, 993) }
        : null,
      pop3: provider.value === 'pop3' && useManualValues && imapHost.value.trim()
        ? { host: imapHost.value.trim(), port: parsePort(imapPort.value, 995) }
        : null,
      smtp: useManualValues && smtpHost.value.trim()
        ? { host: smtpHost.value.trim(), port: parsePort(smtpPort.value, 587) }
        : null,
    });
    services.value = result.services.map((service) => ({
      ...service,
      label: discoveredServiceLabel(service),
    }));
    applyDiscoveredServers(result);
    if (!result.ready) {
      showAdvanced.value = true;
      errorMessage.value = t('onboarding.errors.smtpNeedsHelp');
      errorDetail.value = '';
      return;
    }
    await addConnectedAccount(result);
    password.value = '';
    await new Promise<void>((resolve) => window.setTimeout(resolve, 750));
    completeConnectionStep();
  } catch (error) {
    services.value = services.value.map((service) => ({
      ...service,
      status: service.status === 'checking' ? 'needs_input' : service.status,
    }));
    showAdvanced.value = true;
    imapHost.value ||= `imap.${emailDomain.value}`;
    smtpHost.value ||= `smtp.${emailDomain.value}`;
    showError(error);
  } finally {
    busy.value = false;
  }
}

async function startConnection() {
  if (!canStart.value || busy.value) return;
  busy.value = true;
  clearError();
  try {
    // Submitted before the lookup answered: wait for it, it is bounded.
    if (!providerOverride.value && providerForDomain(emailDomain.value) === 'imap') {
      window.clearTimeout(mxTimer);
      await lookUpMailProvider(emailDomain.value);
    }
    if (!hasTauri) {
      accounts.addAccount({
        displayName: displayName.value.trim(),
        email: normalizedEmail.value,
        provider: provider.value,
      });
      connectedEmail.value = normalizedEmail.value;
      connectedProvider.value = provider.value;
      completeConnectionStep();
    } else if (usesPassword.value) {
      busy.value = false;
      await runDiscovery();
    } else {
      await connectOAuth();
    }
  } catch (error) {
    showError(error);
    if (!usesPassword.value) step.value = 1;
  } finally {
    busy.value = false;
  }
}

function skipAccount() {
  connectedEmail.value = '';
  step.value = 3;
}

async function importContactPhotos() {
  contactImportStatus.value = '';
  const result = await contacts.syncAvatars();
  if (!result) {
    contactImportStatus.value = contacts.lastAvatarSyncError || t('onboarding.ready.photosFailed');
    return;
  }
  contactImportStatus.value = t('onboarding.ready.photosImported', { count: result.avatars_updated });
}

function finish() {
  if (isAccountSetup.value) {
    password.value = '';
    ui.returnToAccountSettings();
    return;
  }
  settings.singleKeyShortcuts = chosenSingleKeys.value;
  settings.completeOnboarding();
}
</script>

<template>
  <Transition name="onboarding">
    <div
      v-if="wizardVisible"
      class="onboarding-shell"
      :class="{ 'account-setup-shell': isAccountSetup }"
      tabindex="-1"
      @keydown.esc="cancelAccountSetup"
    >
      <div class="backdrop-grid" />
      <div class="orb orb-one" />
      <div class="orb orb-two" />

      <main
        ref="frameRef"
        class="wizard-frame"
        role="dialog"
        aria-modal="true"
        tabindex="-1"
        :aria-label="isAccountSetup ? $t('onboarding.addAccountTitle') : $t('onboarding.setupTitle')"
        @keydown.tab="trap.onTab"
      >
        <button
          v-if="isAccountSetup"
          type="button"
          class="wizard-close"
          :aria-label="$t('onboarding.closeSetup')"
          :disabled="busy"
          @click="cancelAccountSetup"
        >
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M4 4l8 8M12 4l-8 8" /></svg>
        </button>
        <aside class="wizard-rail">
          <div class="brand-mark" aria-label="Satchel">
            <span class="brand-glyph">S</span>
            <span class="brand-word">Satchel</span>
          </div>

          <ol class="step-list" :aria-label="$t('onboarding.stepsLabel')">
            <li v-for="(item, index) in flowSteps" :key="item.label"
                :class="{ active: step === item.step, done: currentFlowIndex > index }">
              <span class="step-index">
                <svg v-if="currentFlowIndex > index" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8"><path d="m2 6 2.5 2.5L10 3" /></svg>
                <template v-else>{{ index + 1 }}</template>
              </span>
              <span>{{ item.label }}</span>
            </li>
          </ol>

          <div v-if="step === 1 && validEmail && usesPassword" class="privacy-note">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="3" y="7" width="10" height="7" rx="2"/><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"/></svg>
            <span>{{ $t('onboarding.privacyNote') }}</span>
          </div>
        </aside>

        <section class="wizard-content">
          <div class="progress-track"><div class="progress-fill" :style="{ width: `${progressPercent}%` }" /></div>

          <Transition name="step" mode="out-in">
            <div v-if="step === 1" key="identity" class="panel account-panel">
              <h2>{{ $t('onboarding.account.heading') }}</h2>

              <form class="identity-form" @submit.prevent="startConnection">
                <label class="floating-field">
                  <span>{{ $t('onboarding.account.emailLabel') }}</span>
                  <input v-model="email" type="email" autocomplete="email" :placeholder="$t('onboarding.account.emailPlaceholder')" autofocus>
                  <small v-if="validEmail" class="provider-detection">
                    <span class="pulse-dot" />
                    {{ providerDetection }}
                  </small>
                </label>
                <label v-if="validEmail && usesPassword" class="floating-field">
                  <span>{{ $t('onboarding.account.passwordLabel') }}</span>
                  <input v-model="password" type="password" autocomplete="current-password" :placeholder="$t('onboarding.account.passwordPlaceholder')">
                  <small>{{ $t('onboarding.account.passwordHint') }}</small>
                </label>
                <div v-if="validEmail" class="provider-alternatives">
                  <span>{{ $t('onboarding.account.otherMethod') }}</span>
                  <button v-if="provider !== 'gmail'" type="button" @click="chooseProvider('gmail')">Google</button>
                  <button v-if="provider !== 'outlook'" type="button" @click="chooseProvider('outlook')">Microsoft</button>
                  <button v-if="provider !== 'imap'" type="button" @click="chooseProvider('imap')">IMAP</button>
                  <button v-if="provider !== 'pop3'" type="button" @click="chooseProvider('pop3')">POP3</button>
                </div>
                <label class="floating-field optional-field">
                  <span>{{ $t('onboarding.account.nameLabel') }} <em>{{ $t('common.optional') }}</em></span>
                  <input v-model="displayName" autocomplete="name" :placeholder="$t('onboarding.account.namePlaceholder')">
                </label>

                <p v-if="errorMessage" class="inline-error" role="alert">{{ errorMessage }}</p>
                <details v-if="errorDetail" class="error-detail">
                  <summary>{{ $t('errors.technicalDetails') }}</summary>
                  <pre>{{ errorDetail }}</pre>
                </details>
                <div class="actions-row">
                  <button v-if="isAccountSetup" type="button" class="text-action" @click="cancelAccountSetup">{{ $t('common.cancel') }}</button>
                  <button v-if="!isAccountSetup" type="button" class="text-action" @click="skipAccount">{{ $t('onboarding.account.later') }}</button>
                  <button type="submit" class="primary-action" :disabled="!canStart || busy">
                    {{ usesPassword ? $t('onboarding.account.connectAuto') : $t('onboarding.account.signInSecurely') }} <span>→</span>
                  </button>
                </div>
              </form>
            </div>

            <div v-else-if="step === 2" key="discovery" class="panel discovery-panel">
              <h2>{{ busy ? $t('onboarding.connection.checking', { domain: emailDomain || $t('onboarding.connection.fallbackDomain') }) : failedRequiredServices.length ? $t('onboarding.connection.missing') : $t('onboarding.connection.ready') }}</h2>
              <p v-if="resuming" class="panel-copy">{{ $t('onboarding.connection.resuming') }}</p>
              <p v-if="!busy && failedRequiredServices.length" class="panel-copy">
                {{ $t('onboarding.connection.missingCopy') }}
              </p>

              <div class="service-map" :class="{ scanning: busy }">
                <div class="service-origin"><span>{{ normalizedEmail.charAt(0).toUpperCase() || '@' }}</span><small>{{ emailDomain }}</small></div>
                <div class="service-line" />
                <div class="service-list">
                  <article v-for="service in services" :key="service.kind" class="service-row" :class="service.status">
                    <span class="service-state">
                      <span v-if="service.status === 'checking'" class="spinner" />
                      <svg v-else-if="service.status === 'connected'" width="14" height="14" viewBox="0 0 13 13" fill="none" stroke="currentColor" stroke-width="1.8"><path d="m2 6.5 2.7 2.7L11 3"/></svg>
                      <span v-else>!</span>
                    </span>
                    <span class="service-name"><strong>{{ service.label }}</strong><small>{{ service.status === 'checking' ? $t('onboarding.connection.stateChecking') : service.status === 'connected' ? [service.host, service.port].filter(Boolean).join(':') || $t('onboarding.connection.stateConnected') : service.status === 'not_found' ? $t('onboarding.connection.stateNotOffered') : $t('onboarding.connection.stateNeedsInput') }}</small></span>
                    <span v-if="service.collections" class="service-count">{{ service.collections }}</span>
                  </article>
                </div>
              </div>

              <div v-if="manualOauth" class="manual-card">
                <div class="manual-heading">
                  <span>
                    <strong>{{ $t('onboarding.oauth.manualHeading') }}</strong>
                    <small>{{ $t('onboarding.oauth.manualHint') }}</small>
                  </span>
                </div>
                <div class="manual-grid manual-grid--wide">
                  <label>
                    <span>{{ $t('onboarding.oauth.stateLabel') }}</span>
                    <input v-model="manualState" autocomplete="off" spellcheck="false">
                  </label>
                  <label>
                    <span>{{ $t('onboarding.oauth.codeLabel') }}</span>
                    <input v-model="manualCode" autocomplete="off" spellcheck="false" :placeholder="$t('onboarding.oauth.codePlaceholder')">
                  </label>
                </div>
                <p v-if="errorMessage" class="inline-error" role="alert">{{ errorMessage }}</p>
                <div class="manual-actions">
                  <AppButton variant="ghost" size="sm" :disabled="busy" @click="abandonManualOauth()">
                    {{ $t('common.cancel') }}
                  </AppButton>
                  <AppButton
                    variant="primary"
                    size="sm"
                    :disabled="busy || !manualState.trim() || !manualCode.trim()"
                    @click="completeOauthManually()"
                  >
                    {{ $t('onboarding.oauth.complete') }}
                  </AppButton>
                </div>
              </div>

              <div v-if="showAdvanced && !busy" class="manual-card">
                <button type="button" class="manual-heading" @click="showAdvanced = !showAdvanced">
                  <span><strong>{{ $t('onboarding.connection.manualHeading') }}</strong><small>{{ $t('onboarding.connection.manualHint') }}</small></span><span>−</span>
                </button>
                <div class="manual-grid">
                  <label><span>{{ $t('onboarding.connection.username') }}</span><input v-model="username" autocomplete="username"></label>
                  <span />
                  <label><span>{{ $t('onboarding.connection.imapHost') }}</span><input v-model="imapHost" autocomplete="off"></label>
                  <label class="port-field"><span>{{ $t('onboarding.connection.port') }}</span><input v-model="imapPort" inputmode="numeric"></label>
                  <label><span>{{ $t('onboarding.connection.smtpHost') }}</span><input v-model="smtpHost" autocomplete="off"></label>
                  <label class="port-field"><span>{{ $t('onboarding.connection.port') }}</span><input v-model="smtpPort" inputmode="numeric"></label>
                </div>
                <p v-if="errorMessage" class="inline-error" role="alert">{{ errorMessage }}</p>
                <details v-if="errorDetail" class="error-detail">
                  <summary>{{ $t('errors.technicalDetails') }}</summary>
                  <pre>{{ errorDetail }}</pre>
                </details>
                <p v-if="testStatus" class="inline-success">{{ testStatus }}</p>
                <div class="manual-actions">
                  <button type="button" class="text-action" @click="step = 1">{{ $t('onboarding.connection.changeCredentials') }}</button>
                  <span class="manual-buttons">
                    <AppButton
                      variant="ghost"
                      size="sm"
                      :disabled="testBusy || !imapHost.trim()"
                      @click="testConnection()"
                    >
                      {{ testBusy ? $t('onboarding.connection.testing') : $t('onboarding.connection.test') }}
                    </AppButton>
                    <button type="button" class="primary-action compact" @click="runDiscovery(true)">{{ $t('onboarding.connection.recheck') }} <span>↻</span></button>
                  </span>
                </div>
              </div>
              <button v-else-if="!busy && failedRequiredServices.length" type="button" class="secondary-action" @click="showAdvanced = true">{{ $t('onboarding.connection.enterServerData') }}</button>
            </div>

            <div v-else-if="step === 3" key="preferences" class="panel preferences-panel">
              <h2>{{ $t('onboarding.preferences.heading') }}</h2>
              <p class="panel-copy">{{ $t('onboarding.preferences.copy') }}</p>

              <div class="preference-section">
                <span class="section-label">{{ $t('onboarding.preferences.appearance') }}</span>
                <div class="theme-choice">
                  <button type="button" :class="{ selected: chosenTheme === 'dark' }" @click="selectTheme('dark')">
                    <span class="theme-preview dark"><i/><i/><i/></span><strong>{{ $t('onboarding.preferences.dark') }}</strong>
                  </button>
                  <button type="button" :class="{ selected: chosenTheme === 'light' }" @click="selectTheme('light')">
                    <span class="theme-preview light"><i/><i/><i/></span><strong>{{ $t('onboarding.preferences.light') }}</strong>
                  </button>
                </div>
              </div>

              <div v-if="!ui.isMobile" class="preference-toggle">
                <span><strong>{{ $t('onboarding.preferences.singleKeyShortcuts') }}</strong><small>{{ $t('onboarding.preferences.singleKeyShortcutsHint', { j: 'j', k: 'k' }) }}</small></span>
                <ToggleSwitch
                  v-model="chosenSingleKeys"
                  :label="$t('onboarding.preferences.singleKeyShortcuts')"
                />
              </div>

              <div class="actions-row end"><button type="button" class="primary-action" @click="next">{{ $t('common.next') }} <span>→</span></button></div>
            </div>

            <div v-else key="ready" class="panel ready-panel">
              <div class="ready-seal"><span>✓</span></div>
              <h2>{{ connectedEmail ? $t('onboarding.ready.accountTitle') : $t('onboarding.ready.title') }}</h2>
              <p class="panel-copy">{{ connectedEmail ? $t('onboarding.ready.accountCopy') : $t('onboarding.ready.copy') }}</p>

              <div v-if="connectedEmail" class="completion-card">
                <div class="completion-account"><span>{{ connectedEmail.charAt(0).toUpperCase() }}</span><div><strong>{{ connectedEmail }}</strong><small>{{ completionSummary }}</small></div></div>
                <div v-if="connectedServices.length > 0" class="completion-checks"><span v-for="service in connectedServices" :key="service.kind">✓ {{ service.label }}</span></div>
              </div>

              <div class="ready-actions">
                <button type="button" class="primary-action launch" @click="finish">{{ isAccountSetup ? $t('onboarding.ready.toAccounts') : $t('onboarding.ready.openSatchel') }} <span>→</span></button>
                <button v-if="hasTauri && !isAccountSetup" type="button" class="text-action" :disabled="contacts.avatarSyncRunning" @click="importContactPhotos">
                  {{ contacts.avatarSyncRunning ? $t('onboarding.ready.importingPhotos') : $t('onboarding.ready.importPhotos') }}
                </button>
                <small v-if="contactImportStatus">{{ contactImportStatus }}</small>
              </div>
            </div>
          </Transition>
        </section>
      </main>
    </div>
  </Transition>
</template>

<style scoped>
.onboarding-shell {
  position: fixed;
  inset: 0;
  z-index: 200;
  display: grid;
  place-items: center;
  overflow: auto;
  padding: 28px;
  background: color-mix(in srgb, var(--bg-base) 94%, var(--accent-primary));
  color: var(--text-primary);
}

.account-setup-shell {
  background: color-mix(in srgb, rgb(5 10 18) 74%, transparent);
  backdrop-filter: blur(12px) saturate(.86);
}

.account-setup-shell .wizard-frame {
  min-height: min(620px, calc(100vh - 72px));
  box-shadow: 0 42px 120px rgb(0 0 0 / .5), inset 0 1px rgb(255 255 255 / .04);
}

.backdrop-grid {
  position: fixed;
  inset: 0;
  opacity: .18;
  pointer-events: none;
  background-image: linear-gradient(color-mix(in srgb, var(--text-primary) 8%, transparent) 1px, transparent 1px), linear-gradient(90deg, color-mix(in srgb, var(--text-primary) 8%, transparent) 1px, transparent 1px);
  background-size: 48px 48px;
  mask-image: radial-gradient(circle at center, black, transparent 78%);
}

.orb { position: fixed; width: 340px; height: 340px; border-radius: 50%; filter: blur(90px); opacity: .13; pointer-events: none; }
.orb-one { top: -100px; right: 8%; background: var(--accent-primary); }
.orb-two { bottom: -140px; left: 12%; background: var(--accent-cyan); }

.wizard-frame {
  position: relative;
  display: grid;
  grid-template-columns: 250px minmax(0, 1fr);
  width: min(980px, calc(100vw - 56px));
  min-height: min(680px, calc(100vh - 56px));
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, var(--accent-primary));
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--bg-surface) 95%, transparent);
  box-shadow: 0 36px 100px rgb(0 0 0 / .32), inset 0 1px rgb(255 255 255 / .035);
  backdrop-filter: blur(24px);
}

.wizard-close {
  position: absolute;
  top: 17px;
  right: 17px;
  z-index: 4;
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  padding: 0;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  color: var(--text-tertiary);
  background: color-mix(in srgb, var(--bg-raised) 86%, transparent);
  cursor: pointer;
  transition: color var(--duration-normal) var(--ease-out), background var(--duration-normal) var(--ease-out), border-color var(--duration-normal) var(--ease-out);
}
.wizard-close:hover:not(:disabled) { border-color: var(--border-color); color: var(--text-primary); background: var(--bg-overlay); }
.wizard-close:disabled { opacity: .4; cursor: not-allowed; }

.wizard-rail {
  display: flex;
  flex-direction: column;
  padding: 31px 28px;
  border-right: 1px solid var(--border-color-muted);
  background: color-mix(in srgb, var(--bg-raised) 74%, transparent);
}

.brand-mark { display: flex; align-items: center; gap: 11px; font-weight: var(--weight-bold); letter-spacing: -.02em; }
.brand-glyph { display: grid; place-items: center; width: 30px; height: 30px; border-radius: 9px 9px 9px 3px; color: var(--text-on-accent); background: linear-gradient(145deg, var(--accent-primary), color-mix(in srgb, var(--accent-primary) 55%, var(--accent-cyan))); box-shadow: 0 7px 22px color-mix(in srgb, var(--accent-primary) 28%, transparent); }
.brand-word { font-size: 16px; }

.step-list { display: grid; gap: 0; margin: 72px 0 0; padding: 0; list-style: none; }
.step-list li { position: relative; display: flex; align-items: center; gap: 12px; min-height: 52px; color: var(--text-tertiary); font-size: var(--font-size-sm); transition: color var(--duration-slow) var(--ease-out); }
.step-list li:not(:last-child)::after { content: ''; position: absolute; left: 14px; top: 40px; bottom: -12px; width: 1px; background: var(--border-color); }
.step-list li.done:not(:last-child)::after { background: color-mix(in srgb, var(--accent-primary) 55%, var(--border-color)); }
.step-list li.active { color: var(--text-primary); font-weight: var(--weight-semibold); }
.step-list li.done { color: var(--text-secondary); }
.step-index { position: relative; z-index: 1; display: grid; place-items: center; width: 29px; height: 29px; flex: 0 0 29px; border: 1px solid var(--border-color); border-radius: 50%; background: var(--bg-raised); color: var(--text-tertiary); font-size: var(--font-size-2xs); }
.active .step-index { border-color: var(--accent-primary); color: var(--accent-primary); box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent-primary) 10%, transparent); }
.done .step-index { border-color: color-mix(in srgb, var(--accent-primary) 60%, transparent); color: var(--accent-primary); background: color-mix(in srgb, var(--accent-primary) 8%, var(--bg-raised)); }

.privacy-note { display: flex; gap: 9px; margin-top: auto; color: var(--text-tertiary); font-size: var(--font-size-3xs); line-height: 1.45; }
.privacy-note svg { flex: 0 0 auto; margin-top: 1px; color: var(--accent-green); }

.wizard-content { position: relative; display: grid; min-width: 0; padding: 68px 74px 54px; }
.progress-track { position: absolute; top: 0; left: 0; right: 0; height: 2px; background: var(--border-color-muted); }
.progress-fill { height: 100%; border-radius: 2px; background: linear-gradient(90deg, var(--accent-primary), var(--accent-cyan)); transition: width var(--duration-slow) var(--ease-out); }
.panel { align-self: center; width: 100%; max-width: 570px; margin: auto; }
h2 { margin: 0; color: var(--text-primary); font-size: 25px; font-weight: var(--weight-strong); letter-spacing: -.03em; line-height: 1.2; }
.panel-copy { max-width: 510px; margin: 9px 0 24px; color: var(--text-secondary); font-size: var(--font-size-sm); line-height: 1.6; }

button, input { font-family: inherit; }
button { color: inherit; }
.primary-action { display: inline-flex; align-items: center; justify-content: center; gap: 22px; min-height: 44px; padding: 0 18px; border: 1px solid color-mix(in srgb, var(--accent-primary) 76%, white); border-radius: var(--radius-sm); color: var(--text-on-accent); background: linear-gradient(145deg, color-mix(in srgb, var(--accent-primary) 88%, var(--bg-inset)), var(--accent-primary)); box-shadow: 0 9px 24px color-mix(in srgb, var(--accent-primary) 20%, transparent); cursor: pointer; font-size: var(--font-size-xs); font-weight: var(--weight-strong); transition: transform var(--duration-normal) var(--ease-out), box-shadow var(--duration-normal) var(--ease-out), opacity var(--duration-normal) var(--ease-out); }
.primary-action:hover:not(:disabled) { transform: translateY(-1px); box-shadow: 0 12px 30px color-mix(in srgb, var(--accent-primary) 28%, transparent); }
.primary-action:disabled { opacity: .42; cursor: not-allowed; box-shadow: none; }
.primary-action span { font-size: 16px; font-weight: var(--weight-regular); }
.primary-action.compact { min-height: 38px; }
.text-action, .secondary-action { border: 0; background: none; color: var(--text-tertiary); cursor: pointer; font-size: var(--font-size-2xs); }
.text-action:hover { color: var(--text-primary); }
.secondary-action { padding: 9px 12px; border: 1px solid var(--border-color); border-radius: var(--radius-sm); }

/* The panel copy used to carry this gap; the headline now sits on the form. */
.identity-form { display: grid; gap: 15px; margin-top: 24px; }
.floating-field { display: grid; gap: 7px; }
.floating-field > span, .manual-grid label > span, .section-label { color: var(--text-tertiary); font-size: var(--font-size-3xs); font-weight: var(--weight-strong); letter-spacing: .09em; text-transform: uppercase; }
.floating-field em { margin-left: 4px; font-style: normal; font-weight: var(--weight-regular); letter-spacing: 0; text-transform: none; }
input { width: 100%; min-width: 0; box-sizing: border-box; padding: 12px 13px; border: 1px solid var(--border-color); border-radius: var(--radius-sm); color: var(--text-primary); background: color-mix(in srgb, var(--bg-base) 62%, transparent); font-size: var(--font-size-sm); transition: border-color var(--duration-normal) var(--ease-out), box-shadow var(--duration-normal) var(--ease-out); }
input:focus { border-color: var(--accent-primary); }
input::placeholder { color: color-mix(in srgb, var(--text-tertiary) 72%, transparent); }
.floating-field small { min-height: 14px; color: var(--text-tertiary); font-size: var(--font-size-3xs); }
.provider-detection { display: flex; align-items: center; gap: 6px; color: var(--accent-green) !important; }
.pulse-dot { width: 5px; height: 5px; border-radius: 50%; background: var(--accent-green); box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-green) 12%, transparent); }
.provider-alternatives { display: flex; align-items: center; gap: 7px; margin-top: -5px; color: var(--text-tertiary); font-size: var(--font-size-3xs); }
.provider-alternatives button { padding: 3px 7px; border: 1px solid var(--border-color-muted); border-radius: var(--radius-xs); color: var(--text-secondary); background: var(--bg-raised); cursor: pointer; font-size: var(--font-size-3xs); }
.provider-alternatives button:hover { border-color: var(--accent-primary); color: var(--text-primary); }
.optional-field { margin-top: 2px; }
.actions-row { display: flex; align-items: center; gap: 15px; margin-top: 12px; }
.actions-row .primary-action { margin-left: auto; }
.actions-row.end { justify-content: flex-end; }
.inline-error { margin: 0; padding: 10px 12px; border: 1px solid color-mix(in srgb, var(--accent-red) 24%, transparent); border-radius: var(--radius-sm); color: color-mix(in srgb, var(--accent-red) 86%, var(--text-primary)); background: color-mix(in srgb, var(--accent-red) 7%, transparent); font-size: var(--font-size-3xs); line-height: 1.45; }
.error-detail { margin: 6px 0 0; font-size: var(--font-size-3xs); color: var(--text-tertiary); }
.error-detail summary { cursor: pointer; }
.error-detail pre { margin: 6px 0 0; padding: var(--space-sm); border-radius: var(--radius-sm); background: var(--bg-raised); overflow-x: auto; white-space: pre-wrap; overflow-wrap: anywhere; }

.service-map { position: relative; display: grid; grid-template-columns: 112px 1fr; gap: 44px; align-items: center; margin: 28px 0; padding: 22px; border: 1px solid var(--border-color-muted); border-radius: var(--radius-lg); background: color-mix(in srgb, var(--bg-raised) 55%, transparent); overflow: hidden; }
.service-map.scanning::after { content: ''; position: absolute; inset: 0; pointer-events: none; background: linear-gradient(100deg, transparent 35%, color-mix(in srgb, var(--accent-primary) 8%, transparent), transparent 65%); transform: translateX(-100%); animation: scan 2s infinite; }
@keyframes scan { to { transform: translateX(100%); } }
.service-origin { position: relative; z-index: 1; display: grid; justify-items: center; gap: 8px; }
.service-origin > span { display: grid; place-items: center; width: 54px; height: 54px; border: 1px solid color-mix(in srgb, var(--accent-primary) 45%, var(--border-color)); border-radius: 17px 17px 17px 6px; color: var(--accent-primary); background: color-mix(in srgb, var(--accent-primary) 9%, var(--bg-base)); font-size: 21px; font-weight: var(--weight-strong); }
.service-origin small { max-width: 100px; overflow: hidden; color: var(--text-tertiary); text-overflow: ellipsis; font-size: var(--font-size-3xs); white-space: nowrap; }
.service-line { position: absolute; left: 100px; top: 50%; width: 78px; height: 1px; background: linear-gradient(90deg, var(--accent-primary), var(--border-color)); }
.service-list { display: grid; gap: 7px; }
.service-row { display: flex; align-items: center; gap: 10px; min-height: 38px; padding: 0 11px; border: 1px solid transparent; border-radius: var(--radius-sm); background: color-mix(in srgb, var(--bg-base) 58%, transparent); }
.service-row.needs_input { border-color: color-mix(in srgb, var(--accent-amber) 25%, transparent); }
.service-state { display: grid; place-items: center; width: 20px; height: 20px; border-radius: 50%; color: var(--accent-green); background: color-mix(in srgb, var(--accent-green) 10%, transparent); font-size: var(--font-size-3xs); }
.needs_input .service-state { color: var(--accent-amber); background: color-mix(in srgb, var(--accent-amber) 10%, transparent); }
.not_found { opacity: .62; }
.not_found .service-state { color: var(--text-tertiary); background: var(--bg-overlay); }
.checking .service-state { color: var(--accent-primary); background: color-mix(in srgb, var(--accent-primary) 10%, transparent); }
.service-name { display: grid; gap: 2px; min-width: 0; }
.service-name strong { font-size: var(--font-size-3xs); font-weight: var(--weight-semibold); }
.service-name small { max-width: 275px; overflow: hidden; color: var(--text-tertiary); text-overflow: ellipsis; font-size: var(--font-size-3xs); white-space: nowrap; }
.service-count { margin-left: auto; color: var(--text-tertiary); font-family: var(--font-mono); font-size: var(--font-size-3xs); }
.spinner { width: 9px; height: 9px; border: 1.5px solid color-mix(in srgb, var(--accent-primary) 25%, transparent); border-top-color: var(--accent-primary); border-radius: 50%; animation: spin .8s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }

.manual-card { padding: 15px; border: 1px solid color-mix(in srgb, var(--accent-amber) 22%, var(--border-color)); border-radius: var(--radius-md); background: color-mix(in srgb, var(--accent-amber) 3%, var(--bg-raised)); }
.manual-heading { display: flex; justify-content: space-between; width: 100%; padding: 0 0 12px; border: 0; background: none; text-align: left; }
.manual-heading > span:first-child { display: grid; gap: 3px; }
.manual-heading strong { font-size: var(--font-size-2xs); }
.manual-heading small { color: var(--text-tertiary); font-size: var(--font-size-3xs); }
.manual-grid { display: grid; grid-template-columns: minmax(0, 1fr) 76px; gap: 10px; }
.manual-grid--wide { grid-template-columns: minmax(0, 1fr); }
.manual-grid label { display: grid; gap: 5px; }
.manual-grid input { padding: 9px 10px; font-size: var(--font-size-2xs); }
.manual-actions { display: flex; justify-content: space-between; align-items: center; margin-top: 12px; }
.manual-buttons { display: flex; align-items: center; gap: 8px; }
.inline-success { margin-top: 11px; color: var(--accent-green); font-size: var(--font-size-2xs); }
.manual-card .inline-error { margin-top: 11px; }

.preference-section { margin-top: 32px; }
.section-label { display: block; margin-bottom: 9px; }
.theme-choice { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.theme-choice button { display: grid; grid-template-columns: 85px 1fr; align-items: center; gap: 14px; padding: 12px; border: 1px solid var(--border-color); border-radius: var(--radius-md); color: var(--text-secondary); background: var(--bg-raised); cursor: pointer; text-align: left; }
.theme-choice button.selected { border-color: var(--accent-primary); color: var(--text-primary); box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-primary) 8%, transparent); }
.theme-choice strong { font-size: var(--font-size-2xs); }
.theme-preview { position: relative; display: block; height: 48px; overflow: hidden; border-radius: var(--radius-xs); }
/* Both swatches show at once in the theme picker, so neither may follow the
   active theme — hard-coded on purpose, not a missing token. */
.theme-preview.dark { background: #141922; border: 1px solid #293140; }
.theme-preview.light { background: #f2f3f5; border: 1px solid #d9dce2; }
.theme-preview::before { content: ''; position: absolute; inset: 0 auto 0 0; width: 26%; background: color-mix(in srgb, var(--accent-primary) 12%, #11151d); }
.theme-preview.light::before { background: #e3e6eb; }
.theme-preview i { position: relative; display: block; width: 48%; height: 3px; margin: 9px 0 0 38%; border-radius: 3px; background: #536073; }
.theme-preview.light i { background: #b3b7bf; }
.theme-preview i:nth-child(2) { width: 33%; }
.preference-toggle { display: flex; align-items: center; justify-content: space-between; gap: 16px; width: 100%; margin-top: 16px; padding: 16px; border: 1px solid var(--border-color); border-radius: var(--radius-md); background: var(--bg-raised); text-align: left; }
.preference-toggle > span:first-child { display: grid; gap: 5px; }
.preference-toggle strong { font-size: var(--font-size-2xs); }
.preference-toggle small { color: var(--text-tertiary); font-size: var(--font-size-3xs); }
kbd { padding: 1px 4px; border: 1px solid var(--border-color); border-radius: var(--radius-xs); background: var(--bg-base); font-family: var(--font-mono); font-size: var(--font-size-3xs); }

.ready-panel { text-align: center; }
.ready-seal { display: grid; place-items: center; width: 72px; height: 72px; margin: 0 auto 22px; border: 1px solid color-mix(in srgb, var(--accent-green) 34%, transparent); border-radius: 50%; background: radial-gradient(circle, color-mix(in srgb, var(--accent-green) 15%, transparent), transparent 70%); box-shadow: 0 0 0 9px color-mix(in srgb, var(--accent-green) 4%, transparent); }
.ready-seal span { display: grid; place-items: center; width: 38px; height: 38px; border-radius: 50%; color: var(--accent-green); background: color-mix(in srgb, var(--accent-green) 11%, transparent); font-size: 18px; }
.ready-panel .panel-copy { margin-right: auto; margin-left: auto; }
.completion-card { margin: 27px auto 24px; padding: 15px; border: 1px solid var(--border-color); border-radius: var(--radius-md); background: var(--bg-raised); text-align: left; }
.completion-account { display: flex; align-items: center; gap: 11px; }
.completion-account > span { display: grid; place-items: center; width: 35px; height: 35px; border-radius: 10px 10px 10px 4px; color: var(--accent-primary); background: color-mix(in srgb, var(--accent-primary) 10%, transparent); font-weight: var(--weight-bold); }
.completion-account > div { display: grid; gap: 3px; }
.completion-account strong { font-size: var(--font-size-2xs); }
.completion-account small { color: var(--text-tertiary); font-size: var(--font-size-3xs); }
.completion-checks { display: flex; flex-wrap: wrap; gap: 7px; margin-top: 13px; padding-top: 12px; border-top: 1px solid var(--border-color-muted); }
.completion-checks span { color: var(--accent-green); font-size: var(--font-size-3xs); }
.ready-actions { display: grid; justify-items: center; gap: 10px; }
.ready-actions small { color: var(--text-tertiary); font-size: var(--font-size-3xs); }
.launch { min-width: 170px; }

.step-enter-active, .step-leave-active { transition: opacity var(--duration-normal) var(--ease-out), transform var(--duration-normal) var(--ease-out); }
.step-enter-from { opacity: 0; transform: translateY(10px); }
.step-leave-to { opacity: 0; transform: translateY(-7px); }
.onboarding-leave-active { transition: opacity var(--duration-slow) var(--ease-out); }
.onboarding-leave-to { opacity: 0; }

@media (max-width: 760px) {
  /* On a phone the wizard is not a dialog floating above something — it *is*
     the screen. Keeping the card treatment only drew a rounded border a few
     pixels inside the display edge, which reads as a rendering glitch rather
     than as a card, and cost a gutter the content needed. */
  .onboarding-shell { padding: 0; place-items: stretch; }
  .wizard-frame {
    grid-template-columns: 1fr;
    width: 100%;
    min-height: 100dvh;
    border: 0;
    border-radius: 0;
    box-shadow: none;
  }
  .account-setup-shell .wizard-frame { min-height: 100dvh; }
  .wizard-rail { display: none; }
  .wizard-close {
    top: calc(17px + var(--satchel-safe-area-top));
    right: calc(17px + var(--satchel-safe-area-right));
  }
  .wizard-content {
    padding:
      calc(40px + var(--satchel-safe-area-top))
      calc(24px + var(--satchel-safe-area-right))
      calc(28px + var(--satchel-safe-area-bottom))
      calc(24px + var(--satchel-safe-area-left));
  }
  /* Absolutely positioned at the top of the content box, so on an edge-to-edge
     phone it would otherwise run underneath the system status bar. */
  .progress-track { top: var(--satchel-safe-area-top); }
  /* Start at the top instead of centring: on a tall screen the vertical
     centring left the heading floating in the middle with dead space above
     and below it. */
  .panel { align-self: start; margin: 0; }
  .service-map { grid-template-columns: 1fr; gap: 18px; }
  .service-line { display: none; }
  .theme-choice { grid-template-columns: 1fr; }
}

</style>
