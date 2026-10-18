import type { ProjectedWireFields, RefinedWireFields } from './wireTypes';
import type { OAuthStatus as WireOAuthStatus } from './generated/app/commands/oauth/OAuthStatus';
import type { StartOAuthResponse as WireStartOAuthResponse } from './generated/app/commands/oauth/StartOAuthResponse';
import type { OAuthFlowPollResponse as WireOAuthFlowPollResponse } from './generated/app/commands/oauth/OAuthFlowPollResponse';
import type { OpenOAuthFlow as WireOpenOAuthFlow } from './generated/app/commands/oauth/OpenOAuthFlow';
import type { OptionalWireFields } from './wireTypes';
import type { AccountDto as WireAccountDto } from './generated/app/commands/accounts/AccountDto';
import type { DiscoveredService as WireDiscoveredService } from './generated/app/commands/onboarding/DiscoveredService';
import type { AccountDiscoveryDto as WireAccountDiscoveryDto } from './generated/app/commands/onboarding/AccountDiscoveryDto';
import type { GroupwareSyncSummary as WireGroupwareSyncSummary } from './generated/core/providers/dav/GroupwareSyncSummary';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';

export type BackendAccount = RefinedWireFields<OptionalWireFields<WireAccountDto, 'color' | 'last_oauth_error' | 'last_oauth_error_at' | 'last_sync_at' | 'sync_status' | 'last_sync_error' | 'last_sync_error_at' | 'sync_progress_json'>, { kind: 'email' | 'webhook' | 'script' | 'rss' }>;


export type BackendDiscoveredService = OptionalWireFields<WireDiscoveredService, 'host' | 'port' | 'collections'>;


export type BackendAccountDiscovery = ProjectedWireFields<WireAccountDiscoveryDto, { config: Record<string, unknown> }>;


export type BackendOauthStatus = OptionalWireFields<WireOAuthStatus, 'expires_at' | 'granted_scopes'>;


export type BackendOauthStartResult = OptionalWireFields<WireStartOAuthResponse, 'browser_error'>;


export type BackendOauthPollResult = OptionalWireFields<WireOAuthFlowPollResponse, 'result' | 'error'>;


/** An authorization the backend could still finish after a restart. */
export type BackendOpenOauthFlow = WireOpenOAuthFlow;


export async function backendListAccounts(): Promise<BackendAccount[]> {
  return invoke<BackendAccount[]>('list_accounts');
}


export async function backendAddAccount(payload: {
  displayName: string;
  email: string;
  provider: 'imap' | 'pop3' | 'gmail' | 'outlook';
  config?: Record<string, unknown>;
}): Promise<BackendAccount> {
  return invoke<BackendAccount>('add_account', {
    input: {
      display_name: payload.displayName,
      email: payload.email,
      provider: payload.provider,
      config: payload.config ?? {},
    },
  });
}


/** Stop syncing an account. The mail and the credentials stay in the file. */
export async function backendDeactivateAccount(accountId: string): Promise<void> {
  return invoke<void>('deactivate_account', { accountId });
}

/** Remove an account and everything belonging to it. Not reversible. */
export async function backendDeleteAccount(accountId: string): Promise<void> {
  return invoke<void>('delete_account', { accountId });
}


export async function backendClearAccountOauthError(accountId: string): Promise<void> {
  return invoke<void>('clear_account_oauth_error', { accountId });
}


export async function backendTestImapAccountConnection(accountId: string): Promise<{
  account_id: string;
  ok: boolean;
  folders: number;
  folder_sample: string[];
}> {
  return invoke('test_imap_account_connection', { accountId });
}


/** Check the new password against the server, then store it in place of the old one. */
export async function backendUpdateAccountPassword(accountId: string, password: string): Promise<void> {
  return invoke<void>('update_account_password', { accountId, password });
}


/** The OAuth provider behind a domain's MX records, `null` when none or the lookup failed. */
export async function backendDetectMailProvider(domain: string): Promise<'gmail' | 'outlook' | null> {
  return invoke<'gmail' | 'outlook' | null>('detect_mail_provider', { domain });
}


