/**
 * `labelKey` stays a literal string: the viewer bundle only ships the keys its
 * module graph names, so a built key would be missing there at runtime.
 */
export const SWIPE_ACTION_OPTIONS = [
  { id: 'toggle-read', labelKey: 'swipe.toggleRead', tone: 'blue' },
  { id: 'done', labelKey: 'swipe.done', tone: 'green' },
  { id: 'pin', labelKey: 'swipe.pin', tone: 'amber' },
  { id: 'aside', labelKey: 'swipe.aside', tone: 'cyan' },
  { id: 'trash', labelKey: 'swipe.trash', tone: 'red' },
  { id: 'none', labelKey: 'swipe.none', tone: 'neutral' },
] as const;

export type SwipeActionId = typeof SWIPE_ACTION_OPTIONS[number]['id'];
export type SwipeActionTone = typeof SWIPE_ACTION_OPTIONS[number]['tone'];
export type SwipeSlot = 'rightShort' | 'rightLong' | 'leftShort' | 'leftLong';

export interface SwipePreferences {
  rightShort: SwipeActionId;
  rightLong: SwipeActionId;
  leftShort: SwipeActionId;
  leftLong: SwipeActionId;
}

export const DEFAULT_SWIPE_PREFERENCES: SwipePreferences = {
  rightShort: 'toggle-read',
  rightLong: 'done',
  leftShort: 'pin',
  leftLong: 'trash',
};

const swipeActionIds = new Set<SwipeActionId>(
  SWIPE_ACTION_OPTIONS.map((option) => option.id),
);

export function isSwipeActionId(value: unknown): value is SwipeActionId {
  return typeof value === 'string' && swipeActionIds.has(value as SwipeActionId);
}

export function swipeActionTone(action: SwipeActionId): SwipeActionTone {
  return SWIPE_ACTION_OPTIONS.find((option) => option.id === action)?.tone ?? 'neutral';
}
