/**
 * Labs: experimental features someone turns on per device, in Settings → Labs.
 *
 * The registry is split by when it is needed. This half (ids, defaults,
 * restart) is read by the startup store and must stay free of message keys:
 * a key named here would ship in the startup catalogue for everyone, flag on
 * or off (ADR 0017). The words for each flag live in `./flagCopy`, which only
 * the Labs section imports; `satisfies` there keeps both halves in step.
 *
 * The backend refuses ids it does not know (`db::labs::LABS_FLAGS`).
 */
export interface LabsFlagSpec {
  /** Every flag ships off; the experiment is someone's choice. */
  defaultEnabled: false;
  /** The switch takes effect after a restart, and the section says so. */
  requiresRestart: boolean;
}

export const LABS_FLAGS = {
  /** Delta Chat–compatible chats: a Chats list, chat view and quick reply. */
  chats: { defaultEnabled: false, requiresRestart: false },
} as const satisfies Record<string, LabsFlagSpec>;

export type LabsFlagId = keyof typeof LABS_FLAGS;

export const LABS_FLAG_IDS = Object.keys(LABS_FLAGS) as LabsFlagId[];

export function isLabsFlagId(value: string): value is LabsFlagId {
  return Object.hasOwn(LABS_FLAGS, value);
}
