import type { RefinedWireFields } from '../services/backend/wireTypes';
import type { TopicDto as WireTopicDto } from '../services/backend/generated/app/commands/topics/TopicDto';
import type { TopicMembershipDto as WireTopicMembershipDto } from '../services/backend/generated/app/commands/topics/TopicMembershipDto';
import type { TopicNoteDto as WireTopicNoteDto } from '../services/backend/generated/app/commands/topics/TopicNoteDto';
import type { TopicTimelineItemDto as WireTopicTimelineItemDto } from '../services/backend/generated/app/commands/topics/TopicTimelineItemDto';
import type { TopicFileDto as WireTopicFileDto } from '../services/backend/generated/app/commands/topics/TopicFileDto';
export type TopicStatus = 'active' | 'done' | 'archived';

export type TopicItemType = 'thread' | 'message' | 'event';

export type BackendTopic = RefinedWireFields<WireTopicDto, { status: TopicStatus }>;

export type BackendTopicMembership = WireTopicMembershipDto;

export type BackendTopicNote = WireTopicNoteDto;

export type BackendTopicTimelineItem = RefinedWireFields<WireTopicTimelineItemDto, { item_type: 'thread' | 'message' | 'note' }>;

export type BackendTopicFile = WireTopicFileDto;
