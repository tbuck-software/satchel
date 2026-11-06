import { describe, expect, it } from 'vitest';
import { syncNotices } from './tasks';

const quiet = {
  lists: 1, pulled: 3, pushed: 2, removed: 0, conflicts: 0, restored: 0, failed: 0,
  read_only_dropped: 0, kept_local: 0, lists_gone: 0,
};

describe('sync notices', () => {
  it('says nothing about a sync that only moved changes', () => {
    expect(syncNotices(quiet)).toEqual([]);
  });

  it('names every case where the server overruled or kept local work', () => {
    expect(syncNotices({ ...quiet, conflicts: 2, restored: 1, read_only_dropped: 1, kept_local: 1, lists_gone: 1 })).toEqual([
      '2 tasks were updated on the server; their versions replaced your changes.',
      'A task you deleted came back: it had been changed on the server.',
      'A change was not saved: the list is read-only on the server.',
      'A task deleted on the server kept your change and is now in My tasks.',
      'A task list was removed on the server. It stays here, read-only, until you delete it.',
    ]);
  });
});
