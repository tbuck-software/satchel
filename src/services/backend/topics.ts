// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';
import type {
  BackendTopic,
  BackendTopicFile,
  BackendTopicMembership,
  BackendTopicNote,
  BackendTopicTimelineItem,
  TopicItemType,
  TopicStatus,
} from '@/types/topics';

export type {
  BackendTopic,
  BackendTopicFile,
  BackendTopicMembership,
  BackendTopicNote,
  BackendTopicTimelineItem,
  TopicItemType,
  TopicStatus,
} from '@/types/topics';


export async function backendListTopics(): Promise<BackendTopic[]> {
  return invoke<BackendTopic[]>('list_topics');
}


export async function backendUpsertTopic(input: {
  id?: string | null;
  name: string;
  description?: string | null;
  color?: string | null;
  icon?: string | null;
  status?: TopicStatus;
  /** snake_case so a spread BackendTopic carries it; null clears the date. */
  due_date?: string | null;
}): Promise<BackendTopic> {
  return invoke<BackendTopic>('upsert_topic', {
    input: {
      id: input.id ?? null,
      name: input.name,
      description: input.description ?? null,
      color: input.color ?? null,
      icon: input.icon ?? null,
      status: input.status ?? null,
      due_date: input.due_date ?? null,
    },
  });
}


export async function backendDeleteTopic(id: string): Promise<void> {
  return invoke<void>('delete_topic', { id });
}


export async function backendAddTopicItems(input: {
  topicId: string;
  itemType: TopicItemType;
  itemIds: string[];
  addedBy?: 'user' | 'rule' | 'llm';
}): Promise<string[]> {
  return invoke<string[]>('add_topic_items', {
    input: {
      topic_id: input.topicId,
      item_type: input.itemType,
      item_ids: input.itemIds,
      added_by: input.addedBy ?? null,
    },
  });
}


export async function backendRemoveTopicItem(input: {
  topicId: string;
  itemType: TopicItemType;
  itemId: string;
}): Promise<boolean> {
  return invoke<boolean>('remove_topic_item', {
    input: {
      topic_id: input.topicId,
      item_type: input.itemType,
      item_id: input.itemId,
    },
  });
}


export async function backendListTopicMemberships(
  itemType: TopicItemType,
  itemIds: string[],
): Promise<BackendTopicMembership[]> {
  return invoke<BackendTopicMembership[]>('list_topic_memberships', {
    input: { item_type: itemType, item_ids: itemIds },
  });
}


export async function backendGetTopicTimeline(
  topicId: string,
): Promise<BackendTopicTimelineItem[]> {
  return invoke<BackendTopicTimelineItem[]>('get_topic_timeline', { topicId });
}


export async function backendUpsertTopicNote(input: {
  id?: string | null;
  topicId: string;
  content: string;
}): Promise<BackendTopicNote> {
  return invoke<BackendTopicNote>('upsert_topic_note', {
    input: {
      id: input.id ?? null,
      topic_id: input.topicId,
      content: input.content,
    },
  });
}


export async function backendDeleteTopicNote(id: string): Promise<void> {
  return invoke<void>('delete_topic_note', { id });
}


export async function backendAddTopicFile(input: {
  topicId: string;
  filename: string;
  mimeType?: string;
  contentBase64: string;
}): Promise<BackendTopicFile> {
  return invoke<BackendTopicFile>('add_topic_file', {
    input: {
      topic_id: input.topicId,
      filename: input.filename,
      mime_type: input.mimeType ?? null,
      content_base64: input.contentBase64,
    },
  });
}


export async function backendListTopicFiles(topicId: string): Promise<BackendTopicFile[]> {
  return invoke<BackendTopicFile[]>('list_topic_files', { topicId });
}


export async function backendDownloadTopicFile(
  id: string,
): Promise<{ filename: string; mime_type: string; data_base64: string }> {
  return invoke('download_topic_file', { id });
}


export async function backendDeleteTopicFile(id: string): Promise<void> {
  return invoke<void>('delete_topic_file', { id });
}
