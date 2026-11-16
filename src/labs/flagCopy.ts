import type { LabsFlagId } from './flags';

/**
 * Title and description of each Labs flag. Only the Labs section imports this,
 * so the strings load with Settings and never at startup (see `./flags`).
 */
export const LABS_FLAG_COPY = {
  chats: {
    titleKey: 'settings.labs.flags.chats.title',
    descriptionKey: 'settings.labs.flags.chats.description',
  },
} as const satisfies Record<LabsFlagId, { titleKey: string; descriptionKey: string }>;
