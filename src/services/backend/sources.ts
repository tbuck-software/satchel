import type { PluginDto as WirePluginDto } from './generated/app/commands/plugins/PluginDto';
import type { PluginAuditRecord as WirePluginAuditRecord } from './generated/app/plugins/runtime/PluginAuditRecord';
import type { WebhookDto as WireWebhookDto } from './generated/app/commands/webhooks/WebhookDto';
import type { SourceDto as WireSourceDto } from './generated/app/commands/sources/SourceDto';
import type { SourceCapabilities as WireSourceCapabilities } from './generated/app/commands/sources/SourceCapabilities';
import type { OpmlImportSummary as WireOpmlImportSummary } from './generated/app/commands/sources/OpmlImportSummary';
import type { OptionalWireFields } from './wireTypes';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';
import type { BackendMessageSource } from '@/types/messages';

export type { BackendMessageSource } from '@/types/messages';

export type BackendPlugin = OptionalWireFields<WirePluginDto, 'signature' | 'disabled_by'>;


export type BackendPluginAuditRecord = OptionalWireFields<WirePluginAuditRecord, 'capability' | 'reason' | 'duration_ms'>;


export type BackendWebhook = WireWebhookDto;


export type BackendSource = OptionalWireFields<WireSourceDto, 'color' | 'sync_state'>;

export type SourceCapabilities = WireSourceCapabilities;

export async function backendListSources():Promise<BackendSource[]>{return invoke('list_sources')}

export async function backendCreateSource(input:{name:string;kind:'webhook'|'script'|'rss';identity:string;color?:string;config?:Record<string,unknown>}):Promise<BackendSource>{return invoke('create_source',{input:{...input,config:input.config??{}}})}

export async function backendDeleteSource(sourceId:string):Promise<void>{return invoke('delete_source',{sourceId})}

export async function backendGetSourceCapabilities():Promise<Record<string,SourceCapabilities>>{return invoke('get_source_capabilities')}

export async function backendPollSource(sourceId:string):Promise<number>{return invoke('poll_source',{sourceId})}

export type OpmlImportSummary = WireOpmlImportSummary;

export async function backendImportOpml(input:{opml:string;pollMinutes:number;notify:boolean}):Promise<OpmlImportSummary>{return invoke('import_opml',{input:{opml:input.opml,poll_minutes:input.pollMinutes,notify:input.notify}})}


export async function backendGetMessageSource(messageId: string): Promise<BackendMessageSource> {
  return invoke<BackendMessageSource>('get_message_source', { messageId });
}


export async function backendListPlugins(): Promise<BackendPlugin[]> {
  return invoke<BackendPlugin[]>('list_plugins');
}


export async function backendSetPluginPermission(input: {
  pluginId: string;
  capability: string;
  granted: boolean;
}): Promise<void> {
  return invoke<void>('set_plugin_permission', {
    input: {
      plugin_id: input.pluginId,
      capability: input.capability,
      granted: input.granted,
    },
  });
}


export async function backendSetPluginEnabled(input: {
  pluginId: string;
  enabled: boolean;
}): Promise<void> {
  return invoke<void>('set_plugin_enabled', {
    input: { plugin_id: input.pluginId, enabled: input.enabled },
  });
}


export async function backendExecutePluginCommand(input: {
  pluginId: string;
  command: string;
  payload: string;
}): Promise<string> {
  return invoke<string>('execute_plugin_command', {
    input: {
      plugin_id: input.pluginId,
      command: input.command,
      payload: input.payload,
    },
  });
}


export async function backendListPluginAuditLog(): Promise<BackendPluginAuditRecord[]> {
  return invoke<BackendPluginAuditRecord[]>('list_plugin_audit_log');
}


export async function backendListWebhooks(): Promise<BackendWebhook[]> {
  return invoke<BackendWebhook[]>('list_webhooks');
}


export async function backendUpsertWebhook(input: {
  id?: string;
  url: string;
  events: string[];
  enabled: boolean;
  secret?: string;
}): Promise<BackendWebhook> {
  return invoke<BackendWebhook>('upsert_webhook', { input });
}


export async function backendDeleteWebhook(id: string): Promise<void> {
  return invoke<void>('delete_webhook', { id });
}
