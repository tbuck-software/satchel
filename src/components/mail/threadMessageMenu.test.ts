import { describe, expect, it } from 'vitest';
import { inlineMessageMenuIds } from './threadMessageMenu';

describe('inlineMessageMenuIds', () => {
  it('leaves a thread of one without a ⋯ beside the sender', () => {
    expect([...inlineMessageMenuIds([])]).toEqual([]);
    expect([...inlineMessageMenuIds(['m-1'])]).toEqual([]);
  });

  it('keeps a ⋯ beside every sender of a longer thread, the newest included', () => {
    expect([...inlineMessageMenuIds(['m-1', 'm-2', 'm-3'])]).toEqual(['m-1', 'm-2', 'm-3']);
  });
});
