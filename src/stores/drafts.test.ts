import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendLoadDraft,
  backendDeleteDraft,
  backendListDrafts,
  backendSaveDraft,
  type BackendDraft,
  type BackendDraftInput,
} from '@/services/backend';
import { useDraftsStore } from './drafts';
import { useConnectionStore } from './connection';
import { useMessagesStore } from './messages';
import { useDraftActions } from '@/composables/useDraftActions';

const toast = vi.hoisted(() => ({ info: vi.fn(), success: vi.fn(), error: vi.fn() }));
vi.mock('@/composables/useToast', () => ({ useToast: () => toast }));

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendListDrafts: vi.fn(),
  backendSaveDraft: vi.fn(),
  backendLoadDraft: vi.fn(),
  backendDeleteDraft: vi.fn(),
}));

const listMock = vi.mocked(backendListDrafts);
const saveMock = vi.mocked(backendSaveDraft);
const deleteMock = vi.mocked(backendDeleteDraft);

function draftInput(overrides: Partial<BackendDraftInput> = {}): BackendDraftInput {
  return {
    accountId: 'acc-1',
    to: ['mara@example.com'],
    subject: 'Entwurf',
    body: 'Hallo Mara',
    ...overrides,
  } as BackendDraftInput;
}

function storedDraft(overrides: Partial<BackendDraft> = {}): BackendDraft {
  return {
    id: 'draft-1',
    account_id: 'acc-1',
    thread_id: null,
    to: ['mara@example.com'],
    cc: [],
    bcc: [],
    subject: 'Entwurf',
    body: 'Hallo Mara',
    body_html: null,
    scheduled_for: null,
    attachments: [],
    updated_at: '2026-08-01T10:00:00Z',
    ...overrides,
  } as BackendDraft;
}

