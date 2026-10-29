import type { Priority, WorkflowState } from './workflow';

export interface Participant {
  name: string;
  email: string;
}

export interface Attachment {
  id: string;
  filename: string;
  contentType: string;
  size: number;
  isInline?: boolean;
  contentId?: string | null;
  cachedPath?: string | null;
  dataUrl?: string | null;
}

export type PgpMessageState =
  | 'encrypted_no_key'
  | 'key_available_locked'
  | 'decrypting'
  | 'decrypted'
  | 'signature_valid'
  | 'signature_partial'
  /** The maths held, but the signing key does not carry the `From:` mailbox. */
  | 'signature_identity_mismatch'
  | 'signature_invalid'
  | 'signature_unknown_key'
  | 'error';

/** PGP states requiring an inline warning next to the sender. */
export type PgpIndicatorState = Extract<PgpMessageState, 'signature_identity_mismatch' | 'signature_partial'>;

export interface MessagePgpStatus {
  state: PgpMessageState;
  errorDetail?: string;
  signerName?: string;
  signerFingerprint?: string;
}

export type SmimeMessageState =
  | 'encrypted_no_cert'
  | 'cert_available_locked'
  | 'decrypting'
  | 'decrypted'
  | 'signature_valid'
  | 'signature_partial'
  /** The maths held, but no certificate authority vouches for the signer. */
  | 'signature_unchained'
  /** The chain held, but the certificate speaks for another mailbox. */
  | 'signature_identity_mismatch'
  | 'signature_invalid'
  | 'error';

/** UI-only state while a detached signature is known but not verified yet. */
export type SmimeDisplayState = SmimeMessageState | 'signature_unverified';
export type SmimeIndicatorState = Extract<
  SmimeMessageState,
  'signature_valid' | 'signature_partial' | 'signature_unchained' | 'signature_identity_mismatch' | 'signature_invalid' | 'decrypted'
> | 'signature_unverified';

export interface MessageSmimeStatus {
  state: SmimeMessageState;
  errorDetail?: string;
  signerEmail?: string;
  signerCn?: string;
}

export interface Message {
  id: string;
  /** Saved IMAP generation; a change invalidates body and inline-image caches. */
  remoteUidValidity?: number | null;
  threadId: string;
  from: Participant;
  to: Participant[];
  cc?: Participant[];
  replyTo?: string;
  subject: string;
  bodyText: string;
  bodyHtml?: string;
  bodyLoaded?: boolean;
  bodyLoadError?: string;
  /** `cid:` images still waiting for their bytes; the body keeps the `cid:` URL. */
  inlineImagesPending?: number;
  /** Why the last inline image fetch failed, if it did. */
  inlineImagesError?: string;
  timestamp: string;
  isRead: boolean;
  isDraft?: boolean;
  attachments: Attachment[];
  pgp?: MessagePgpStatus;
  smime?: MessageSmimeStatus;
  listUnsubscribe?: string | null;
  listUnsubscribePost?: string | null;
  authSpf?: string | null;
  authDkim?: string | null;
  authDmarc?: string | null;
  sourcePayload?: { kind?: string; level?: string; url?: string; data?: unknown };
}

export type ThreadCategory = 'personal' | 'updates' | 'promotions' | 'social';
export type ThreadSmartBucket = 'priority' | 'pinned' | 'invitations' | 'personal' | 'notifications' | 'newsletter';

export interface Thread {
  normalizedSubject?: string;
  id: string;
  accountId: string;
  subject: string;
  participants: Participant[];
  sentParticipants?: Participant[];
  snippet: string;
  lastMessageAt: string;
  /** Immutable stored date used by thread-list keyset pagination. */
  sortDate?: string;
  /** `lastMessageAt` as epoch ms. Every sort in the list path reads this. */
  lastMessageAtMs: number;
  messageCount: number;
  unreadCount: number;
  hasAttachments: boolean;
  hasDraft?: boolean;
  draftSnippet?: string;
  workflowState: WorkflowState;
  priority: Priority;
  boardPosition?: number;
  mailboxNames: string[];
  tags: string[];
  assignee?: string;
  sharedWith?: string[];
  dueDate?: string;
  snoozeUntil?: string;
  /** When the thread last changed workflow state; "waiting since" in Later. */
  workflowChangedAt?: string;
  /** Kept on the thread only; `messages.category` is unused (ADR 0013). */
  category?: ThreadCategory | null;
  /** Backend projection shared with smart-folder counts and queries. */
  smartBucket?: ThreadSmartBucket;
  isMuted?: boolean;
  isStarred?: boolean;
  senderKnown?: boolean;
  hasInvite?: boolean;
  inviteStatus?: string;
  inviteCancelled?: boolean;
}

export interface Account {
  id: string;
  displayName: string;
  email: string;
  provider: string;
  kind: 'email' | 'webhook' | 'script' | 'rss';
  color: string;
  /** A small `data:` URL the user chose; without it the account shows initials. */
  avatar?: string | null;
  lastOauthError?: string | null;
  lastOauthErrorAt?: string | null;
  lastSyncAt?: string | null;
  syncStatus?: string | null;
  lastSyncError?: string | null;
  lastSyncErrorAt?: string | null;
  syncProgressJson?: string | null;
}
