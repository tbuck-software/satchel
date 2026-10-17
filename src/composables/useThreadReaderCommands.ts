import { shallowRef } from 'vue';

export type ReaderMessageScope = 'latest' | 'conversation';
export type ReaderExportFormat = 'eml' | 'markdown' | 'text';

/** The reader actions that need its loaded messages, offered to the palette and the keyboard. */
export interface ThreadReaderCommands {
  readonly threadId: string;
  /** Read reactively: a getter over the reader's own computed. */
  readonly moveTargets: ReadonlyArray<{ id: string; label: string }>;
  readonly isMuted: boolean;
  move: (targetId: string) => unknown;
  toggleMuted: () => unknown;
  markSpam: () => unknown;
  blockSender: (scope: 'address' | 'domain') => unknown;
  print: (scope: ReaderMessageScope) => unknown;
  exportThread: (format: ReaderExportFormat) => unknown;
}

const activeReader = shallowRef<ThreadReaderCommands | null>(null);

/**
 * Bridges the palette and the global shortcut handler to the mounted reader,
 * the way `registerThreadMessageKeyboardNavigation` does for J/K.
 */
export function registerThreadReaderCommands(commands: ThreadReaderCommands): () => void {
  activeReader.value = commands;
  return () => {
    if (activeReader.value === commands) activeReader.value = null;
  };
}

export function activeThreadReaderCommands(): ThreadReaderCommands | null {
  return activeReader.value;
}
