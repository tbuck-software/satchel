<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import type {
  Message,
  PgpIndicatorState,
  PgpMessageState,
  SmimeIndicatorState,
  SmimeMessageState,
} from '@/types/mail';
import { useMessagesStore } from '@/stores/messages';
import { useThreadsStore } from '@/stores/threads';
import { useTopicsStore } from '@/stores/topics';
import { useUiStore } from '@/stores/ui';
import { useSettingsStore } from '@/stores/settings';
import Avatar from '@/components/common/Avatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppBanner from '@/components/ui/AppBanner.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import AttachmentCard from './AttachmentCard.vue';
import EmailRenderer from './EmailRenderer.vue';
import ItemRenderer from './ItemRenderer.vue';
import InvitationCard from './InvitationCard.vue';
import PgpStatusCard from './PgpStatusCard.vue';
import SmimeStatusCard from './SmimeStatusCard.vue';
import MessageSecurityBadge from './MessageSecurityBadge.vue';
import UnsubscribeBanner from './UnsubscribeBanner.vue';
import MessageSourceDialog from './MessageSourceDialog.vue';
import MessageRecipientDetails from './MessageRecipientDetails.vue';
import PassphraseDialog from './PassphraseDialog.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import SenderFilingMenu from './SenderFilingMenu.vue';
import { useInvitesStore } from '@/stores/invites';
import { useAccountsStore } from '@/stores/accounts';
import { isOwnEmail, participantDisplayName } from '@/utils/mailIdentity';
import { isKnownInviteBoilerplate } from '@/utils/inviteText';
import { detectSenderSpoofing } from '@/utils/senderSpoofing';
import { formatMessageTimestamp } from '@/utils/listTime';
import { isSmimeSignatureAttachment, isSystemAttachment } from '@/utils/messageAttachments';
import { summarizeAuthentication } from '@/utils/messageAuthentication';
import type { AnchorBox } from '@/utils/popoverPlacement';
import { useMessageSecurityActions } from '@/composables/useMessageSecurityActions';

const props = withDefaults(defineProps<{
  message: Message;
  canReply?: boolean;
  collapsed?: boolean;
  keyboardFocused?: boolean;
  /** False while the message is far from the viewport: no renderer, no frame. */
  bodyMounted?: boolean;
}>(), {
  canReply: true,
  collapsed: false,
  keyboardFocused: false,
  bodyMounted: true,
});
const emit = defineEmits<{
  reply: [];
  forward: [];
  focusMessage: [];
  toggleCollapsed: [];
}>();
const ui = useUiStore();
const settings = useSettingsStore();
const topicsStore = useTopicsStore();
const messagesStore = useMessagesStore();
const invitesStore = useInvitesStore();
const accountsStore = useAccountsStore();
const bodyCacheKey = computed(() =>
  `${props.message.id}:uidvalidity:${props.message.remoteUidValidity ?? 'local'}`);
const { t } = useI18n();
const sourceDialogOpen = ref(false);
const messageActionsOpen = ref(false);
const pgpDismissed = ref(false);
const smimeDismissed = ref(false);
const {
  pgpBusy,
  pgpStatus,
  smimeStatus,
  passphraseRequest,
  submitPassphrase,
  cancelPassphrase,
  unlockPgp,
  unlockSmime,
  reset: resetSecurityActions,
} = useMessageSecurityActions();

const matchingInvite = computed(() => invitesStore.findByMessageId(props.message.id));

// Invite mails from known generators (Exchange field sheets, Google
// Calendar, group-calendar notices) only duplicate what the invitation card
// shows. Those disappear; anything unrecognized is treated as a real
// hand-written message and stays fully visible.
const inviteBodySuppressed = computed(
  () =>
    matchingInvite.value !== null && isKnownInviteBoilerplate(props.message.bodyText ?? ''),
);

const inlineImagesBusy = ref(false);
/**
 * Inline images that stayed unresolved. Almost always transient — a provider
 * rate limit or a lost connection — so the mail says so and offers a retry
 * instead of silently rendering an empty box.
 */
