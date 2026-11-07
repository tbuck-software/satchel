import { nextTick, ref } from 'vue';
import { describe, expect, it, vi } from 'vitest';

const chats = vi.hoisted(() => ({ activate: vi.fn(), deactivate: vi.fn() }));
vi.mock('@/components/chats/activateChats', () => ({
  activate: () => {
    chats.activate();
    return chats.deactivate;
  },
}));

import { startLabsActivation } from './activation';

async function settle() {
  for (let i = 0; i < 5; i += 1) await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe('labs activation', () => {
  it('loads an experiment only while its flag is on and undoes it when it goes off', async () => {
    const on = ref(false);
    const stop = startLabsActivation(() => on.value);
    await settle();
    expect(chats.activate).not.toHaveBeenCalled();

    on.value = true;
    await settle();
    expect(chats.activate).toHaveBeenCalledTimes(1);

    on.value = false;
    await settle();
    expect(chats.deactivate).toHaveBeenCalledTimes(1);

    on.value = true;
    await settle();
    stop();
    expect(chats.activate).toHaveBeenCalledTimes(2);
    expect(chats.deactivate).toHaveBeenCalledTimes(2);
  });
});
