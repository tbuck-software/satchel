<script setup lang="ts">
import { useTextareaAutosize } from '@vueuse/core';
import { defineAsyncComponent, computed, nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue';
import type { AnchorBox } from '@/utils/popoverPlacement';
import { attachmentBytes, encodeBase64 } from '@/utils/attachmentBytes';
import { useI18n } from 'vue-i18n';
import { useRoute, useRouter } from 'vue-router';
import type {
  BackendDraft,
  BackendOutgoingAttachment,
  BackendSendMessageResult,
} from '@/types/composer';
import { useAccountsStore } from '@/stores/accounts';
import { useAliasesStore } from '@/stores/aliases';
import { useDraftsStore } from '@/stores/drafts';
import { bodyHtmlWithInlineImages, useMessagesStore } from '@/stores/messages';
import { useSettingsStore } from '@/stores/settings';
import { useTemplatesStore } from '@/stores/templates';
import { useThreadsStore } from '@/stores/threads';
import RecipientChips from '@/components/mail/RecipientChips.vue';
import AttachmentDropZone from '@/components/mail/AttachmentDropZone.vue';
import TemplatePicker from '@/components/mail/TemplatePicker.vue';
import RichTextEditor from '@/components/mail/RichTextEditor.vue';
import ComposerSignature from '@/components/mail/ComposerSignature.vue';
import ComposerThreadContext from '@/components/mail/ComposerThreadContext.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import AppBanner from '@/components/ui/AppBanner.vue';
import AppButton from '@/components/ui/AppButton.vue';
import SplitButton from '@/components/ui/SplitButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { composerHtmlFromText, hasComposerHtmlHistory, hasComposerTextHistory } from '@/utils/composerHistory';
import { useComposerBody } from '@/composables/useComposerBody';
import type { ComposerMode } from '@/stores/composer';
import { useAttachmentActions } from '@/composables/useAttachmentActions';
import { useComposerCloseGuard } from '@/composables/useComposerCloseGuard';
import {
  useComposerDraftPersistence,
  type ComposerStatus,
} from '@/composables/useComposerDraftPersistence';
import {
  useComposerCrypto,
  type ComposerCryptoMethod,
} from '@/composables/useComposerCrypto';
import { useComposerInlineAttachments, type ComposerInlineAttachmentEditor } from '@/composables/useComposerInlineAttachments';
import { useComposerLocalBackup } from '@/composables/useComposerLocalBackup';
import { useDraftEditorClaim } from '@/composables/useDraftEditorClaim';
import { useComposerSignature } from '@/composables/useComposerSignature';
import { useComposerTransport } from '@/composables/useComposerTransport';
import { useToast } from '@/composables/useToast';
import { useToastClearance } from '@/composables/useToastClearance';
import { outboxErrorLabel } from '@/utils/outboxLabels';
import {
  formatTimestamp,
  mergeRecipients,
} from '@/utils/composerText';
import { ariaKeyshortcuts, bindingKeycaps, formatBinding } from '@/config/keybindings';
import { composerSendBlocker } from '@/utils/composerSendBlocker';
import { deferredChangeTracker } from '@/utils/deferredChangeTracker';
import {
  findResendSource,
  resendEditorHtml,
  forwardThreadPrefill,
  normalizeOwnedAddresses,
  replyThreadPrefill,
} from '@/utils/composerThreadPrefill';
import { composerDraftInput, composerDraftSnapshot, composerRecoveryContent, type ComposerDraftState } from '@/utils/composerDraftState';
import type { ComposerBackup, ComposerBackupContent } from '@/persistence/composerBackup';

const SendLaterMenu = defineAsyncComponent(() => import('@/components/mail/SendLaterMenu.vue'));

const props = withDefaults(defineProps<{
  embedded?: boolean;
  mode?: ComposerMode;
  threadId?: string | null;
  initialDraftId?: string | null;
  initialTo?: string[];
  initialCc?: string[];
  initialBcc?: string[];
  initialSubject?: string;
  initialBody?: string;
  initialBodyHtml?: string | null;
}>(), {
  embedded: false,
  mode: 'new',
  threadId: null,
  initialDraftId: null,
  initialTo: () => [],
  initialCc: () => [],
  initialBcc: () => [],
  initialSubject: '',
  initialBody: '',
  initialBodyHtml: null,
});

const emit = defineEmits<{
  sent: [];
  close: [];
  'update:title': [value: string];
  /** What a draft opened as a new message turned out to be once it loaded. */
  'update:mode': [value: ComposerMode];
}>();

const route = useRoute();
const router = useRouter();
const { t } = useI18n();

const accountsStore = useAccountsStore();
const aliasesStore = useAliasesStore();
const settings = useSettingsStore();
const templatesStore = useTemplatesStore();
const draftsStore = useDraftsStore();
const threadsStore = useThreadsStore();
const messagesStore = useMessagesStore();
const attachmentActions = useAttachmentActions();
const composerTransport = useComposerTransport();
const toast = useToast();
const supportsBackend = composerTransport.commandsAvailable();

const accountId = ref<string>('');
const aliasEmail = ref<string | null>(null);
const to = ref<string[]>([...props.initialTo]);
const cc = ref<string[]>([...props.initialCc]);
const bcc = ref<string[]>([...props.initialBcc]);
const showCcBcc = ref(cc.value.length > 0 || bcc.value.length > 0);
const subject = ref(props.initialSubject);
const body = ref(props.initialBody);
const bodyHtml = ref<string | null>(props.initialBodyHtml);
const { writingText, writingHtml, historyText, historyHtml, removeHistory } = useComposerBody(body, bodyHtml);
const { textarea: writingTextarea } = useTextareaAutosize({ input: writingText });
const { textarea: historyTextarea } = useTextareaAutosize({ input: historyText });
const contextThreadId = ref<string | null>(props.threadId);
const replyToMessageId = ref<string | null>(null);
const attachments = ref<BackendOutgoingAttachment[]>([]);
const sending = ref(false);
const discarding = ref(false);
const status = ref<ComposerStatus | null>(null);
const scheduledFor = ref<string | null>(null);
const followUpAt = ref<string | null>(null);
const showTemplatePicker = ref(false);
const showSendLater = ref(false);
const sendLaterAnchor = ref<AnchorBox | null>(null);

function openSendLater(event: MouseEvent | KeyboardEvent) {
  const trigger = event.currentTarget as HTMLElement;
  trigger.focus({ preventScroll: true });
  const rect = trigger.getBoundingClientRect();
  sendLaterAnchor.value = { left: rect.right, right: rect.right, top: rect.top, bottom: rect.top };
  showSendLater.value = true;
}
const priority = ref<'high' | 'normal' | 'low'>('normal');
/**
 * Plain-text composition. Seeded from the global preference but switchable per
 * message, like Outlook and Thunderbird. When on, no HTML body is sent at all
 * — not an HTML body that merely looks plain.
 */
const plainTextCompose = ref(settings.composePlainText);
const richEditorRef = ref<InstanceType<typeof RichTextEditor> | null>(null);
const historyEditorRef = ref<InstanceType<typeof RichTextEditor> | null>(null);
/**
 * Finding the quote parses the whole body, so it is looked for once typing
 * pauses rather than on every keystroke.
 */
const quotedHistory = deferredChangeTracker(
  () => Boolean(contextThreadId.value && (plainTextCompose.value
    ? hasComposerTextHistory(body.value)
    : hasComposerHtmlHistory(bodyHtml.value))),
  (value) => { hasQuotedHistory.value = value; },
  250,
);
const hasQuotedHistory = ref(quotedHistory.current());
/**
 * A one-message thread that is already quoted below: "Show conversation" would
 * open the same message a second time.
 */
const quoteHoldsWholeThread = computed(() => {
  const threadId = contextThreadId.value;
  if (!threadId || !hasQuotedHistory.value) return false;
  return messagesStore.getByThread(threadId).filter((message) => !message.isDraft).length <= 1;
});

async function removeQuotedHistory() {
  removeHistory();
  await nextTick();
  quotedHistory.settle();
}
const toFieldRef = ref<InstanceType<typeof RecipientChips> | null>(null);

/**
 * Where a fresh composer starts typing.
 *
 * A reply, a forward and a resend already know who they are going to, so the
 * body is the first empty thing. "New mail" does not: `n` followed by an
 * address used to put the address in the message.
 */
const startsInRecipients = computed(
  () => props.mode === 'new' && props.initialTo.length === 0 && !props.initialDraftId,
);
const attachmentZoneRef = ref<InstanceType<typeof AttachmentDropZone> | null>(null);
// Send and the composer's own status line: no toast may sit on them.
const footerRef = ref<HTMLElement | null>(null);
useToastClearance(footerRef);
const composerInlineAttachments = useComposerInlineAttachments(
  attachments,
  (): ComposerInlineAttachmentEditor => ({
    insertAttachmentImage: (image) => richEditorRef.value?.insertAttachmentImage(image),
    removeAttachmentImage: (image) => {
      richEditorRef.value?.removeAttachmentImage(image);
      historyEditorRef.value?.removeAttachmentImage(image);
    },
  }),
);
const initialized = ref(false);
// A loaded reply or draft shows its history control at once, not after a pause.
watch(initialized, (ready) => {
  if (ready) quotedHistory.settle();
}, { flush: 'post' });

const accounts = computed(() => accountsStore.emailAccounts);
const activeAccount = computed(() =>
  accounts.value.find((a) => a.id === accountId.value) ?? accounts.value[0],
);
const accountAliases = computed(() =>
  activeAccount.value ? aliasesStore.aliasesFor(activeAccount.value.id) : [],
);
/**
 * One flat "Von" picker over every identity — each account's primary address
 * plus its aliases — like Gmail's and Apple Mail's From menu, instead of two
 * chained selects for account and alias.
 */
const identityChoices = computed(() =>
  accounts.value.map((account) => ({
    account,
    options: [
      {
        key: `${account.id}|${account.email.toLowerCase()}`,
        label: account.email,
        accountId: account.id,
        aliasEmail: null as string | null,
      },
      ...aliasesStore.aliasesFor(account.id).map((alias) => ({
        key: `${account.id}|${alias.email.toLowerCase()}`,
        label: t('compose.identityAlias', { email: alias.email }),
        accountId: account.id,
        aliasEmail: alias.email as string | null,
      })),
    ],
  })),
);

const currentIdentityKey = computed(
  () => `${accountId.value}|${fromAddress.value.toLowerCase()}`,
);

function onIdentitySelected(event: Event) {
  const key = (event.target as HTMLSelectElement).value;
  const choice = identityChoices.value
    .flatMap((group) => group.options)
    .find((option) => option.key === key);
  if (!choice) return;
  if (choice.accountId !== accountId.value) contextThreadId.value = null;
  accountId.value = choice.accountId;
  aliasEmail.value = choice.aliasEmail;
}
const fromAddress = computed(() => aliasEmail.value ?? activeAccount.value?.email ?? '');
const draftReplyTo = ref<string | null>(null);
const replyTo = computed(() => draftReplyTo.value ?? (aliasEmail.value ? activeAccount.value?.email ?? null : null));
const {
  load: loadSignatures,
  availableSignatures,
  selectedSignatureId,
  signatureVariant,
  signaturePreviewLoading,
  signatureOverrideText,
  signatureOverrideHtml,
  signatureDisplayHtml,
  signatureDisplayText,
  signatureOverrideActive,
  clearOverride: clearSignatureOverride,
  syncSelection: syncSignatureSelection,
  refreshPreview: refreshSignaturePreview,
  select: onSignatureSelect,
  selectVariant: onSignatureVariantChange,
  edit: onSignatureEdit,
} = useComposerSignature({
  activeAccount,
  fromAddress,
  contextThreadId,
  supportsBackend,
  transport: composerTransport,
});
const {
  load: loadCrypto,
  smimeSignCertId,
  smimeEncrypt,
  pgpSignKeyId,
  pgpEncrypt,
  cryptoMethod,
  cryptoOptions,
  canSignSelected,
  canEncryptSelected,
  cryptoSign,
  cryptoEncrypt,
  cryptoHint,
} = useComposerCrypto({
  to,
  cc,
  bcc,
  fromAddress,
  t,
});
const {
  draftId,
  serverRevision: draftRevision,
  saveError,
  saveAsNewDraft,
  persist: persistDraft,
  startAutosave,
  pause: pauseDraftPersistence,
  resume: resumeDraftPersistence,
  finish: finishDraftPersistence,
  discard: discardPersistedDraft,
  closeWithoutSaving: closeDraftWithoutSaving,
} = useComposerDraftPersistence({
  initialized,
  sending,
  status,
  buildInput: () => activeAccount.value ? composerDraftInput(currentDraftState()) : null,
  drafts: draftsStore,
  t,
  onCaughtUp: () => localBackup.clear(),
  onSaved: (input) => markCloseBaselineSaved(input),
});
const {
  captureBaseline: captureCloseBaseline,
  markSaved: markCloseBaselineSaved,
  hasMeaningfulContent,
  shouldConfirmClose,
  closeWithoutSaving: closeGuardWithoutSaving,
  installStandaloneCloseGuard,
  isPopout,
  closePopout,
} = useComposerCloseGuard({
  embedded: () => props.embedded,
  standalone: () => route.query.standalone === '1',
  supportsBackend,
  initialized,
  currentState: currentDraftState,
  persistDraft,
  finishDraftPersistence,
  closeDraftWithoutSaving,
});
const localBackup = useComposerLocalBackup({
  enabled: initialized,
  draftId,
  content: backupContent,
});
const editorClaim = useDraftEditorClaim(draftId, {
  // Another window has this draft and was brought forward; this one steps aside.
  onRefused: () => {
    toast.info(t('compose.draft.openElsewhere'));
    void leaveComposer();
  },
});

async function closeWithoutSaving() {
  localBackup.clear();
  await closeGuardWithoutSaving();
}

/** Leaves without saving: embedded composers close, popouts close their window. */
async function leaveComposer() {
  await closeWithoutSaving();
  if (props.embedded) emit('close');
  else if (isPopout()) await closePopout();
  else await router.push({ name: 'inbox' });
}

/** A draft that could not be loaded must not be sent as an empty message. */
const draftLoadFailed = ref(false);
const sendBlocker = computed(() => composerSendBlocker({
  initialized: initialized.value,
  sending: sending.value,
  draftLoadFailed: draftLoadFailed.value,
  hasAccount: Boolean(activeAccount.value),
  to: to.value,
  cc: cc.value,
  bcc: bcc.value,
  body: body.value,
  attachmentCount: attachments.value.length,
}));
const canSend = computed(() => sendBlocker.value === null);
/** What the sender can do about a disabled Send button, if anything. */
const sendBlockedHint = computed(() => {
  const blocker = sendBlocker.value;
  if (!blocker) return null;
  switch (blocker.reason) {
    case 'draftLoadFailed': return t('compose.sendBlocked.draftLoadFailed');
    case 'noAccount': return t('compose.sendBlocked.noAccount');
    case 'noRecipients': return t('compose.sendBlocked.noRecipients');
    case 'invalidRecipients':
      return t('compose.sendBlocked.invalidRecipients', { addresses: blocker.invalid.join(', ') });
    case 'empty': return t('compose.sendBlocked.empty');
    default: return null;
  }
});
const sendHintId = `compose-send-hint-${useId()}`;
const senderFieldId = `compose-sender-${useId()}`;
const subjectFieldId = `compose-subject-${useId()}`;
const sendShortcut = {
  label: formatBinding('mod+enter'),
  keys: bindingKeycaps('mod+enter'),
  aria: ariaKeyshortcuts('mod+enter'),
};

watch(accountId, () => {
  // Keep the alias only when it belongs to the newly selected account; the
  // identity picker sets both values together.
  if (aliasEmail.value && !accountAliases.value.some((alias) => alias.email === aliasEmail.value)) {
    aliasEmail.value = null;
  }
  if (initialized.value) syncSignatureSelection();
});

watch(aliasEmail, () => {
  if (initialized.value) syncSignatureSelection();
});

function ownedAddresses() {
  return normalizeOwnedAddresses([
    ...accountsStore.accounts.map((account) => account.email),
    ...aliasesStore.items.map((alias) => alias.email),
  ]);
}

async function loadDraft() {
  const id = props.initialDraftId || (route.query.draftId as string | undefined);
  if (!id) return;
  const draft = await draftsStore.fetchOne(id);
  if (!draft) return;
  if (draft.send_cancelled) toast.info(t('compose.schedule.editCancelled'));
  await applyDraft(draft);
  // A reply carries the message it answers.
  emit('update:mode', draft.send_options?.reply_to_message_id ? 'reply' : 'new');
}

async function applyDraft(draft: BackendDraft) {
  draftId.value = draft.id;
  draftRevision.value = draft.revision ?? null;
  contextThreadId.value = draft.thread_id ?? null;
  accountId.value = draft.account_id;
  aliasEmail.value =
    draft.from_address && draft.from_address !== activeAccount.value?.email
      ? draft.from_address
      : null;
  to.value = draft.to ?? [];
  cc.value = draft.cc ?? [];
  bcc.value = draft.bcc ?? [];
  showCcBcc.value = cc.value.length > 0 || bcc.value.length > 0;
  subject.value = draft.subject ?? '';
  body.value = draft.body ?? '';
  plainTextCompose.value = !draft.body_html;
  attachments.value = draft.attachments ?? [];
  bodyHtml.value = composerInlineAttachments.hydrateInlineImages(
    draft.body_html ?? null,
    attachments.value,
  );
  scheduledFor.value = draft.scheduled_for ?? null;
  draftReplyTo.value = draft.reply_to ?? null;
  replyToMessageId.value = draft.send_options?.reply_to_message_id ?? null;
  followUpAt.value = draft.send_options?.follow_up_at ?? null;
  cryptoMethod.value = draft.send_options?.smime_sign_with_cert_id || draft.send_options?.smime_encrypt_to_recipients
    ? 'smime' : draft.send_options?.pgp_sign_with_key_id || draft.send_options?.pgp_encrypt_to_recipients ? 'pgp' : 'none';
  await nextTick();
  smimeSignCertId.value = draft.send_options?.smime_sign_with_cert_id ?? null;
  smimeEncrypt.value = draft.send_options?.smime_encrypt_to_recipients ?? false;
  pgpSignKeyId.value = draft.send_options?.pgp_sign_with_key_id ?? null;
  pgpEncrypt.value = draft.send_options?.pgp_encrypt_to_recipients ?? false;
  priority.value = draft.send_options?.priority ?? 'normal';
  selectedSignatureId.value = draft.signature_id ?? null;
  signatureVariant.value = draft.signature_variant ?? (selectedSignatureId.value ? 'auto' : 'none');
  signatureOverrideText.value = draft.signature_override_text ?? null;
  signatureOverrideHtml.value = draft.signature_override_html ?? null;
}

function defaultComposerTitle() {
  if (props.mode === 'forward') return t('common.forward');
  if (props.mode === 'resend') return t('compose.title.resend');
  if (props.mode.startsWith('reply')) return t('common.reply');
  return t('compose.title.new');
}

function appendThreadBody(prefill: { body: string | null; bodyHtml: string | null }) {
  if (prefill.body === null) return;
  // Keep initial text and anything typed while older bodies were loading.
  const paragraph = document.createElement('p');
  paragraph.textContent = body.value;
  const existingHtml = bodyHtml.value || (body.value
    ? paragraph.outerHTML.replace(/\r\n?|\n/g, '<br>')
    : '');
  body.value += prefill.body;
  bodyHtml.value = existingHtml + (prefill.bodyHtml ?? '');
}

async function initializeThreadContext() {
  const threadId = props.threadId;
  if (!threadId || props.mode === 'new') return;
  contextThreadId.value = threadId;
  if (!threadsStore.threads.some((thread) => thread.id === threadId)) {
    await threadsStore.load();
  }
  await messagesStore.loadByThread(threadId);
  // Thread loading prioritizes the newest bodies. Quotes need the complete
  // older messages too, rather than the snippets in their preview rows.
  if (props.mode !== 'resend') {
    const missing = messagesStore.getByThread(threadId)
      .filter((message) => !message.isDraft && message.bodyLoaded === false)
      .map((message) => message.id);
    if (missing.length) await messagesStore.loadAttachmentsForMessages(threadId, missing);
  }
  const thread = threadsStore.threads.find((item) => item.id === threadId);
  const threadMessages = messagesStore.getByThread(threadId);
  if (thread?.accountId) accountId.value = thread.accountId;

  if (props.mode === 'resend') {
    const own = ownedAddresses();
    let source = findResendSource(threadMessages, own);
    contextThreadId.value = null;
    if (!source) return;
    await messagesStore.loadAttachmentsForMessages(threadId, [source.id]);
    source = messagesStore.getByThread(threadId).find((message) => message.id === source?.id) ?? source;
    to.value = source.to.map((participant) => participant.email);
    cc.value = (source.cc ?? []).map((participant) => participant.email);
    showCcBcc.value = cc.value.length > 0;
    subject.value = source.subject || thread?.subject || '';
    body.value = source.bodyText ?? '';
    bodyHtml.value = resendEditorHtml(bodyHtmlWithInlineImages(source));
    const copiedAttachments: Array<BackendOutgoingAttachment | null> = await Promise.all(source.attachments.map(async (attachment) => {
      try {
        const dataUrl = attachment.dataUrl;
        const downloaded = dataUrl
          ? null
          : await attachmentActions.downloadAttachment(attachment.id);
        // A forwarded attachment over 4 MiB arrives as a cache path, not
        // base64. Reading it back here is what keeps it on the message.
        const contentBase64 = dataUrl?.split(',')[1]
          ?? (downloaded ? encodeBase64(await attachmentBytes(downloaded)) : undefined);
        if (!contentBase64) return null;
        return {
          filename: downloaded?.filename || attachment.filename,
          mime_type: downloaded?.mime_type || attachment.contentType,
          content_base64: contentBase64,
          content_id: attachment.contentId ?? null,
          inline: Boolean(attachment.isInline),
        } satisfies BackendOutgoingAttachment;
      } catch {
        return null;
      }
    }));
    attachments.value = copiedAttachments.filter(
      (attachment): attachment is BackendOutgoingAttachment => attachment !== null,
    );
    return;
  }

  if (props.mode === 'forward') {
    const prefill = forwardThreadPrefill(threadMessages, thread?.subject ?? '');
    subject.value = prefill.subject;
    appendThreadBody(prefill);
    return;
  }

  const prefill = replyThreadPrefill(
    props.mode === 'reply-all' ? 'reply-all' : 'reply',
    threadMessages,
    thread?.subject ?? '',
    ownedAddresses(),
  );
  subject.value = prefill.subject;
  to.value = prefill.to;
  cc.value = prefill.cc;
  showCcBcc.value = prefill.cc.length > 0;
  replyToMessageId.value = prefill.replyToMessageId;
  appendThreadBody(prefill);
}

type SetupPart = 'accounts' | 'signatures' | 'aliases' | 'templates' | 'security';

/** Side loads that fail leave their feature empty, not the composer dead. */
const failedSetupParts = ref<SetupPart[]>([]);
const retryingSetup = ref(false);

function setupLoaders(): Array<[SetupPart, () => Promise<unknown>]> {
  const loaders: Array<[SetupPart, () => Promise<unknown>]> = [
    ['accounts', () => accountsStore.load()],
    ['signatures', () => loadSignatures()],
    ['aliases', () => aliasesStore.load()],
    ['templates', () => templatesStore.load()],
  ];
  if (supportsBackend) loaders.push(['security', () => loadCrypto()]);
  return loaders;
}

async function runSetupLoads(only?: SetupPart[]) {
  const loaders = setupLoaders().filter(([part]) => !only || only.includes(part));
  const results = await Promise.allSettled(loaders.map(([, load]) => load()));
  failedSetupParts.value = loaders
    .filter(([part], index) => {
      const result = results[index];
      if (result?.status !== 'rejected') return false;
      console.warn(`composer setup: ${part} failed to load`, result.reason);
      return true;
    })
    .map(([part]) => part);
  if (!accountId.value) {
    accountId.value = accountsStore.activeAccountId || accountsStore.accounts[0]?.id || '';
  }
}

const SETUP_PART_KEYS: Record<SetupPart, string> = {
  accounts: 'compose.setup.parts.accounts',
  signatures: 'compose.setup.parts.signatures',
  aliases: 'compose.setup.parts.aliases',
  templates: 'compose.setup.parts.templates',
  security: 'compose.setup.parts.security',
};

const setupErrorDetail = computed(() =>
  failedSetupParts.value.map((part) => t(SETUP_PART_KEYS[part])).join(', '),
);

async function retrySetup() {
  if (retryingSetup.value) return;
  retryingSetup.value = true;
  try {
    await runSetupLoads([...failedSetupParts.value]);
    syncSignatureSelection();
    await refreshSignaturePreview();
  } finally {
    retryingSetup.value = false;
  }
}

onMounted(async () => {
  const fromDraft = Boolean(props.initialDraftId || route.query.draftId);
  try {
    await runSetupLoads();
    syncSignatureSelection();
    try {
      if (fromDraft) await loadDraft();
      else await initializeThreadContext();
    } catch (err) {
      const text = outboxErrorLabel(err instanceof Error ? err.message : String(err));
      status.value = { tone: 'error', text };
      toast.error(text);
      draftLoadFailed.value = fromDraft;
    }
    if (!fromDraft) syncSignatureSelection();
    await refreshSignaturePreview();
    await nextTick();
  } finally {
    // Whatever failed above, typing from here on is tracked, autosaved and
    // guarded on close.
    captureCloseBaseline();
    initialized.value = true;
    emit('update:title', subject.value || defaultComposerTitle());
    startAutosave();
  }
  offerLocalBackup();
  if (props.embedded && startsInRecipients.value && to.value.length === 0) {
    toFieldRef.value?.focus();
  }
  await installStandaloneCloseGuard();
});

function backupContent(): ComposerBackupContent {
  return composerRecoveryContent(currentDraftState());
}

function sameBackupText(a: ComposerBackupContent, b: ComposerBackupContent) {
  const key = (content: ComposerBackupContent) => composerDraftSnapshot({ ...content, attachments: [] });
  return key(a) === key(b);
}

/** A local copy a closed composer left behind, waiting for the user's call. */
const backupOffer = ref<ComposerBackup | null>(null);

function offerLocalBackup() {
  const blankNew = startsInRecipients.value && !props.initialSubject && !props.initialBody;
  if (!draftId.value && !blankNew) return;
  const orphan = localBackup.findOrphan(draftId.value);
  if (!orphan) return;
  if (sameBackupText(orphan.content, backupContent())) {
    localBackup.discardOrphan(orphan);
    return;
  }
  backupOffer.value = orphan;
}

async function restoreBackup() {
  const offer = backupOffer.value;
  if (!offer) return;
  backupOffer.value = null;
  // A blank composer takes over the draft the copy belongs to, if it still exists.
  if (offer.draftId && offer.draftId !== draftId.value) {
    const draft = await draftsStore.fetchOne(offer.draftId).catch(() => null);
    if (draft) await applyDraft(draft);
  }
  const { content } = offer;
  if (accounts.value.some((account) => account.id === content.accountId)) {
    accountId.value = content.accountId;
    aliasEmail.value = content.fromAddress && content.fromAddress !== activeAccount.value?.email
      ? content.fromAddress
      : null;
  }
  to.value = [...content.to];
  cc.value = [...content.cc];
  bcc.value = [...content.bcc];
  showCcBcc.value = cc.value.length > 0 || bcc.value.length > 0;
  subject.value = content.subject;
  plainTextCompose.value = content.plainText;
  body.value = content.body;
  bodyHtml.value = composerInlineAttachments.hydrateInlineImages(content.bodyHtml, attachments.value);
  contextThreadId.value = content.threadId;
  draftReplyTo.value = content.replyTo;
  scheduledFor.value = content.scheduledFor;
  replyToMessageId.value = content.sendOptions.reply_to_message_id ?? null;
  followUpAt.value = content.sendOptions.follow_up_at ?? null;
  cryptoMethod.value = content.sendOptions.smime_sign_with_cert_id || content.sendOptions.smime_encrypt_to_recipients
    ? 'smime' : content.sendOptions.pgp_sign_with_key_id || content.sendOptions.pgp_encrypt_to_recipients ? 'pgp' : 'none';
  await nextTick();
  smimeSignCertId.value = content.sendOptions.smime_sign_with_cert_id ?? null;
  smimeEncrypt.value = content.sendOptions.smime_encrypt_to_recipients ?? false;
  pgpSignKeyId.value = content.sendOptions.pgp_sign_with_key_id ?? null;
  pgpEncrypt.value = content.sendOptions.pgp_encrypt_to_recipients ?? false;
  priority.value = content.sendOptions.priority ?? 'normal';
  selectedSignatureId.value = content.signatureId;
  signatureVariant.value = content.signatureVariant;
  signatureOverrideText.value = content.signatureOverrideText;
  signatureOverrideHtml.value = content.signatureOverrideHtml;
  localBackup.discardOrphan(offer);
}

function dismissBackup() {
  if (backupOffer.value) localBackup.discardOrphan(backupOffer.value);
  backupOffer.value = null;
}

const backupOfferTitle = computed(() =>
  backupOffer.value
    ? t('compose.backup.title', { time: formatTimestamp(backupOffer.value.savedAt) })
    : '',
);

const retryingSave = ref(false);

async function retrySave() {
  retryingSave.value = true;
  try {
    await persistDraft({});
  } finally {
    retryingSave.value = false;
  }
}

async function saveConflictAsNewDraft() {
  retryingSave.value = true;
  try {
    await saveAsNewDraft();
  } finally {
    retryingSave.value = false;
  }
}

/** Undo Send, kept reachable in a popout until its window has passed. */
const popoutUndo = ref<{ queuedId: string; until: number } | null>(null);
/** What a popout says after a send that needs attention, in place of the form. */
const popoutNotice = ref<string | null>(null);
const undoSecondsLeft = ref(0);
const undoing = ref(false);
let undoTimer: ReturnType<typeof setInterval> | null = null;

function stopUndoTimer() {
  if (undoTimer !== null) clearInterval(undoTimer);
  undoTimer = null;
}

function tickUndoWindow() {
  const pending = popoutUndo.value;
  if (!pending) return;
  undoSecondsLeft.value = Math.max(0, Math.ceil((pending.until - Date.now()) / 1000));
  if (undoSecondsLeft.value === 0 && !undoing.value) {
    stopUndoTimer();
    void closePopout();
  }
}

function startPopoutUndo(queuedId: string, untilIso: string) {
  popoutUndo.value = { queuedId, until: Date.parse(untilIso) };
  tickUndoWindow();
  undoTimer = setInterval(tickUndoWindow, 250);
}

async function undoFromPopout() {
  const pending = popoutUndo.value;
  if (!pending || undoing.value) return;
  undoing.value = true;
  try {
    const draft = await composerTransport.undoSend(pending.queuedId);
    stopUndoTimer();
    toast.success(t('app.mail.sendCancelled'));
    // Reopen the restored draft in this window, from a clean composer.
    window.location.replace(
      router.resolve({ name: 'compose', query: { standalone: '1', draftId: draft.id } }).href,
    );
  } catch (err) {
    undoing.value = false;
    toast.error(t('app.mail.cancelFailed', {
      error: err instanceof Error ? err.message : String(err),
    }));
  }
}

onBeforeUnmount(stopUndoTimer);

/**
 * Where the composer goes once a send is committed. A popout closes: routing
 * it into the mailbox showed a sidebar with no stores behind it.
 */
async function leaveAfterSend(
  result: BackendSendMessageResult,
  location: { name: string; params?: Record<string, string> },
) {
  if (props.embedded) return;
  if (!isPopout()) {
    await router.push(location);
    return;
  }
  if (result.status === 'queued' && result.queued_id && result.undo_until) {
    startPopoutUndo(result.queued_id, result.undo_until);
  } else if (result.status === 'delivery_unknown' || result.status === 'delivered') {
    popoutNotice.value = result.status === 'delivered'
      ? t('compose.status.sentLocalSaveFailed')
      : t('outbox.status.deliveryUnknown');
  } else {
    await closePopout();
  }
}

function focusComposeBody(event: MouseEvent) {
  const target = event.target as HTMLElement;
  if (target.closest('.ProseMirror, [data-composer-signature], [data-composer-context], button, input, select, textarea, a, [contenteditable="true"]')) {
    return;
  }
  richEditorRef.value?.focusEditor();
}

watch(
  [accountId, aliasEmail, contextThreadId, selectedSignatureId, signatureVariant],
  () => {
    if (initialized.value) void refreshSignaturePreview();
  },
);

watch(subject, (value) => {
  emit('update:title', value || defaultComposerTitle());
});

async function send() {
  if (!canSend.value || !activeAccount.value) {
    // The shortcut has no greyed-out button to look at; say why nothing happened.
    if (sendBlockedHint.value) status.value = { tone: 'warning', text: sendBlockedHint.value };
    return;
  }
  sending.value = true;
  let sendCommitted = false;
  try {
    await pauseDraftPersistence();
    status.value = null;
    const result = await composerTransport.sendMessage({
      accountId: activeAccount.value.id,
      to: to.value,
      cc: cc.value,
      bcc: bcc.value,
      subject: subject.value.trim() || t('compose.noSubject'),
      body: body.value,
      bodyHtml: plainTextCompose.value
        ? null
        : composerInlineAttachments.serializeInlineImages(bodyHtml.value ?? composerHtmlFromText(body.value)),
      threadId: contextThreadId.value,
      replyToMessageId: replyToMessageId.value,
      attachments: attachments.value,
      fromAddress: aliasEmail.value,
      fromName: activeAccount.value.displayName,
      replyTo: replyTo.value,
      draftId: draftId.value,
      scheduledFor: scheduledFor.value,
      followUpAt: followUpAt.value,
      smimeSignWithCertId: smimeSignCertId.value,
      smimeEncryptToRecipients: smimeEncrypt.value,
      pgpSignWithKeyId: pgpSignKeyId.value,
      pgpEncryptToRecipients: pgpEncrypt.value,
      priority: priority.value,
      signatureId: selectedSignatureId.value,
      signatureVariant: signatureVariant.value,
      signatureOverrideText: signatureOverrideText.value,
      signatureOverrideHtml: plainTextCompose.value ? null : signatureOverrideHtml.value,
    });
    sendCommitted = true;
    finishDraftPersistence();
    localBackup.clear();
    // The draft has left this editor; Undo in another window may reopen it.
    editorClaim.release();
    if (result.status === 'sent') {
      status.value = {
        tone: 'success',
        text: t('compose.status.sentFrom', { address: fromAddress.value }),
      };
      await Promise.all([
        accountsStore.load(),
        threadsStore.load(),
        messagesStore.loadByThread(result.thread_id),
      ]);
      emit('sent');
      await leaveAfterSend(result, { name: 'thread', params: { id: result.thread_id } });
    } else if (result.status === 'delivery_unknown') {
      toast.error(t('outbox.status.deliveryUnknown'));
      emit('sent');
      await leaveAfterSend(result, { name: 'outbox' });
    } else if (result.status === 'delivered') {
      status.value = { tone: 'warning', text: t('compose.status.sentLocalSaveFailed') };
      emit('sent');
      await leaveAfterSend(result, { name: 'outbox' });
    } else if (result.status === 'scheduled') {
      status.value = {
        tone: 'info',
        text: t('compose.status.scheduledFor', { time: formatScheduled(scheduledFor.value) }),
      };
      emit('sent');
      await leaveAfterSend(result, { name: 'inbox' });
    } else {
      status.value = { tone: 'info', text: t('compose.status.queued') };
      emit('sent');
      await leaveAfterSend(result, { name: 'inbox' });
    }
  } catch (err) {
    if (sendCommitted) {
      console.warn('post-send refresh failed', err);
    } else {
      // The draft stays put and the window stays open: this is the one outcome
      // the user must not be able to mistake for progress.
      const raw = err instanceof Error ? err.message : String(err);
      const cause = outboxErrorLabel(raw) || t('errors.backendUnknown');
      const text = t('compose.status.sendFailed', { error: cause });
      status.value = { tone: 'error', text, raw: cause === raw ? null : raw };
      // Outlives the composer — closing the window must not close the news.
      toast.error(text);
    }
  } finally {
    if (!sendCommitted) resumeDraftPersistence();
    sending.value = false;
  }
}

function formatScheduled(value: string | null): string {
  return value ? formatTimestamp(value) : '';
}

async function discardDraft(options: { close?: boolean } = {}) {
  if (discarding.value) return;
  discarding.value = true;
  try {
    await discardPersistedDraft({ resumeAutosave: options.close === false });
    localBackup.clear();
  } catch (err) {
    discarding.value = false;
    throw err;
  }
  if (options.close === false) {
    discarding.value = false;
    return;
  }
  if (props.embedded) emit('close');
  else if (isPopout()) await closePopout();
  else await router.push({ name: 'inbox' });
}

function currentDraftState(): ComposerDraftState {
  return {
    accountId: accountId.value,
    fromAddress: fromAddress.value,
    fromName: activeAccount.value?.displayName ?? null,
    threadId: contextThreadId.value,
    replyTo: replyTo.value,
    sendOptions: {
      reply_to_message_id: replyToMessageId.value,
      follow_up_at: followUpAt.value,
      smime_sign_with_cert_id: smimeSignCertId.value,
      smime_encrypt_to_recipients: smimeEncrypt.value,
      pgp_sign_with_key_id: pgpSignKeyId.value,
      pgp_encrypt_to_recipients: pgpEncrypt.value,
      priority: priority.value,
    },
    to: to.value,
    cc: cc.value,
    bcc: bcc.value,
    subject: subject.value,
    body: body.value,
    bodyHtml: plainTextCompose.value ? null : composerInlineAttachments.serializeInlineImages(bodyHtml.value ?? composerHtmlFromText(body.value)),
    plainText: plainTextCompose.value,
    scheduledFor: scheduledFor.value,
    signatureId: selectedSignatureId.value,
    signatureVariant: signatureVariant.value,
    signatureOverrideText: signatureOverrideText.value,
    signatureOverrideHtml: signatureOverrideHtml.value,
    attachments: attachments.value,
  };
}

function onTemplateApply(rendered: {
  subject?: string | null;
  body_text: string;
  body_html?: string | null;
  cc?: string[];
  bcc?: string[];
  attachments_json?: string | null;
}) {
  if (rendered.subject) subject.value = rendered.subject;
  body.value = rendered.body_text;
  bodyHtml.value = rendered.body_html ?? bodyHtml.value;
  cc.value = mergeRecipients(cc.value, rendered.cc ?? []);
  bcc.value = mergeRecipients(bcc.value, rendered.bcc ?? []);
  if ((rendered.cc?.length ?? 0) > 0 || (rendered.bcc?.length ?? 0) > 0) {
    showCcBcc.value = true;
  }
  if (rendered.attachments_json) {
    try {
      const parsed = JSON.parse(rendered.attachments_json) as BackendOutgoingAttachment[];
      attachments.value = [...attachments.value, ...parsed];
    } catch {
      // Ignore malformed legacy snippet attachment payloads.
    }
  }
  showTemplatePicker.value = false;
}

function onSendLaterChosen(value: string | null) {
  scheduledFor.value = value;
  showSendLater.value = false;
}

/** Marks the ••• button when anything in the panel deviates from defaults. */
const advancedActive = computed(
  () =>
    Boolean(followUpAt.value)
    || priority.value !== 'normal'
    || plainTextCompose.value !== settings.composePlainText
    || cryptoMethod.value !== 'none',
);

const priorityOptions = computed(() => [
  { value: 'high', label: t('compose.priority.high') },
  { value: 'normal', label: t('compose.priority.normal') },
  { value: 'low', label: t('compose.priority.low') },
]);

defineExpose({
  persistDraft,
  discardDraft,
  hasMeaningfulContent,
  shouldConfirmClose,
  closeWithoutSaving,
  currentDraftId: () => draftId.value,
  canSend,
  sending,
});
</script>

<template>
  <section :class="['compose', { 'compose--embedded': props.embedded }]">
    <!-- The header carries the title alone. Template and Save live in the
         footer beside Send, Discard in the options panel. -->
    <header v-if="!props.embedded" class="compose-header">
      <h2>{{ defaultComposerTitle() }}</h2>
    </header>

    <div v-if="popoutUndo || popoutNotice" class="compose-after-send">
      <AppBanner
        v-if="popoutUndo"
        tone="info"
        :title="$t('compose.afterSend.sendingIn', { seconds: undoSecondsLeft }, undoSecondsLeft)"
        :detail="$t('compose.afterSend.undoHint')"
      >
        <template #actions>
          <AppButton size="sm" variant="primary" :disabled="undoing" @click="undoFromPopout">{{ $t('app.mail.undo') }}</AppButton>
          <AppButton size="sm" variant="ghost" :disabled="undoing" @click="closePopout">{{ $t('common.close') }}</AppButton>
        </template>
      </AppBanner>
      <AppBanner v-else tone="warning" :title="popoutNotice ?? ''">
        <template #actions>
          <AppButton size="sm" variant="soft" @click="closePopout">{{ $t('common.close') }}</AppButton>
        </template>
      </AppBanner>
    </div>

    <template v-else>
    <div v-if="saveError || failedSetupParts.length > 0 || backupOffer" class="compose-notices">
      <AppBanner
        v-if="saveError"
        tone="danger"
        :title="saveError.kind === 'conflict' ? $t('compose.unsaved.conflictTitle') : $t('compose.unsaved.title')"
        :detail="saveError.message"
      >
        <template #actions>
          <template v-if="saveError.kind === 'conflict'">
            <AppButton size="sm" variant="soft" :disabled="retryingSave" @click="saveConflictAsNewDraft">{{ $t('compose.unsaved.saveAsNew') }}</AppButton>
            <AppButton size="sm" variant="ghost" :disabled="retryingSave" @click="leaveComposer">{{ $t('compose.unsaved.discardMine') }}</AppButton>
          </template>
          <AppButton v-else size="sm" variant="soft" :disabled="retryingSave" @click="retrySave">{{ $t('common.retry') }}</AppButton>
        </template>
      </AppBanner>
      <AppBanner
        v-if="failedSetupParts.length > 0"
        tone="warning"
        :title="$t('compose.setup.failed')"
        :detail="setupErrorDetail"
      >
        <template #actions>
          <AppButton size="sm" variant="soft" :disabled="retryingSetup" @click="retrySetup">{{ $t('common.retry') }}</AppButton>
        </template>
      </AppBanner>
      <AppBanner
        v-if="backupOffer"
        tone="info"
        :title="backupOfferTitle"
        :detail="$t('compose.backup.detail')"
      >
        <template #actions>
          <AppButton size="sm" variant="primary" @click="restoreBackup">{{ $t('compose.backup.restore') }}</AppButton>
          <AppButton size="sm" variant="ghost" @click="dismissBackup">{{ $t('common.discard') }}</AppButton>
        </template>
      </AppBanner>
    </div>

    <div class="row sender-row">
      <label class="label" :for="senderFieldId">{{ $t('common.from') }}</label>
      <div class="sender-field">
        <select
          class="sender-select"
          :id="senderFieldId"
          :disabled="sending"
          :title="$t('compose.sender')"
          :aria-label="$t('compose.sender')"
          :value="currentIdentityKey"
          @change="onIdentitySelected"
        >
          <template v-if="identityChoices.length > 1">
            <optgroup
              v-for="group in identityChoices"
              :key="group.account.id"
              :label="group.account.displayName"
            >
              <option v-for="option in group.options" :key="option.key" :value="option.key">{{ option.label }}</option>
            </optgroup>
          </template>
          <template v-else>
            <option
              v-for="option in identityChoices[0]?.options ?? []"
              :key="option.key"
              :value="option.key"
            >{{ option.label }}</option>
          </template>
        </select>
        <AppIcon class="sender-chevron" size="sm"><path d="m4 6 4 4 4-4" /></AppIcon>
      </div>
    </div>

    <div class="row recipient-row">
      <label class="label">{{ $t('common.to') }}</label>
      <RecipientChips ref="toFieldRef" v-model="to" :placeholder="$t('compose.recipients.toPlaceholder')" class="field" @submit="send" />
      <AppButton size="sm" variant="ghost" class="cc-link"
        :aria-expanded="showCcBcc"
        :title="showCcBcc ? $t('compose.recipients.collapseCcBcc') : $t('compose.recipients.showCcBcc')"
        @click="showCcBcc = !showCcBcc"
      >
        {{ $t('compose.recipients.ccBcc') }}
        <AppIcon :class="{ 'cc-link-icon--open': showCcBcc }"><path d="m4 6 4 4 4-4" /></AppIcon>
      </AppButton>
    </div>

    <template v-if="showCcBcc">
      <div class="row">
        <label class="label">{{ $t('common.cc') }}</label>
        <RecipientChips v-model="cc" :placeholder="$t('compose.recipients.ccPlaceholder')" class="field" @submit="send" />
      </div>
      <div class="row">
        <label class="label">{{ $t('common.bcc') }}</label>
        <RecipientChips v-model="bcc" :placeholder="$t('compose.recipients.bccPlaceholder')" class="field" @submit="send" />
      </div>
    </template>

    <div class="row subject-row">
      <label class="label" :for="subjectFieldId">{{ $t('common.subject') }}</label>
      <input
        :id="subjectFieldId"
        v-model="subject"
        class="compose-subject"
        :aria-label="$t('common.subject')"
        :placeholder="$t('compose.subject.placeholder')"
        @keydown.meta.enter.prevent="send"
        @keydown.ctrl.enter.prevent="send"
      />
    </div>

    <div class="body" :class="{ 'body--with-history': plainTextCompose ? historyText : historyHtml }" @click="focusComposeBody">
      <textarea
        v-if="plainTextCompose"
        ref="writingTextarea"
        v-model="writingText"
        class="plain-editor"
        :disabled="sending"
        :placeholder="$t('compose.editor.placeholder')"
        :aria-label="$t('compose.body.plainAriaLabel')"
        spellcheck="true"
        @input="bodyHtml = null"
        @keydown.meta.enter.prevent="send"
        @keydown.ctrl.enter.prevent="send"
      />
      <RichTextEditor
        v-else
        ref="richEditorRef"
        v-model="writingText"
        :html="writingHtml"
        :disabled="sending"
        :autofocus="props.embedded && !startsInRecipients"
        :focus-position="!props.initialDraftId && (props.mode === 'reply' || props.mode === 'reply-all' || props.mode === 'forward') ? 'start' : 'end'"
        :min-height="historyHtml ? '1.68em' : '5.04em'"
        :placeholder="$t('compose.editor.placeholder')"
        @update:html="(v) => (writingHtml = v)"
        @inline-image-added="composerInlineAttachments.onInlineImageAdded"
        @image-to-attachment="composerInlineAttachments.onImageToAttachment"
        @paste-files="composerInlineAttachments.addPastedFiles"
        @submit="send"
      />
      <ComposerSignature
        v-if="supportsBackend && availableSignatures.length > 0"
        :signatures="availableSignatures"
        :selected-id="selectedSignatureId"
        :variant="signatureVariant"
        :loading="signaturePreviewLoading"
        :display-html="signatureDisplayHtml"
        :display-text="signatureDisplayText"
        :override-active="signatureOverrideActive"
        :plain-text="plainTextCompose"
        :disabled="sending"
        @select="onSignatureSelect"
        @variant-change="onSignatureVariantChange"
        @edit="onSignatureEdit"
        @reset="clearSignatureOverride"
      />
      <div v-if="plainTextCompose ? historyText : historyHtml" class="quoted-history" data-composer-history>
        <!-- The note heads the quote it is about, under the signature, instead
             of sitting above the subject line, far from what it removes. -->
        <div v-if="hasQuotedHistory" class="history-control" @click.stop>
          <span>{{ $t('compose.context.included') }}</span>
          <IconButton :title="$t('compose.context.remove')" :disabled="sending" size="sm" @click="removeQuotedHistory">
            <AppIcon size="sm"><path d="m4 4 8 8M12 4l-8 8" /></AppIcon>
          </IconButton>
        </div>
        <textarea
          v-if="plainTextCompose"
          ref="historyTextarea"
          v-model="historyText"
          class="plain-editor"
          :disabled="sending"
          :aria-label="$t('compose.context.included')"
          spellcheck="true"
          @input="bodyHtml = null"
          @keydown.meta.enter.prevent="send"
          @keydown.ctrl.enter.prevent="send"
        />
        <RichTextEditor
          v-else
          ref="historyEditorRef"
          v-model="historyText"
          :html="historyHtml"
          :disabled="sending"
          :aria-label="$t('compose.context.included')"
          min-height="0"
          @update:html="(v) => (historyHtml = v)"
          @inline-image-added="composerInlineAttachments.onInlineImageAdded"
          @image-to-attachment="composerInlineAttachments.onImageToAttachment"
          @paste-files="composerInlineAttachments.addPastedFiles"
          @submit="send"
        />
      </div>
      <ComposerThreadContext v-if="contextThreadId && !quoteHoldsWholeThread" :thread-id="contextThreadId" />
    </div>

    <AttachmentDropZone
      ref="attachmentZoneRef"
      v-model="attachments"
      inline-enabled
      :show-empty-state="false"
      class="compose-attachments"
      @insert-inline="composerInlineAttachments.onAttachmentInsertedInline"
      @remove-inline="composerInlineAttachments.onInlineAttachmentRemoved"
      @move-to-attachment="composerInlineAttachments.onInlineAttachmentRemoved"
    />

    <footer ref="footerRef" class="footer">
      <div v-if="status" class="status-block">
        <p
          class="status"
          :class="`status--${status.tone}`"
          :role="status.tone === 'error' ? 'alert' : 'status'"
        >{{ status.text }}</p>
        <details v-if="status.raw" class="status-details">
          <summary>{{ $t('compose.status.technical') }}</summary>
          <pre>{{ status.raw }}</pre>
        </details>
      </div>
      <p v-if="scheduledFor" class="schedule-info">
        {{ $t('compose.schedule.scheduled', { time: formatScheduled(scheduledFor) }) }}
        <AppButton size="sm" variant="ghost" @click="scheduledFor = null"><template #icon><AppIcon><path d="m4 4 8 8M12 4l-8 8"/></AppIcon></template>{{ $t('compose.schedule.remove') }}</AppButton>
      </p>
      <p v-if="followUpAt" class="schedule-info">
        {{ $t('compose.schedule.followUp', { time: formatScheduled(followUpAt) }) }}
        <AppButton size="sm" variant="ghost" @click="followUpAt = null"><template #icon><AppIcon><path d="m4 4 8 8M12 4l-8 8"/></AppIcon></template>{{ $t('compose.schedule.remove') }}</AppButton>
      </p>
      <div class="footer-actions">
        <!-- The attachment label folds away when the composer is narrow. -->
        <AppButton variant="ghost" class="tool-button" :title="$t('compose.toolbar.attach')" :aria-label="$t('compose.toolbar.attach')" @click="attachmentZoneRef?.openPicker()">
          <template #icon><AppIcon size="lg"><path d="M13.5 7.5l-6.2 6.2a3.5 3.5 0 01-5-5l6.5-6.5a2.5 2.5 0 013.5 3.5L5.8 12.2a1.5 1.5 0 01-2.1-2.1l6.1-6.1"/></AppIcon></template>
          <span class="tool-label">{{ $t('compose.toolbar.attachShort') }}</span>
        </AppButton>
        <IconButton size="lg" v-if="supportsBackend" :title="$t('compose.toolbar.template')" :aria-label="$t('compose.toolbar.template')" @click="showTemplatePicker = true">
          <AppIcon size="lg"><path d="M3 1.5h7l3 3v10H3z"/><path d="M10 1.5v3h3M5.5 8h5M5.5 11h4"/></AppIcon>
        </IconButton>
        <details class="more-settings">
          <summary class="more-summary" :title="$t('compose.toolbar.more')" :aria-label="$t('compose.toolbar.more')">
            <AppIcon size="lg"><circle cx="3" cy="8" r=".75" fill="currentColor" /><circle cx="8" cy="8" r=".75" fill="currentColor" /><circle cx="13" cy="8" r=".75" fill="currentColor" /></AppIcon><span v-if="advancedActive" class="tool-dot" aria-hidden="true" />
          </summary>
          <div class="advanced-panel">
            <div class="panel-row">
              <span class="panel-label">{{ $t('compose.options.priority') }}</span>
              <SegmentedControl
                :options="priorityOptions"
                :model-value="priority"
                :aria-label="$t('compose.options.priorityAriaLabel')"
                @update:model-value="priority = $event as 'high' | 'normal' | 'low'"
              />
            </div>
            <div class="panel-row">
              <span class="panel-label">{{ $t('compose.options.format') }}</span>
              <label class="panel-check">
                <input v-model="plainTextCompose" type="checkbox" />
                {{ $t('compose.options.plainText') }}
              </label>
            </div>
            <div class="panel-row">
              <span class="panel-label">{{ $t('compose.options.followUp') }}</span>
              <div class="panel-field">
                <input
                  class="follow-input"
                  type="datetime-local" step="60"
                  :aria-label="$t('compose.options.followUpAriaLabel')"
                  @change="followUpAt = ($event.target as HTMLInputElement).value ? new Date(($event.target as HTMLInputElement).value).toISOString() : null"
                />
                <small class="panel-note">{{ $t('compose.options.followUpNote') }}</small>
              </div>
            </div>
            <div class="panel-sep" role="separator" />
            <div class="panel-row">
              <span class="panel-label">{{ $t('compose.options.security') }}</span>
              <SegmentedControl
                :options="cryptoOptions"
                :model-value="cryptoMethod"
                :aria-label="$t('compose.options.securityAriaLabel')"
                @update:model-value="cryptoMethod = $event as ComposerCryptoMethod"
              />
            </div>
            <div v-if="cryptoMethod !== 'none'" class="panel-row">
              <span class="panel-label" aria-hidden="true" />
              <div class="panel-field">
                <label class="panel-check">
                  <input v-model="cryptoSign" type="checkbox" :disabled="!canSignSelected" />
                  {{ $t('compose.crypto.sign') }}
                </label>
                <label class="panel-check">
                  <input v-model="cryptoEncrypt" type="checkbox" :disabled="!canEncryptSelected" />
                  {{ $t('compose.crypto.encrypt') }}
                </label>
                <small v-if="cryptoHint" class="panel-note">{{ cryptoHint }}</small>
              </div>
            </div>
            <template v-if="draftId">
              <div class="panel-sep" role="separator" />
              <AppButton size="sm" variant="danger"
                :disabled="sending || discarding"
                @click="discardDraft()"
              >
                <template #icon><AppIcon><path d="M2.5 4.5h11M6 2h4l.5 2.5h-5L6 2ZM4 4.5l.5 9h7l.5-9M6.5 7v4M9.5 7v4"/></AppIcon></template>
                {{ $t('compose.discardDraft') }}
              </AppButton>
            </template>
          </div>
        </details>
        <span class="footer-spacer" />
        <AppButton
          v-if="!props.embedded && supportsBackend"
          variant="ghost"
          :disabled="sending || discarding"
          @click="persistDraft({})"
        >
          <template #icon><AppIcon><path d="M2.5 2h9l2 2v10h-11z"/><path d="M5 2v4h6V2M5 14V9h6v5"/></AppIcon></template>
          {{ $t('common.save') }}
        </AppButton>
        <!-- A disabled button gets no hover of its own; the wrapper carries the tooltip. -->
        <span class="send-wrap" :title="sendBlockedHint ?? undefined" data-testid="compose-send-wrap">
          <SplitButton
            :secondary-label="$t('compose.sendLater.title')"
            :expanded="showSendLater"
            popup="menu"
            :title="sendBlockedHint ?? (sending ? $t('compose.sending') : scheduledFor ? $t('compose.schedule.submit') : $t('compose.send'))"
            :shortcut="canSend ? sendShortcut : null"
            :disabled="!canSend"
            :description-id="sendBlockedHint ? sendHintId : undefined"
            @click="send"
            @open="openSendLater"
          >
            <template #icon>
              <AppIcon v-if="scheduledFor"><circle cx="8" cy="8" r="6"/><path d="M8 4.5V8l2.5 1.5"/></AppIcon>
              <AppIcon v-else><path d="m14 2-4.5 12L7 9 2 6.5 14 2ZM7 9l4-4"/></AppIcon>
            </template>
            {{
              sending
                ? $t('compose.sending')
                : scheduledFor
                ? $t('compose.schedule.submit')
                : $t('compose.send')
            }}
          </SplitButton>
          <span v-if="sendBlockedHint" :id="sendHintId" class="sr-only">{{ sendBlockedHint }}</span>
        </span>
      </div>
    </footer>

    </template>

    <TemplatePicker
      v-if="showTemplatePicker"
      @apply="onTemplateApply"
      @close="showTemplatePicker = false"
    />
    <SendLaterMenu
      v-if="sendLaterAnchor"
      :open="showSendLater"
      :anchor="sendLaterAnchor"
      :value="scheduledFor"
      @select="onSendLaterChosen"
      @close="showSendLater = false"
    />
  </section>
</template>

<style scoped>
.history-control {
  display: flex;
  flex: none;
  align-items: center;
  gap: var(--space-xs);
  margin-bottom: var(--space-xs);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.plain-editor {
  width: 100%;
  flex: 1 1 auto;
  min-height: 120px;
  padding: 2px 0;
  resize: none;
  border: 0;
  outline: none;
  background: transparent;
  color: var(--text-primary);
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: var(--font-size-md);
  line-height: 1.6;
}

.plain-editor::placeholder {
  opacity: 1;
  color: var(--text-secondary);
}

.compose {
  position: relative;
  display: grid;
  gap: 12px;
  padding: 24px;
  height: 100%;
  overflow: auto;
  align-content: start;
  background: var(--bg-base);
}

.compose-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-lg);
  /* With the page's 16px padding the title's line box starts 24px down. The
     header exists only on the page, so the embedded composer keeps its edge. */
  padding-block: var(--space-sm) var(--space-lg);
  border-bottom: 1px solid var(--border-color-muted);
}

