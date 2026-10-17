import type { OutboxItemDto as WireOutboxItemDto } from '../services/backend/generated/app/commands/outbox/OutboxItemDto';
import type { OutboxAttachmentDto as WireOutboxAttachmentDto } from '../services/backend/generated/app/commands/outbox/OutboxAttachmentDto';
import type { OptionalWireFields } from '../services/backend/wireTypes';
export type BackendOutboxItem = OptionalWireFields<WireOutboxItemDto, 'last_error' | 'next_retry_at' | 'scheduled_for' | 'subject' | 'cc' | 'bcc' | 'body' | 'body_html' | 'attachments' | 'submission_started_at'>;

export type BackendOutboxAttachment = WireOutboxAttachmentDto;
