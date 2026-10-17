import type { RefinedWireFields } from '../services/backend/wireTypes';
import type { MessageDto as WireMessageDto } from '../services/backend/generated/app/commands/messages/MessageDto';
import type { MessageBodyDto as WireMessageBodyDto } from '../services/backend/generated/app/commands/messages/MessageBodyDto';
import type { MessageSearchHitDto as WireMessageSearchHitDto } from '../services/backend/generated/core/db/messages/MessageSearchHitDto';
import type { SearchWarningDto as WireSearchWarningDto } from '../services/backend/generated/core/db/messages/SearchWarningDto';
import type { MessageSearchResponseDto as WireMessageSearchResponseDto } from '../services/backend/generated/core/db/messages/MessageSearchResponseDto';
import type { UnsubscribeResult as WireUnsubscribeResult } from '../services/backend/generated/app/commands/unsubscribe/UnsubscribeResult';
import type { AttachmentDto as WireAttachmentDto } from '../services/backend/generated/app/commands/attachments/AttachmentDto';
import type { DownloadAttachmentResult as WireDownloadAttachmentResult } from '../services/backend/generated/app/commands/attachments/DownloadAttachmentResult';
import type { ServerSearchHit as WireServerSearchHit } from '../services/backend/generated/app/commands/server_search/ServerSearchHit';
import type { LoadedMessageBody as WireLoadedMessageBody } from '../services/backend/generated/app/commands/message_body/LoadedMessageBody';
import type { InlineCacheProgress as WireInlineCacheProgress } from '../services/backend/generated/app/commands/inline_cache/InlineCacheProgress';
import type { InlineCacheStatus as WireInlineCacheStatus } from '../services/backend/generated/app/commands/inline_cache/InlineCacheStatus';
import type { MessageSourceDto as WireMessageSourceDto } from '../services/backend/generated/app/commands/messages/MessageSourceDto';
import type { OptionalWireFields } from '../services/backend/wireTypes';
export type BackendMessage = OptionalWireFields<WireMessageDto, 'reply_to' | 'body_html' | 'pgp_state' | 'pgp_error_detail' | 'pgp_signer_name' | 'pgp_signer_fingerprint' | 'smime_state' | 'smime_signer_email' | 'smime_signer_cn' | 'smime_error_detail' | 'list_unsubscribe' | 'list_unsubscribe_post' | 'auth_spf' | 'auth_dkim' | 'auth_dmarc' | 'source_payload'>;

export type BackendMessageBody = OptionalWireFields<WireMessageBodyDto, 'body_html' | 'pgp_state' | 'pgp_error_detail' | 'pgp_signer_name' | 'pgp_signer_fingerprint' | 'smime_state' | 'smime_signer_email' | 'smime_signer_cn' | 'smime_error_detail' | 'auth_spf' | 'auth_dkim' | 'auth_dmarc' | 'source_payload'>;

export type BackendMessageSearchHit = WireMessageSearchHitDto;

export type BackendSearchWarning = WireSearchWarningDto;

export type BackendMessageSearchResponse = WireMessageSearchResponseDto;

export type BackendUnsubscribeResult = WireUnsubscribeResult;

export type BackendAttachmentDto = OptionalWireFields<WireAttachmentDto, 'filename' | 'size' | 'content_id' | 'cached_path'>;

export type BackendDownloadAttachmentResult = OptionalWireFields<WireDownloadAttachmentResult, 'cached_path'>;

export type BackendServerSearchHit = OptionalWireFields<WireServerSearchHit, 'date'>;

export type BackendLoadedMessageBody = WireLoadedMessageBody;

/** A thread as the reader opens it: every preview, plus the latest bodies. */
export interface BackendOpenedThread {
  messages: BackendMessage[];
  bodies: BackendMessageBody[];
}

export type BackendInlineCacheProgress = WireInlineCacheProgress;

export type BackendInlineCacheStatus = WireInlineCacheStatus;

export type BackendMessageSource = RefinedWireFields<OptionalWireFields<WireMessageSourceDto, 'subject' | 'raw_headers' | 'body_text' | 'body_html' | 'raw_size' | 'raw_source' | 'raw_source_status'>, { raw_source_status?: 'loaded' | 'unavailable' | 'unsupported' }>;