h2 {
  margin: 0;
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.row {
  display: grid;
  grid-template-columns: 80px 1fr;
  gap: 12px;
  align-items: center;
}

.label {
  font-size: var(--font-size-xs);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-secondary);
}

.field {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  flex: 1;
}

input,
select,
textarea {
  width: 100%;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  padding: 9px 12px;
  font-size: var(--font-size-md);
  color: var(--text-primary);
  font-family: inherit;
}

input:focus,
select:focus,
textarea:focus {
  border-color: var(--accent-primary);
}

.body textarea {
  min-height: 320px;
  resize: vertical;
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, monospace);
  line-height: 1.55;
}

.footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--border-color-muted);
}

.status-block {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: var(--space-2xs);
}

.status {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

/* Info and success keep the neutral line — a closing window is the success
   signal. Only the two outcomes the user has to act on carry a colour, and
   they use the soft accents: saturated body text vibrates on a grey pane. */
.status--warning {
  color: var(--accent-amber-soft);
}

.status--error {
  color: var(--accent-red-soft);
  font-weight: var(--weight-semibold);
}

.status-details {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.status-details summary {
  cursor: pointer;
}

.status-details pre {
  margin: var(--space-2xs) 0 0;
  padding: var(--space-sm);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  font-family: var(--font-mono);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.schedule-info {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--accent-primary);
}

.footer-actions {
  display: flex;
  gap: 8px;
  margin-left: auto;
  flex-wrap: wrap;
  justify-content: flex-end;
}


/* Banners sit in the composer's flow, inset like the fields below them. */
.compose-notices,
.compose-after-send {
  flex: none;
  display: grid;
  gap: var(--space-xs);
  margin-bottom: var(--space-sm);
}

.compose-notices :deep(.banner),
.compose-after-send :deep(.banner) {
  border: var(--border-muted);
  border-radius: var(--radius-sm);
}

/* Composer window layout */
.compose {
  /* Labels sit in a column of their own; every value — field, body text,
     attachments — starts at the same left edge after it. */
  --compose-label-w: 64px;
  --compose-value-x: calc(var(--compose-label-w) + var(--space-sm));
  display: flex;
  flex-direction: column;
  gap: 0;
  min-height: 0;
  width: 100%;
  max-width: calc(48rem + var(--space-xl) * 2);
  margin-inline: auto;
  padding: var(--space-lg) var(--space-xl) var(--space-3-5);
  overflow: hidden;
  background: transparent;
}

.compose--embedded {
  height: 100%;
}

.compose-subject {
  width: 100%;
  min-width: 0;
  min-height: 36px;
  padding: var(--space-sm) 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  color: var(--text-primary);
  font-size: var(--font-size-lg);
  font-weight: var(--weight-semibold);
}

.compose-subject::placeholder { color: var(--text-tertiary); }
.compose-subject:focus { border-color: var(--border-color-muted); box-shadow: none; }

.compose .row {
  flex: none;
  grid-template-columns: var(--compose-label-w) minmax(0, 1fr);
  min-height: 44px;
  gap: var(--space-sm);
  padding: var(--space-xs) 0;
  border-bottom: 1px solid var(--border-color);
}

.compose .recipient-row {
  grid-template-columns: var(--compose-label-w) minmax(0, 1fr) auto;
}

.compose .label {
  align-self: center;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
  letter-spacing: 0;
  text-transform: none;
}

.compose .cc-link {
  align-self: center;
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 5px 3px;
  color: var(--text-secondary);
}

.compose .cc-link:hover { color: var(--text-primary); }

.compose .cc-link svg { transition: transform var(--duration-normal) var(--ease-out); }
.compose .cc-link .cc-link-icon--open { transform: rotate(180deg); }

.compose :deep(.chips) {
  min-height: 31px;
  padding: 2px 0;
  border: 0;
  background: transparent;
}

.compose :deep(.chips:focus-within) { border-color: transparent; box-shadow: none; }
.compose :deep(.chip) { padding-block: 3px; }

.compose .body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
  padding: var(--space-md) 0 var(--space-xs) var(--compose-value-x);
  cursor: text;
}

/* On the page, the writing area grows with the text and the action bar
   follows right under it, instead of waiting at the bottom of the window.
   The floating composer keeps filling its frame. */
.compose:not(.compose--embedded) .body {
  flex: 0 1 auto;
  min-height: min(18rem, 40vh);
}

/* The editor grows to fill leftover space but must not claim the full
   container height — signature block and thread history render below it
   and have to stay visible without scrolling. */
.compose .body :deep(.rt-editor) {
  flex: 1 0 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 0;
  padding: 0;
  border: 0;
  background: transparent;
}

/* Reserve three writing lines so the signature also fits at minimum height. */
.compose .body > .plain-editor {
  min-height: 5.04em;
}

/* The signature follows the answer's last line, including in plain-text mode. */
.compose .body--with-history > :deep(.rt-editor),
.compose .body--with-history > .plain-editor {
  flex: none;
}

.compose .body--with-history .plain-editor {
  min-height: 0;
  resize: none;
}

.quoted-history {
  flex: none;
  margin-top: var(--space-lg);
}

.compose .body :deep(.content) {
  flex: 1;
  padding: 2px 0;
  cursor: text;
  font-size: var(--font-size-lg);
  line-height: 1.68;
}

.compose .compose-attachments {
  flex: none;
  max-height: 132px;
  overflow: auto;
  margin: 3px 0 8px var(--compose-value-x);
  padding: 8px 10px;
  border-color: var(--border-color-muted);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-base) 28%, transparent);
}

