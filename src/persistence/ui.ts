import { createPersistedSlot, persistence } from './index';

const UI_STORAGE_KEYS = {
  sidebarWidth: 'satchel.ui.sidebarWidth',
  threadListWidth: 'satchel.ui.threadListWidth',
  newSendersExpanded: 'satchel.ui.newSendersExpanded',
} as const;

function widthSlot(key: string, fallback: number) {
  return createPersistedSlot<number>(key, {
    fallback: () => fallback,
    decode: (raw) => {
      const parsed = Number(raw);
      return Number.isFinite(parsed) ? parsed : undefined;
    },
    encode: (value) => String(value),
  });
}

const sidebarWidthSlot = widthSlot(UI_STORAGE_KEYS.sidebarWidth, 240);
const threadListWidthSlot = widthSlot(UI_STORAGE_KEYS.threadListWidth, 408);
// Collapsed by default: the review is a banner until someone asks for the rows.
const newSendersExpandedSlot = createPersistedSlot<boolean>(UI_STORAGE_KEYS.newSendersExpanded, {
  fallback: () => false,
  decode: (raw) => raw === 'true',
  encode: (value) => String(value),
});

export const uiPersistence = {
  sidebarWidth: {
    read: () => persistence.read(sidebarWidthSlot),
    write: (value: number) => {
      persistence.write(sidebarWidthSlot, value);
    },
  },
  threadListWidth: {
    read: () => persistence.read(threadListWidthSlot),
    write: (value: number) => {
      persistence.write(threadListWidthSlot, value);
    },
  },
  newSendersExpanded: {
    read: () => persistence.read(newSendersExpandedSlot),
    write: (value: boolean) => {
      persistence.write(newSendersExpandedSlot, value);
    },
  },
};