export async function backendDiscoverAccount(input: {
  email: string;
  password: string;
  username?: string | null;
  protocol?: 'imap' | 'pop3' | null;
  imap?: { host: string; port: number } | null;
  pop3?: { host: string; port: number } | null;
  smtp?: { host: string; port: number } | null;
}): Promise<BackendAccountDiscovery> {
  return invoke<BackendAccountDiscovery>('discover_account', {
    input: {
      email: input.email,
      password: input.password,
      username: input.username ?? null,
      protocol: input.protocol ?? null,
      imap: input.imap ?? null,
      pop3: input.pop3 ?? null,
      smtp: input.smtp ?? null,
    },
  });
}


export async function backendAutoconfigureGroupware(accountId: string): Promise<{
  caldav_server?: string | null;
  carddav_server?: string | null;
  summary: BackendGroupwareSyncSummary;
}> {
  return invoke('autoconfigure_groupware', { accountId });
}


export type BackendGroupwareSyncSummary = WireGroupwareSyncSummary;


export async function backendConfigureGroupware(input: {
  accountId: string;
  username: string;
  password: string;
  caldavServer?: string | null;
  carddavServer?: string | null;
}): Promise<BackendGroupwareSyncSummary> {
  return invoke<BackendGroupwareSyncSummary>('configure_groupware', {
    input: {
      account_id: input.accountId,
      username: input.username,
      password: input.password,
      caldav_server: input.caldavServer ?? null,
      carddav_server: input.carddavServer ?? null,
    },
  });
}


export async function backendSyncGroupware(
  accountId?: string | null,
): Promise<BackendGroupwareSyncSummary> {
  return invoke<BackendGroupwareSyncSummary>('sync_groupware', {
    accountId: accountId ?? null,
  });
}


export async function backendRunImapSync(): Promise<{
  accounts_synced: number;
  accounts_failed: number;
  folders_synced: number;
  messages_synced: number;
  backfill_pending_accounts: number;
  backfill_remaining_messages: number;
  backfill_total_messages: number;
  failures: string[];
}> {
  return invoke('run_imap_sync');
}


export async function backendTestImapConnection(payload: {
  email: string;
  config: Record<string, unknown>;
}): Promise<{
  ok: boolean;
  folders: number;
  folder_sample: string[];
}> {
  return invoke('test_imap_connection', {
    input: {
      email: payload.email,
      config: payload.config,
    },
  });
}


export async function backendStartOauthFlow(payload: {
  accountId: string;
  provider: 'gmail' | 'outlook';
  purpose?: 'mail' | 'contacts' | 'calendar';
}): Promise<BackendOauthStartResult> {
  return invoke('start_oauth_flow', {
    input: {
      account_id: payload.accountId,
      provider: payload.provider,
      purpose: payload.purpose ?? null,
    },
  });
}


export async function backendPollOauthFlow(state: string): Promise<BackendOauthPollResult> {
  return invoke('poll_oauth_flow', {
    input: {
      state,
    },
  });
}


export async function backendCompleteOauthFlow(payload: {
  state: string;
  code: string;
}): Promise<BackendOauthStatus> {
  return invoke('complete_oauth_flow', {
    input: {
      state: payload.state,
      code: payload.code,
    },
  });
}


/**
 * End a flow the provider refused.
 *
 * A cancelled deep link carries no code to spend, so without this the flow
 * would stay pending until it times out.
 */
export async function backendAbortOauthFlow(state: string): Promise<void> {
  return invoke('abort_oauth_flow', {
    input: {
      state,
    },
  });
}


export async function backendListOpenOauthFlows(): Promise<BackendOpenOauthFlow[]> {
  return invoke('list_open_oauth_flows');
}


export async function backendOauthStatus(accountId: string): Promise<BackendOauthStatus> {
  return invoke('oauth_status', { accountId });
}

