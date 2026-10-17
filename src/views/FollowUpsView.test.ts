// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import FollowUpsView from './FollowUpsView.vue';

const harness = vi.hoisted(() => ({
  replace: vi.fn(),
  setMailboxFilter: vi.fn(),
  threads: {
    workflowFilter: 'all' as string,
  },
}));

vi.mock('vue-router', () => ({
  useRouter: () => ({ replace: harness.replace }),
}));

vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => ({
    get workflowFilter() {
      return harness.threads.workflowFilter;
    },
    set workflowFilter(value: string) {
      harness.threads.workflowFilter = value;
    },
    setMailboxFilter: harness.setMailboxFilter,
  }),
}));

describe('FollowUpsView', () => {
  beforeEach(() => {
    harness.replace.mockReset().mockResolvedValue(undefined);
    harness.setMailboxFilter.mockReset();
    harness.threads.workflowFilter = 'all';
  });

  it('initializes the waiting workflow and returns to the inbox', () => {
    mount(FollowUpsView);

    expect(harness.threads.workflowFilter).toBe('waiting');
    expect(harness.setMailboxFilter).toHaveBeenCalledTimes(1);
    expect(harness.setMailboxFilter).toHaveBeenCalledWith({ kind: 'all', value: null });
    expect(harness.replace).toHaveBeenCalledTimes(1);
    expect(harness.replace).toHaveBeenCalledWith({ name: 'inbox' });
  });
});