const inlineImagesFailed = computed(
  () => (props.message.inlineImagesPending ?? 0) > 0 && Boolean(props.message.inlineImagesError),
);

/**
 * Provider errors arrive in English and unabbreviated. Only the rate limit is
 * worth translating: it is the frequent one, and it is the one where the wait
 * decides whether pressing the button again makes any sense.
 */
const inlineImagesDetail = computed(() => {
  const raw = props.message.inlineImagesError ?? '';
  if (!/rate limit/i.test(raw)) return raw;
  const seconds = Number(/(\d+)\s*s\b/.exec(raw)?.[1] ?? 0);
  if (!seconds) return t('mail.message.rateLimited');
  return t('mail.message.rateLimitedRetry', { count: Math.ceil(seconds / 60) });
});

async function retryInlineImages() {
  inlineImagesBusy.value = true;
  try {
    await messagesStore.retryInlineImages(props.message.threadId, props.message.id);
  } finally {
    inlineImagesBusy.value = false;
  }
}

const fullTimestamp = computed(() => formatMessageTimestamp(props.message.timestamp));

/* Folded away, the avatar is a marker beside one line, not the head of a
   block of text. The indent under it stays put — see --indented below. */
const avatarSize = computed(() => (props.collapsed ? 24 : 40));

/* A glance at the folded message. Empty until the body has arrived, and the
   line simply does without it then rather than reserving space for nothing. */
const collapsedSnippet = computed(
  () => props.message.bodyText.replace(/\s+/g, ' ').trim().slice(0, 160),
);

const senderName = computed(() => participantDisplayName(accountsStore.accounts, props.message.from));
const isOwn = computed(() => isOwnEmail(accountsStore.accounts, props.message.from.email));

const spoofingWarning = computed(() =>
  isOwn.value
    ? null
    : detectSenderSpoofing(senderName.value, props.message.from.email, {
        spf: props.message.authSpf,
        dkim: props.message.authDkim,
        dmarc: props.message.authDmarc,
      }),
);
const bodyLoaded = computed(() => props.message.bodyLoaded !== false);
const showBodySkeleton = ref(false);
let bodySkeletonTimer: ReturnType<typeof setTimeout> | undefined;

