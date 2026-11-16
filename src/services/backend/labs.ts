import type { LabsFlagDto } from './generated/app/commands/labs/LabsFlagDto';
import { invoke } from './core';

export type BackendLabsFlag = LabsFlagDto;

/** Every flag the backend knows, with its state on this device. */
export async function backendListLabsFlags(): Promise<BackendLabsFlag[]> {
  return invoke<BackendLabsFlag[]>('list_labs_flags');
}

export async function backendSetLabsFlag(id: string, enabled: boolean): Promise<BackendLabsFlag> {
  return invoke<BackendLabsFlag>('set_labs_flag', { input: { id, enabled } });
}
