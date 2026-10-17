import { createPersistedSlot, persistence } from './index';

const UI_STORAGE_KEYS = {
  sidebarWidth: 'satchel.ui.sidebarWidth',
  threadListWidth: 'satchel.ui.threadListWidth',
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
};