describe('drafts store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('inserts a saved draft and upserts one with a known id', async () => {
    saveMock.mockResolvedValueOnce(storedDraft());
    const drafts = useDraftsStore();

    await drafts.save(draftInput());
    expect(drafts.items).toHaveLength(1);

    saveMock.mockResolvedValueOnce(
      storedDraft({ id: 'draft-1', subject: 'Entwurf (bearbeitet)', updated_at: '2026-08-02T10:00:00Z' }),
    );
    await drafts.save(draftInput({ id: 'draft-1' }));
    expect(drafts.items).toHaveLength(1);
    expect(drafts.items[0]?.subject).toBe('Entwurf (bearbeitet)');
  });

  it('records the error and rethrows when saving fails — no silent data loss', async () => {
    saveMock.mockRejectedValueOnce(new Error('disk full'));
    const drafts = useDraftsStore();

    await expect(drafts.save(draftInput())).rejects.toThrow('disk full');
    expect(drafts.lastError).toBe('disk full');
    expect(drafts.items).toHaveLength(0);
  });

  it('reports a failed list to the connection channel and clears it on the next success', async () => {
    listMock.mockResolvedValueOnce([storedDraft()]);
    const drafts = useDraftsStore();
    await drafts.load();

    listMock.mockRejectedValueOnce(new Error('database is locked'));
    await drafts.load();
    // The view reads this to say "could not load" instead of "no drafts".
    expect(useConnectionStore().backendErrorFor('drafts')).toBe('database is locked');
    // A failed list says nothing about the drafts that were already shown.
    expect(drafts.items).toHaveLength(1);

    listMock.mockResolvedValueOnce([]);
    await drafts.load();
    expect(useConnectionStore().backendErrorFor('drafts')).toBeNull();
  });

  it('sorts newest first and only lists future sends as scheduled', async () => {
    listMock.mockResolvedValue([
      storedDraft({ id: 'd-old', updated_at: '2026-08-01T10:00:00Z' }),
      storedDraft({ id: 'd-new', updated_at: '2026-08-03T10:00:00Z' }),
      storedDraft({
        id: 'd-scheduled',
        updated_at: '2026-08-02T10:00:00Z',
        scheduled_for: '2099-01-01T09:00:00Z',
      }),
      storedDraft({
        id: 'd-sent-already',
        updated_at: '2026-08-02T11:00:00Z',
        scheduled_for: '2001-01-01T09:00:00Z',
      }),
    ]);
    const drafts = useDraftsStore();
    await drafts.load();

    expect(drafts.sorted.map((draft) => draft.id)).toEqual([
      'd-new',
      'd-sent-already',
      'd-scheduled',
      'd-old',
    ]);
    expect(drafts.scheduled.map((draft) => draft.id)).toEqual(['d-scheduled']);
  });

  it('removes locally only after the backend confirmed deletion', async () => {
    listMock.mockResolvedValue([storedDraft()]);
    const drafts = useDraftsStore();
    await drafts.load();

    deleteMock.mockRejectedValueOnce(new Error('foreign key constraint'));
    await expect(drafts.remove('draft-1')).rejects.toThrow();
    expect(drafts.items).toHaveLength(1);

    deleteMock.mockResolvedValueOnce(undefined);
    await drafts.remove('draft-1');
    expect(drafts.items).toHaveLength(0);
  });

  it('restores a deleted draft under its original id', async () => {
    const drafts = useDraftsStore();
    const original = storedDraft();

    saveMock.mockResolvedValueOnce(original);
    await drafts.restore(original);

    expect(saveMock).toHaveBeenCalledWith(expect.objectContaining({ id: 'draft-1' }));
    expect(drafts.items.map((draft) => draft.id)).toEqual(['draft-1']);
  });

  /**
   * A thread draft is a message under the draft's id. Without this the card at
   * the end of the thread kept the text and time it was loaded with after the
   * composer had saved newer ones.
   */
  it('brings the draft message in its thread up to the saved text and time', async () => {
    const messages = useMessagesStore();
    messages.messagesByThread = {
      'thread-1': [{ id: 'draft-1', threadId: 'thread-1', isDraft: true, bodyText: 'Hallo', timestamp: '2026-08-01T09:00:00Z' } as never],
    };
    saveMock.mockResolvedValueOnce(storedDraft({ thread_id: 'thread-1', body: 'Hallo Mara, danke!', updated_at: '2026-08-01T10:30:00Z' }));

    await useDraftsStore().save(draftInput({ id: 'draft-1', threadId: 'thread-1' }));

    expect(messages.getByThread('thread-1')[0]).toMatchObject({ bodyText: 'Hallo Mara, danke!', timestamp: '2026-08-01T10:30:00Z' });
  });

  it('keeps mixed availability, rejects unsafe restore/save/load, and still edits valid drafts', async () => {
    const unavailable = storedDraft({ id: 'blocked', body: '', content_error: JSON.stringify({
      key: 'backend.errors.imapUidValidityMissing', params: { folder: 'Work/Drafts' },
    }) });
    listMock.mockResolvedValueOnce([unavailable, storedDraft()]);
    const drafts = useDraftsStore();
    await drafts.load();
    expect(drafts.items).toHaveLength(2);
    await expect(drafts.restore(unavailable)).rejects.toThrow('Work/Drafts');
    await expect(drafts.save(draftInput({ id: 'blocked' }))).rejects.toThrow('Work/Drafts');
    vi.mocked(backendLoadDraft).mockResolvedValueOnce(unavailable);
    await expect(drafts.fetchOne('blocked')).rejects.toThrow('Work/Drafts');
    expect(saveMock).not.toHaveBeenCalled();
    saveMock.mockResolvedValueOnce(storedDraft({ body: 'Edited' }));
    expect((await drafts.save(draftInput({ id: 'draft-1', body: 'Edited' })))?.body).toBe('Edited');
    await drafts.remove('draft-1');
    expect(drafts.items.map((draft) => draft.id)).toEqual(['blocked']);
  });

  it('replaces unavailable list metadata after a verified fresh read so editing can resume', async () => {
    listMock.mockResolvedValueOnce([storedDraft({ content_error: 'backend.errors.imapUidValidityMissing' })]);
    const drafts = useDraftsStore();
    await drafts.load();
    vi.mocked(backendLoadDraft).mockResolvedValueOnce(storedDraft());
    expect((await drafts.fetchOne('draft-1'))?.body).toBe('Hallo Mara');
    expect(drafts.items[0]?.content_error).toBeUndefined();
    saveMock.mockResolvedValueOnce(storedDraft({ body: 'Edited after sync' }));
    expect((await drafts.save(draftInput({ id: 'draft-1', body: 'Edited after sync' })))?.body).toBe('Edited after sync');
  });
  it('edits, deletes and undoes a valid draft while an unavailable row remains in the real store', async () => {
    const blocked = storedDraft({ id: 'blocked', body: '', content_error: 'backend.errors.imapUidValidityMissing' });
    const edited = storedDraft({ body: 'Edited beside an unavailable draft' });
    listMock.mockResolvedValueOnce([blocked, storedDraft()]);
    const drafts = useDraftsStore();
    await drafts.load();
    saveMock.mockResolvedValueOnce(edited);
    await drafts.save(draftInput({ id: 'draft-1', body: edited.body }));
    vi.mocked(backendLoadDraft).mockResolvedValueOnce(edited);
    deleteMock.mockResolvedValueOnce(undefined);
    await expect(useDraftActions().discard('draft-1')).resolves.toBe(true);
    expect(drafts.items.map((draft) => draft.id)).toEqual(['blocked']);
    const action = toast.info.mock.calls.at(-1)?.[1].action;
    expect(action).toBeDefined();
    saveMock.mockResolvedValueOnce(edited);
    action.handler();
    await vi.waitFor(() => expect(drafts.items.find((draft) => draft.id === 'draft-1')?.body).toBe(edited.body));
    expect(saveMock).toHaveBeenLastCalledWith(expect.objectContaining({ id: 'draft-1', body: edited.body }));
    expect(drafts.items.find((draft) => draft.id === 'blocked')?.content_error).toBeTruthy();
  });
});