watch(
  bodyLoaded,
  (loaded) => {
    if (bodySkeletonTimer) {
      clearTimeout(bodySkeletonTimer);
      bodySkeletonTimer = undefined;
    }
    showBodySkeleton.value = false;
    if (!loaded) {
      bodySkeletonTimer = setTimeout(() => {
        bodySkeletonTimer = undefined;
        showBodySkeleton.value = !bodyLoaded.value;
      }, 160);
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  if (bodySkeletonTimer) clearTimeout(bodySkeletonTimer);
});

const renderBody = computed(() => props.bodyMounted && !props.collapsed);
const showBody = computed(() =>
  bodyLoaded.value
  && !props.message.bodyLoadError
  && (Boolean(props.message.sourcePayload) || !inviteBodySuppressed.value),
);
/* The unmounted body keeps the height it last had, so the scroll position holds. */
const bodyHost = ref<HTMLElement | null>(null);
const placeholderHeight = ref(240);
watch(renderBody, (mounted, wasMounted) => {
  if (!mounted && wasMounted && bodyHost.value && bodyHost.value.offsetHeight > 0) {
    placeholderHeight.value = bodyHost.value.offsetHeight;
  }
}, { flush: 'pre' });

const visibleBodyText = computed(() => {
  if (!bodyLoaded.value || props.message.bodyLoadError) return '';
  return props.message.bodyText;
});

const visibleAttachments = computed(() => {
  const seen = new Set<string>();
  return props.message.attachments.filter((attachment) => {
    if (isSystemAttachment(attachment)) {
      return false;
    }
    const key = [
      attachment.filename.trim().toLowerCase(),
      attachment.contentType.trim().toLowerCase(),
      attachment.size ?? 0,
      attachment.contentId ?? '',
    ].join('\u0000');
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
});

/*
 * One rhythm on the desktop: the same step above and below a message, so the
 * rule between two of them sits halfway. Drafts are not messages here; they
 * end the thread as ThreadDraftCard.
 */
const surfaceClasses = computed(() => {
  return ui.isMobile
    ? ['border-b border-border-muted py-4']
    : ['message-surface--indented border-b border-border-muted py-5 last:border-b-0'];
});

const threadsStore = useThreadsStore();

/**
 * Sender filing hangs off the avatar. It used to appear on hover, in a box
 * offset far enough from the avatar that the pointer left the trigger before
 * reaching it — the menu was visible and unusable. Click opens it now, and
 * `ContextMenu` keeps it on screen.
 */
const senderMenuAnchor = ref<AnchorBox | null>(null);
const senderAccountId = computed(
  () => threadsStore.find(props.message.threadId)?.accountId
    ?? threadsStore.activeThread?.accountId
    ?? '',
);

function toggleSenderMenu(event: MouseEvent) {
  if (senderMenuAnchor.value) {
    senderMenuAnchor.value = null;
    return;
  }
  const trigger = (event.currentTarget as HTMLElement).getBoundingClientRect();
  senderMenuAnchor.value = {
    left: trigger.left,
    top: trigger.bottom + 6,
    right: trigger.right,
    bottom: trigger.top,
  };
}

/* A folded message is one line; a click on it (not on its buttons) opens it. */
function openFoldedFromClick(event: MouseEvent) {
  if (!props.collapsed) return;
  if ((event.target as Element | null)?.closest('button, a, [role="button"]')) return;
  emit('toggleCollapsed');
}

const sourceDialogUsed = ref(false);
const messageActionsUsed = ref(false);
const senderMenuUsed = ref(false);
watch(sourceDialogOpen, (open) => { if (open) sourceDialogUsed.value = true; });
watch(messageActionsOpen, (open) => { if (open) messageActionsUsed.value = true; });
watch(senderMenuAnchor, (anchor) => { if (anchor) senderMenuUsed.value = true; });

function runMessageAction(action: () => void) {
  messageActionsOpen.value = false;
  action();
}

const pgpState = computed<PgpMessageState | null>(() => props.message.pgp?.state ?? null);
const smimeState = computed<SmimeMessageState | null>(() => props.message.smime?.state ?? null);
const hasSmimeSignatureAttachment = computed(() =>
  props.message.attachments.some(isSmimeSignatureAttachment),
);
const pgpIndicatorState = computed<PgpIndicatorState | null>(
  () => ((pgpState.value === 'signature_identity_mismatch' || pgpState.value === 'signature_partial') ? pgpState.value : null),
);
const smimeDisplayState = computed<SmimeIndicatorState | null>(() => {
  if (
    smimeState.value === 'signature_valid'
    || smimeState.value === 'signature_partial'
    || smimeState.value === 'signature_unchained'
    || smimeState.value === 'signature_identity_mismatch'
    || smimeState.value === 'signature_invalid'
    || smimeState.value === 'decrypted'
  ) {
    return smimeState.value;
  }
  if (hasSmimeSignatureAttachment.value) return 'signature_unverified';
  return null;
});
const authenticationTone = computed(() => summarizeAuthentication({
  spf: props.message.authSpf,
  dkim: props.message.authDkim,
  dmarc: props.message.authDmarc,
}));
const showPgpCard = computed(() => {
  const state = pgpState.value;
  if (!state || pgpDismissed.value) {
    return false;
  }

  if (state === 'key_available_locked' || state === 'decrypted') {
    return false;
  }

  return true;
});
const showSmimeCard = computed(() =>
  Boolean(smimeState.value && smimeDisplayState.value === null && !smimeDismissed.value),
);

watch(
  () => props.message.id,
  () => {
    pgpDismissed.value = false;
    smimeDismissed.value = false;
    resetSecurityActions();
  },
);

function openKeyImport(section: 'pgp' | 'smime') {
  ui.openSettings(section);
}

function dismissPgpCard() {
  pgpDismissed.value = true;
}

function dismissSmimeCard() {
  smimeDismissed.value = true;
}

async function handleUnlock() {
  await unlockPgp(props.message.threadId, props.message.id);
}

async function handleSmimeUnlock() {
  await unlockSmime(props.message.threadId);
}

const recipientDetailsRef = ref<{ openDetails: () => void } | null>(null);

function openSenderDetails() {
  recipientDetailsRef.value?.openDetails();
}
</script>

<template>
  <!--
    No frame, on either platform. The reading canvas is the sheet the message
    is printed on; a card around it draws a second sheet on the first. What
    the frame was there for — telling two messages apart — a rule and the
    sender header do, and on a phone the frame also cost 48px of a 411px
    width, which is where a fixed-width table stops fitting.

    Desktop indents everything below the header to the sender's name, so a
    thread reads as one column of text with the avatars down its left edge.
  -->
  <article
    class="message-surface group relative overflow-hidden"
    :class="[
      surfaceClasses,
      props.collapsed ? 'message-surface--collapsed' : '',
      props.keyboardFocused ? 'message-surface--keyboard-focused' : '',
    ]"
    :data-thread-message-id="props.message.id"
    :aria-expanded="!props.collapsed"
    tabindex="-1"
    @pointerdown="emit('focusMessage')"
    @click="openFoldedFromClick"
  >
    <!-- Header -->
    <div class="message-header flex items-start justify-between">
      <div class="message-sender-group flex min-w-0 items-center gap-3.5">
        <CardButton
          v-if="props.canReply && !ui.isMobile && senderAccountId"
          class="sender-avatar-trigger"
          :class="{ 'sender-avatar-trigger--open': senderMenuAnchor !== null }"
          :title="$t('mail.message.fileSender', { name: senderName })"
          :aria-label="$t('mail.message.fileSender', { name: senderName })"
          aria-haspopup="menu"
          :aria-expanded="senderMenuAnchor !== null"
          @click.stop="toggleSenderMenu"
        >
          <Avatar :name="senderName" :email="props.message.from.email" :size="avatarSize" />
        </CardButton>
        <Avatar v-else :name="senderName" :email="props.message.from.email" :size="avatarSize" />
        <div class="min-w-0">
          <!-- A folded message keeps one line and lets its snippet give way. -->
          <div class="flex items-center gap-x-2 gap-y-0.5">
            <AppButton
              class="sender-identity-trigger text-md font-semibold text-primary"
              :aria-label="$t('mail.recipients.showSenderDetails', { name: senderName })"
              @click.stop="openSenderDetails"
              variant="link"
              size="sm"
            >
              {{ senderName }}
            </AppButton>
            <span v-if="props.collapsed && collapsedSnippet" class="message-collapsed-snippet">
              {{ collapsedSnippet }}
            </span>
            <MessageSecurityBadge
              :smime-state="smimeDisplayState"
              :pgp-state="pgpIndicatorState"
              :authentication="authenticationTone"
              :from-address="props.message.from.email"
              :smime-signer="props.message.smime?.signerEmail ?? props.message.smime?.signerCn"
              :pgp-signer="props.message.pgp?.signerName"
              :pgp-signer-fingerprint="props.message.pgp?.signerFingerprint"
              @show-details="openSenderDetails"
            />
          </div>
          <div class="message-address-line mt-0.5 text-xs text-secondary">
            <AppButton
              v-if="props.message.from.email"
              class="message-sender-email sender-identity-trigger"
              :aria-label="$t('mail.recipients.showSenderDetails', { name: senderName })"
              @click.stop="openSenderDetails"
              variant="link"
              size="sm"
            >
              {{ props.message.from.email }}
            </AppButton>
            <span v-if="props.message.from.email && (props.message.to.length || props.message.cc?.length)" class="message-address-separator" aria-hidden="true">&middot;</span>
            <MessageRecipientDetails
              ref="recipientDetailsRef"
              :from="props.message.from"
              :to="props.message.to"
              :cc="props.message.cc"
              :subject="props.message.subject"
              :timestamp="props.message.timestamp"
              :reply-to="props.message.replyTo"
              :auth-spf="props.message.authSpf"
              :auth-dkim="props.message.authDkim"
              :auth-dmarc="props.message.authDmarc"
              :smime-state="smimeDisplayState"
              :smime-signer-email="props.message.smime?.signerEmail"
              :smime-signer-cn="props.message.smime?.signerCn"
            />
          </div>
        </div>
      </div>
      <div class="flex items-start gap-1.5 shrink-0">
        <!-- Desktop contextual actions, on hover beside the date: kept in the
             header, a hidden row under every message would hold its height
             open and push the rhythm apart. Mobile has the fixed thread bar. -->
        <div
          v-if="!ui.isMobile"
          class="message-hover-actions opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
        >
          <template v-if="!props.collapsed">
            <template v-if="props.canReply">
              <IconButton size="sm" :title="$t('common.reply')" @click="emit('reply')">
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M6 3L1 8l5 5"/>
                  <path d="M1 8h9a5 5 0 015 5"/>
                </svg>
              </IconButton>
              <IconButton size="sm" :title="$t('common.forward')" @click="emit('forward')">
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M10 3l5 5-5 5"/>
                  <path d="M15 8H6a5 5 0 00-5 5"/>
                </svg>
              </IconButton>
            </template>
            <IconButton size="sm" :title="$t('mail.message.showSource')" @click="sourceDialogOpen = true">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <path d="M6 4L2.5 8 6 12"/>
                <path d="M10 4l3.5 4L10 12"/>
              </svg>
            </IconButton>
          </template>
          <IconButton
            size="sm"
            :title="$t('mail.message.addToTopicSingle')"
            @click="topicsStore.openPicker({ itemType: 'message', itemIds: [props.message.id] })"
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M2.5 4.5C2.5 3.7 3.2 3 4 3H6.5L8 4.5H12C12.8 4.5 13.5 5.2 13.5 6V11.5C13.5 12.3 12.8 13 12 13H4C3.2 13 2.5 12.3 2.5 11.5V4.5Z" />
              <path d="M8 7V11M6 9H10" />
            </svg>
          </IconButton>
        </div>
        <div class="text-right">
          <div class="text-xs text-secondary font-medium">{{ fullTimestamp }}</div>
        </div>
        <IconButton
          v-if="!ui.isMobile"
          class="message-collapse-toggle"
          :title="props.collapsed ? $t('mail.message.expand') : $t('mail.message.collapse')"
          :aria-label="props.collapsed ? $t('mail.message.expand') : $t('mail.message.collapse')"
          @click="emit('toggleCollapsed')"
        >
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <path d="m5 6 3 3 3-3" />
          </svg>
        </IconButton>
        <IconButton
          v-else
          size="lg"
          :title="$t('mail.message.actions')"
          @click="messageActionsOpen = true"
        >
          <AppIcon size="lg"><circle cx="3" cy="8" r=".75" fill="currentColor" /><circle cx="8" cy="8" r=".75" fill="currentColor" /><circle cx="13" cy="8" r=".75" fill="currentColor" /></AppIcon>
        </IconButton>
      </div>
    </div>

    <!-- A folded message is its header line; nothing below it is built. -->
    <template v-if="!props.collapsed">
    <!-- Sender spoofing / authentication warning -->
    <aside v-if="spoofingWarning" class="spoofing-notice" role="alert">
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M8 2.2 14.4 13H1.6L8 2.2Z" />
        <path d="M8 6.4v3.2" />
        <path d="M8 11.6h.01" />
      </svg>
      <span>{{ spoofingWarning.text }}</span>
    </aside>

    <!-- PGP Status -->
    <PgpStatusCard
      v-if="showPgpCard && pgpState"
      class="mt-4"
      :state="pgpState"
      :error-detail="props.message.pgp?.errorDetail"
      :signer-name="props.message.pgp?.signerName"
      :signer-fingerprint="props.message.pgp?.signerFingerprint"
      :from-address="props.message.from.email"
      @import-key="openKeyImport('pgp')"
      @unlock="handleUnlock"
      @retry="messagesStore.reloadPgpMessages(props.message.threadId)"
      @dismiss="dismissPgpCard"
    />
    <p v-if="pgpStatus" role="status" class="mt-1 text-xs text-secondary">{{ pgpStatus }}</p>

    <SmimeStatusCard
      v-if="showSmimeCard && smimeState"
      class="mt-4"
      :state="smimeState"
      :error-detail="props.message.smime?.errorDetail"
      :signer-email="props.message.smime?.signerEmail"
      :signer-cn="props.message.smime?.signerCn"
      :from-address="props.message.from.email"
      @import-cert="openKeyImport('smime')"
      @unlock="handleSmimeUnlock"
      @dismiss="dismissSmimeCard"
    />
    <p v-if="smimeStatus" class="mt-1 font-mono text-xs text-accent-green">{{ smimeStatus }}</p>

    <!-- Calendar invite (auto-detected ICS) -->
    <InvitationCard v-if="matchingInvite" :invite="matchingInvite" />

    <!-- Newsletter / List-Unsubscribe -->
    <UnsubscribeBanner
      v-if="props.message.listUnsubscribe"
      :message-id="props.message.id"
    />

    <aside v-if="inlineImagesFailed" class="inline-images-notice">
      <span>
        {{ $t('mail.message.inlineImagesFailed') }}
        <span class="detail">{{ inlineImagesDetail }}</span>
      </span>
      <AppButton
        size="sm"
        variant="ghost"
        :disabled="inlineImagesBusy"
        @click="retryInlineImages"
      >
        {{ inlineImagesBusy ? $t('mail.message.loadingImages') : $t('mail.message.reloadImages') }}
      </AppButton>
    </aside>

    <!-- Body -->
    <div v-if="showBody" ref="bodyHost" class="message-body-host">
      <template v-if="!renderBody">
        <div
          class="message-body-placeholder"
          :style="{ minHeight: `${placeholderHeight}px` }"
          aria-hidden="true"
        />
      </template>
      <template v-else-if="props.message.sourcePayload">
        <ItemRenderer
          :message="props.message"
          :show-body="props.message.sourcePayload.kind !== 'rss' || !props.message.bodyHtml"
        />
        <EmailRenderer
          v-if="props.message.sourcePayload.kind === 'rss' && props.message.bodyHtml"
          class="mt-3 text-secondary leading-relaxed"
          :html="props.message.bodyHtml"
          :text="visibleBodyText"
          :sender-email="props.message.from?.email"
          :trusted-dmarc-pass="props.message.authDmarc === 'pass'"
          :attachments="props.message.attachments"
          :normalize-quotes="settings.collapseQuotedHistory"
          :cache-key="bodyCacheKey"
        />
      </template>
      <EmailRenderer
        v-else
        class="mt-3 text-secondary leading-relaxed"
        :html="props.message.bodyHtml"
        :text="visibleBodyText"
        :pgp-state="pgpState"
        :can-decrypt="true"
        :decrypting="pgpBusy"
        :sender-email="props.message.from?.email"
        :trusted-dmarc-pass="props.message.authDmarc === 'pass'"
        :attachments="props.message.attachments"
        :normalize-quotes="settings.collapseQuotedHistory"
        :cache-key="bodyCacheKey"
        @request-unlock="handleUnlock"
      />
    </div>
    <div
      v-if="!bodyLoaded"
      class="message-body-loading mt-3"
      role="status"
      :aria-label="$t('mail.message.loading')"
      aria-busy="true"
    >
      <div v-if="showBodySkeleton" class="message-body-skeleton" aria-hidden="true">
        <i /><i /><i />
      </div>
    </div>
    <AppBanner
      v-else-if="props.message.bodyLoadError"
      class="mt-4"
      tone="danger"
      :title="$t('mail.message.bodyLoadFailed')"
      :detail="props.message.bodyLoadError"
    />

    <!-- Attachments -->
    <div v-if="visibleAttachments.length > 0" class="mt-4 flex flex-wrap gap-2">
      <AttachmentCard
        v-for="attachment in visibleAttachments"
        :key="attachment.id"
        :attachment="attachment"
      />
    </div>

    </template>

    <!-- Dialogs mount on first use: a long thread has a hundred closed ones. -->
    <MessageSourceDialog
      v-if="sourceDialogUsed"
      :open="sourceDialogOpen"
      :message-id="props.message.id"
      :remote-uid-validity="props.message.remoteUidValidity"
      :content-error="props.message.contentError"
      @close="sourceDialogOpen = false"
    />

    <Teleport v-if="messageActionsUsed" to="body">
      <BottomSheet
        :label="$t('mail.message.actions')"
        :open="messageActionsOpen"
        @dismiss="messageActionsOpen = false"
      >
        <nav role="menu" class="message-action-sheet" :aria-label="$t('mail.message.actionsNav')">
          <h2>{{ $t('mail.message.actions') }}</h2>
          <MenuItem label=""
            v-if="props.canReply"
            @select="runMessageAction(() => emit('reply'))"
          >
            <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m8 4-5 5 5 5" /><path d="M3 9h9a5 5 0 0 1 5 5" /></svg></template>
            {{ $t('common.reply') }}
          </MenuItem>
          <MenuItem label=""
            v-if="props.canReply"
            @select="runMessageAction(() => emit('forward'))"
          >
            <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m12 4 5 5-5 5" /><path d="M17 9H8a5 5 0 0 0-5 5" /></svg></template>
            {{ $t('common.forward') }}
          </MenuItem>
          <MenuItem label=""
            @select="runMessageAction(() => topicsStore.openPicker({ itemType: 'message', itemIds: [props.message.id] }))"
          >
            <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 5a2 2 0 0 1 2-2h3l2 2h5a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /><path d="M10 8v6M7 11h6" /></svg></template>
            {{ $t('mail.message.addToTopic') }}
          </MenuItem>
          <MenuItem label="" @select="runMessageAction(() => { sourceDialogOpen = true; })">
            <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m8 5-4 5 4 5M12 5l4 5-4 5" /></svg></template>
            {{ $t('mail.message.showSource') }}
          </MenuItem>
          <MenuItem label="" @select="runMessageAction(() => emit('toggleCollapsed'))">
            <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path v-if="props.collapsed" d="m5 8 5 5 5-5" /><path v-else d="m5 12 5-5 5 5" /></svg></template>
            {{ props.collapsed ? $t('mail.message.expand') : $t('mail.message.collapse') }}
          </MenuItem>
        </nav>
      </BottomSheet>
    </Teleport>

    <ContextMenu
      v-if="senderMenuUsed"
      :open="senderMenuAnchor !== null"
      :anchor="senderMenuAnchor"
      :label="$t('mail.message.fileSender', { name: senderName })"
      :width="236"
      @close="senderMenuAnchor = null"
    >
      <SenderFilingMenu
        :account-id="senderAccountId"
        :email="props.message.from.email"
        @close="senderMenuAnchor = null"
      />
    </ContextMenu>

    <PassphraseDialog
      v-if="passphraseRequest"
      :scheme="passphraseRequest"
      @submit="submitPassphrase"
      @cancel="cancelPassphrase"
    />
  </article>
</template>

<style scoped>
/* The avatar is the affordance, so the button adds only a focus/open ring —
   a bordered wrapper would put a second circle around a circle.
   It must not shrink: `CardButton` allows it, and a long address squeezed
   the button under the avatar into an ellipse the avatar half covered. */
.sender-avatar-trigger {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
  padding: 0;
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  cursor: pointer;
}

/* Drawn inside the avatar's edge, over it: the surface is `overflow-hidden`
   with no side padding, so a ring outside the avatar lost its left half. */
.sender-avatar-trigger::after {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: inherit;
  box-shadow: inset 0 0 0 2px transparent;
  pointer-events: none;
  transition: box-shadow var(--duration-fast) var(--ease-out);
}

.sender-avatar-trigger:hover::after,
.sender-avatar-trigger--open::after {
  box-shadow:
    inset 0 0 0 2px color-mix(in srgb, var(--accent-primary) 70%, transparent),
    inset 0 0 0 4px var(--bg-surface);
}

/* The body, attachments and banners line up under the sender's name; the
   header keeps the avatar in the margin. 52px = the 40px avatar plus its gap. */
.message-surface--indented > :not(.message-header) {
  padding-left: 52px;
}

.message-surface {
  /* Long expanded conversations can contain many HTML documents and images.
     Let the browser skip layout and paint for messages well outside the
     viewport while retaining a representative block size for stable scroll. */
  content-visibility: auto;
  contain-intrinsic-block-size: auto 320px;
  outline: none;
  transition:
    background var(--duration-fast) var(--ease-out),
    border-color var(--duration-fast) var(--ease-out),
    box-shadow var(--duration-fast) var(--ease-out);
}

.message-surface--keyboard-focused {
  border-color: color-mix(in srgb, var(--accent-primary) 48%, var(--border-color));
  background: color-mix(in srgb, var(--accent-primary) 10%, var(--bg-surface));
  box-shadow:
    inset 0 0 0 1px color-mix(in srgb, var(--accent-primary) 34%, transparent),
    var(--shadow-glow);
}

.message-surface--collapsed {
  cursor: pointer;
}

.message-surface--collapsed > :not(.message-header) {
  display: none;
}

/*
 * Folded away, a message is a line in the thread rather than a header with
 * nothing under it: the avatar shrinks to a marker, the address line goes,
 * and the snippet takes the space the body had.
 */
.message-surface--collapsed .message-sender-group {
  gap: var(--space-2-5);
}

.message-surface--collapsed .message-address-line {
  display: none;
}

/* The name keeps its width; the snippet is what gives way. */
.message-surface--collapsed .sender-identity-trigger { flex-shrink: 0; }

.message-collapsed-snippet {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

.sender-identity-trigger { min-width: 0; }

.message-address-line {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: var(--space-xs);
}

.message-sender-email {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.message-address-separator {
  flex: 0 0 auto;
}

@media (max-width: 767px) {
  .message-sender-email,
  .message-address-separator {
    display: none;
  }
}

.message-hover-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2xs);
  /* Pulled up to the date's line: the icons are taller than its text. */
  margin-top: calc(var(--space-xs) * -1);
}

.message-collapse-toggle { width: 24px; flex: 0 0 auto; }

.message-collapse-toggle svg {
  width: 14px;
  height: 14px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.6;
}

.message-action-sheet {
  display: grid;
  width: min(420px, 100vw);
  padding: 8px 12px calc(14px + var(--satchel-safe-area-bottom));
}

.message-action-sheet h2 {
  margin: 0;
  padding: 10px 8px 12px;
  color: var(--text-primary);
  font-size: var(--font-size-lg);
}

@media (prefers-reduced-motion: reduce) {
}

.inline-images-notice {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  margin: 8px 0;
  border: 1px solid color-mix(in srgb, var(--accent-amber) 26%, var(--border-color));
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-amber) 10%, var(--bg-raised));
  padding: 8px 12px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.inline-images-notice .detail {
  display: block;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.spoofing-notice {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-top: 10px;
  border: 1px solid color-mix(in srgb, var(--accent-red) 30%, var(--border-color));
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-red) 10%, var(--bg-raised));
  padding: 8px 12px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.spoofing-notice svg {
  flex: 0 0 auto;
  margin-top: 2px;
  color: var(--accent-red);
}

.message-body-loading {
  min-height: 58px;
}

.message-body-placeholder {
  margin-top: var(--space-md);
}

.message-body-skeleton {
  display: grid;
  gap: 10px;
  width: min(100%, 620px);
  animation: message-body-skeleton-pulse 1.2s ease-in-out infinite alternate;
}

.message-body-skeleton i {
  display: block;
  width: 94%;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--bg-raised);
}

.message-body-skeleton i:nth-child(2) { width: 82%; }
.message-body-skeleton i:nth-child(3) { width: 58%; }

@keyframes message-body-skeleton-pulse {
  to { opacity: 0.52; }
}

@media (prefers-reduced-motion: reduce) {
  .message-body-skeleton { animation: none; }
}
</style>