.compose .compose-attachments :deep(.empty) {
  justify-content: center;
  min-height: 20px;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.compose .footer {
  flex: none;
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 5px;
  padding-top: var(--space-sm);
  border-top-color: var(--border-color);
}

.compose .footer-actions {
  container-type: inline-size;
  width: 100%;
  align-items: center;
  flex-wrap: nowrap;
  gap: var(--space-2xs);
  margin: 0;
}

/* Labelled tools sit flush with the icon-only ones beside them. */
.tool-button {
  padding-inline: var(--space-2-5) var(--space-3-5);
}

@container (max-width: 34rem) {
  .tool-label { display: none; }
  .tool-button { width: 38px; padding: 0; }
  .tool-button :deep(.app-button__icon) { margin: 0; }
}

.more-summary {
  width: 38px;
  height: 38px;
  display: grid;
  flex: none;
  place-items: center;
  padding: 0;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.more-summary:hover { background: var(--bg-hover); color: var(--text-primary); }

.more-settings {
  position: relative;
  flex: none;
}

.more-settings > summary { list-style: none; letter-spacing: 1px; }
.more-settings > summary::-webkit-details-marker { display: none; }

.advanced-panel {
  position: absolute;
  left: 0;
  bottom: calc(100% + 10px);
  z-index: 40;
  width: min(380px, calc(100vw - 60px));
  display: grid;
  gap: 10px;
  padding: 14px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--bg-overlay);
  box-shadow: var(--shadow-lg);
}

.panel-row {
  display: grid;
  grid-template-columns: 84px minmax(0, 1fr);
  gap: 12px;
  align-items: center;
  min-height: 30px;
}

.panel-label {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
}

.panel-field {
  display: grid;
  gap: 7px;
  justify-items: start;
}

.panel-check {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: var(--text-primary);
  cursor: pointer;
  font-size: var(--font-size-sm);
}

.panel-check:has(input:disabled) {
  color: var(--text-tertiary);
  cursor: not-allowed;
}

.panel-check input {
  width: 14px;
  height: 14px;
  margin: 0;
  padding: 0;
  accent-color: var(--accent-primary);
}

.panel-note {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  line-height: 1.45;
}

.panel-sep {
  height: 1px;
  margin: 2px 0;
  background: var(--border-color-muted);
}

.follow-input {
  width: 200px;
  padding: 7px 9px;
  font-size: var(--font-size-sm);
}

.more-summary {
  position: relative;
}

.tool-dot {
  position: absolute;
  top: 5px;
  right: 5px;
  width: 6px;
  height: 6px;
  border-radius: var(--radius-full, 999px);
  background: var(--accent-primary);
}


.footer-spacer { flex: 1; }

.send-wrap { display: inline-flex; }

.sender-field {
  position: relative;
  min-width: 0;
  justify-self: start;
  max-width: 100%;
}

.sender-select {
  appearance: none;
  width: 100%;
  min-width: 0;
  min-height: 36px;
  padding: var(--space-sm) var(--space-2xl) var(--space-sm) 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  text-overflow: ellipsis;
  cursor: pointer;
}

.sender-select option,
.sender-select optgroup {
  background: var(--bg-overlay);
  color: var(--text-primary);
}

.sender-chevron {
  position: absolute;
  right: var(--space-sm);
  top: 50%;
  transform: translateY(-50%);
  color: var(--text-secondary);
  pointer-events: none;
}

.send-wrap :deep(.split-button button:disabled) {
  opacity: 1;
  background: color-mix(in srgb, var(--accent-primary) 75%, var(--bg-raised));
  color: var(--text-on-accent);
}


.compose .status,
.compose .schedule-info { font-size: var(--font-size-xs); }

@media (max-width: 620px) {
  .compose { padding: var(--space-3-5) var(--space-3-5) var(--space-2-5); }
  /* A phone has no width to spare for an empty label column. */
  .compose .body { padding-left: 0; }
  .compose .compose-attachments { margin-left: 0; }
  .compose .compose-attachments :deep(.empty) > span { display: none; }
  .composer-header { display: none; }
}
</style>
