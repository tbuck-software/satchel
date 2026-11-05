import type { ServerRulesOverviewDto as WireServerRulesOverviewDto } from './generated/app/commands/server_rules/ServerRulesOverviewDto';
import type { ServerRulesAccountDto as WireServerRulesAccountDto } from './generated/app/commands/server_rules/ServerRulesAccountDto';
import type { RulePlacementDto as WireRulePlacementDto } from './generated/app/commands/server_rules/RulePlacementDto';
import { invoke } from './core';

/** Server-side rules (ManageSieve) of every IMAP account, and where each rule runs. */
export type BackendServerRulesOverview = WireServerRulesOverviewDto;
export type BackendServerRulesAccount = WireServerRulesAccountDto;
export type BackendRulePlacement = WireRulePlacementDto;

/**
 * What to do when another Sieve script is active on the server: `ask` never
 * takes it over, `wrapper` keeps it running beside Satchel's (needs the
 * `include` extension), `replace` makes Satchel's the active one instead.
 */
export type BackendServerRulesChoice = 'ask' | 'wrapper' | 'replace';

export async function backendGetServerRulesOverview(): Promise<BackendServerRulesOverview> {
  return invoke<BackendServerRulesOverview>('get_server_rules_overview');
}

export async function backendDetectServerRules(accountId: string): Promise<BackendServerRulesOverview> {
  return invoke<BackendServerRulesOverview>('detect_server_rules', { accountId });
}

export async function backendSetServerRulesEndpoint(input: {
  accountId: string;
  host: string | null;
  port: number | null;
  tls: 'starttls' | 'tls' | null;
}): Promise<BackendServerRulesOverview> {
  return invoke<BackendServerRulesOverview>('set_server_rules_endpoint', {
    input: { account_id: input.accountId, host: input.host, port: input.port, tls: input.tls },
  });
}

export async function backendEnableServerRules(
  accountId: string,
  choice: BackendServerRulesChoice,
): Promise<BackendServerRulesOverview> {
  return invoke<BackendServerRulesOverview>('enable_server_rules', {
    input: { account_id: accountId, choice },
  });
}

export async function backendDisableServerRules(accountId: string): Promise<BackendServerRulesOverview> {
  return invoke<BackendServerRulesOverview>('disable_server_rules', { accountId });
}
