// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useToast } from '@/composables/useToast';
import { useUndoFab } from '@/composables/useUndoFab';

beforeEach(() => {
  setActivePinia(createPinia());
  // Toast state is module-global by design; keep tests hermetic anyway.
  useToast().clear();
});

describe('undo fab derivation', () => {
  it('mirrors only fab-flagged actions and keeps the earliest deadline', () => {
    const toast = useToast();
    const { entries } = useUndoFab();

    toast.success('E-Mail erledigt', {
      groupKey: 'workflow:done',
      count: 1,
      action: {
        label: 'Rückgängig',
        present: 'fab',
        handler: () => {},
        batch: { key: 'workflow:restore', items: [1], handler: () => {} },
      },
      timeout: 9_000,
    });
    toast.error('Kritisch', {}); // not actionable → never in the FAB
    toast.info('Bericht kopieren', {
      action: { label: 'Bericht kopieren', handler: () => {} }, // no flag → stays a toast
      timeout: 0,
    });

    expect(entries.value).toHaveLength(1);
    expect(entries.value[0]?.label).toBe('E-Mail erledigt');
  });

  it('merges grouped sources into one entry with combined count and earliest expiry', async () => {
    vi.useFakeTimers();
    const now = Date.now();
    vi.setSystemTime(now);
    const toast = useToast();
    const { entries } = useUndoFab();

    for (const [index, timeout] of [9_000, 4_000].entries()) {
      toast.success('E-Mail erledigt', {
        groupKey: 'workflow:done',
        count: 2,
        action: {
          label: 'Rückgängig',
          present: 'fab',
          handler: () => {},
          batch: { key: 'workflow:restore', items: [index], handler: () => {} },
        },
        timeout,
      });
    }

    const entry = entries.value[0]!;
    expect(entry.count).toBe(4);
    // Equivalent pushes now merge into one source; the FAB's own grouping
    // still covers actions that arrive from unrelated places.
    expect(entry.ids).toHaveLength(1);
    expect(entry.expiresAt - now).toBe(4_000);

    // Expiry prunes through the toast ticker per source; the mirrored entry
    // lives until its last source lapses, even though the ring tracks the
    // earliest deadline.
    vi.advanceTimersByTime(9_100);
    vi.useRealTimers();
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(entries.value).toHaveLength(0);
  });

  it('runs one batched undo pass for all merged sources', async () => {
    const toast = useToast();
    const batchHandler = vi.fn().mockResolvedValue(undefined);
    const { entries, run } = useUndoFab();

    for (const index of [0, 1]) {
      toast.success('E-Mail erledigt', {
        groupKey: 'workflow:done',
        count: 1,
        action: {
          label: 'Rückgängig',
          present: 'fab',
          handler: () => {},
          batch: { key: 'workflow:restore', items: [`item-${index}`], handler: batchHandler },
        },
        timeout: 9_000,
      });
    }

    await run(entries.value[0]!);

    expect(batchHandler).toHaveBeenCalledExactlyOnceWith(['item-0', 'item-1']);
    expect(toast.state.toasts).toHaveLength(0);
  });
});

it('retains a receipt when its handler refuses the action', async () => {
  const toast = useToast();
  toast.info('Undo', { action: { label: 'Undo', present: 'fab', handler: () => false } });
  const fab = useUndoFab();
  await fab.run(fab.entries.value[0]!);
  expect(toast.state.toasts).toHaveLength(1);
});

it('retains a receipt until an asynchronous handler accepts it', async () => {
  const toast = useToast();
  let accept!: (value: boolean) => void;
  toast.info('Undo', { action: { label: 'Undo', present: 'fab', handler: () => new Promise(resolve => { accept = resolve; }) } });
  const fab = useUndoFab();
  const pending = fab.run(fab.entries.value[0]!);
  expect(toast.state.toasts).toHaveLength(1);
  accept(true);
  await pending;
  expect(toast.state.toasts).toHaveLength(0);
});
