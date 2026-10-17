import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  BackendAlias,
  BackendSignature,
  BackendSignatureAssignment,
  BackendSmartFolder,
  BackendTemplate,
} from '@/services/backend';

const backend = vi.hoisted(() => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendListSignatures: vi.fn(),
  backendListSignatureAssignments: vi.fn(),
  backendListSignatureSuggestions: vi.fn(),
  backendSetSignatureSuggestionStatus: vi.fn(),
  backendDiscoverSignatureSuggestions: vi.fn(),
  backendUpsertSignature: vi.fn(),
  backendUpsertSignatureAssignment: vi.fn(),
  backendDeleteSignature: vi.fn(),
  backendListAliases: vi.fn(),
  backendUpsertAlias: vi.fn(),
  backendDeleteAlias: vi.fn(),
  backendListTemplates: vi.fn(),
  backendRenderTemplate: vi.fn(),
  backendUpsertTemplate: vi.fn(),
  backendDeleteTemplate: vi.fn(),
  backendListSmartFolders: vi.fn(),
  backendUpsertSmartFolder: vi.fn(),
  backendDeleteSmartFolder: vi.fn(),
}));

vi.mock('@/services/backend', () => backend);

import { useAliasesStore } from './aliases';
import { useSignaturesStore, type RemovedSignature } from './signatures';
import { useSmartFoldersStore } from './smartFolders';
import { useTemplatesStore } from './templates';

const signature: BackendSignature = {
  id: 'sig-1',
  account_id: 'acc-1',
  name: 'Work',
  body_text: 'Regards',
  body_html: '<p>Regards</p>',
  short_body_text: 'R',
  short_body_html: null,
  assets: [],
  is_default: 1,
  created_at: '2026-08-01T10:00:00Z',
  updated_at: '2026-08-01T10:00:00Z',
};

const assignment: BackendSignatureAssignment = {
  account_id: 'acc-1',
  identity_email: 'me@example.com',
  signature_id: 'sig-1',
  first_contact_mode: 'full_then_short',
  created_at: '2026-08-01T10:00:00Z',
  updated_at: '2026-08-01T10:00:00Z',
};

const alias: BackendAlias = {
  id: 'alias-1',
  account_id: 'acc-1',
  email: 'sales@example.com',
  display_name: 'Sales',
  is_default: 0,
  created_at: '2026-08-01T10:00:00Z',
  updated_at: '2026-08-01T10:00:00Z',
};

const template: BackendTemplate = {
  id: 'tpl-1',
  account_id: 'acc-1',
  name: 'Offer',
  subject: 'Your offer',
  body_text: 'Hello',
  body_html: '<p>Hello</p>',
  shortcut: 'off',
  cc_json: '["team@example.com"]',
  bcc_json: null,
  attachments_json: null,
  created_at: '2026-08-01T10:00:00Z',
  updated_at: '2026-08-01T10:00:00Z',
};

const smartFolder: BackendSmartFolder = {
  id: 'sf-1',
  name: 'Invoices',
  icon: '📄',
  query: 'has:attachment',
  sort_order: 3,
  created_at: '2026-08-01T10:00:00Z',
  updated_at: '2026-08-01T10:00:00Z',
};

/**
 * One case per settings store. `remove` hands the snapshot back, `restore`
 * takes it — the four stores differ only in which command they call and how
 * their row maps onto the upsert input.
 */
interface StoreCase {
  label: string;
  entryId: string;
  deleteMock: ReturnType<typeof vi.fn>;
  upsertMock: ReturnType<typeof vi.fn>;
  /** Seeds the store with the one entry and returns its handles. */
  open: () => {
    ids: () => string[];
    remove: (id: string) => Promise<unknown>;
    restore: (snapshot: unknown) => Promise<unknown>;
  };
  /** The fields the restored upsert has to carry over. */
  expectedUpsert: Record<string, unknown>;
}

const cases: StoreCase[] = [
  {
    label: 'signatures',
    entryId: signature.id,
    deleteMock: backend.backendDeleteSignature,
    upsertMock: backend.backendUpsertSignature,
    open: () => {
      const store = useSignaturesStore();
      store.items = [signature];
      store.assignments = [assignment];
      return {
        ids: () => store.items.map((item) => item.id),
        remove: (id) => store.remove(id),
        restore: (snapshot) => store.restore(snapshot as RemovedSignature),
      };
    },
    expectedUpsert: {
      id: 'sig-1',
      accountId: 'acc-1',
      name: 'Work',
      bodyText: 'Regards',
      bodyHtml: '<p>Regards</p>',
      shortBodyText: 'R',
      shortBodyHtml: null,
      assets: [],
      isDefault: true,
    },
  },
  {
    label: 'aliases',
    entryId: alias.id,
    deleteMock: backend.backendDeleteAlias,
    upsertMock: backend.backendUpsertAlias,
    open: () => {
      const store = useAliasesStore();
      store.items = [alias];
      return {
        ids: () => store.items.map((item) => item.id),
        remove: (id) => store.remove(id),
        restore: (snapshot) => store.restore(snapshot as BackendAlias),
      };
    },
    expectedUpsert: {
      id: 'alias-1',
      accountId: 'acc-1',
      email: 'sales@example.com',
      displayName: 'Sales',
      isDefault: false,
    },
  },
  {
    label: 'templates',
    entryId: template.id,
    deleteMock: backend.backendDeleteTemplate,
    upsertMock: backend.backendUpsertTemplate,
    open: () => {
      const store = useTemplatesStore();
      store.items = [template];
      return {
        ids: () => store.items.map((item) => item.id),
        remove: (id) => store.remove(id),
        restore: (snapshot) => store.restore(snapshot as BackendTemplate),
      };
    },
    expectedUpsert: {
      id: 'tpl-1',
      accountId: 'acc-1',
      name: 'Offer',
      subject: 'Your offer',
      bodyText: 'Hello',
      bodyHtml: '<p>Hello</p>',
      shortcut: 'off',
      cc: ['team@example.com'],
      bcc: [],
      attachmentsJson: null,
    },
  },
  {
    label: 'smart folders',
    entryId: smartFolder.id,
    deleteMock: backend.backendDeleteSmartFolder,
    upsertMock: backend.backendUpsertSmartFolder,
    open: () => {
      const store = useSmartFoldersStore();
      store.items = [smartFolder];
      return {
        ids: () => store.items.map((item) => item.id),
        remove: (id) => store.remove(id),
        restore: (snapshot) => store.restore(snapshot as BackendSmartFolder),
      };
    },
    expectedUpsert: {
      id: 'sf-1',
      name: 'Invoices',
      icon: '📄',
      query: 'has:attachment',
      sortOrder: 3,
    },
  },
];

describe.each(cases)('$label store deletion', (testCase) => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    backend.backendSupportsNativeCommands.mockReturnValue(true);
    testCase.deleteMock.mockResolvedValue(undefined);
    backend.backendUpsertSignature.mockResolvedValue(signature);
    backend.backendUpsertSignatureAssignment.mockResolvedValue(assignment);
    backend.backendUpsertAlias.mockResolvedValue(alias);
    backend.backendUpsertTemplate.mockResolvedValue(template);
    backend.backendUpsertSmartFolder.mockResolvedValue(smartFolder);
  });

  it('drops the entry and hands back a snapshot', async () => {
    const store = testCase.open();

    const snapshot = await store.remove(testCase.entryId);

    expect(testCase.deleteMock).toHaveBeenCalledWith(testCase.entryId);
    expect(store.ids()).toEqual([]);
    expect(snapshot).toBeTruthy();
  });

  it('leaves the list untouched when the backend refuses', async () => {
    testCase.deleteMock.mockRejectedValue(new Error('nope'));
    const store = testCase.open();

    await expect(store.remove(testCase.entryId)).rejects.toThrow('nope');

    expect(store.ids()).toEqual([testCase.entryId]);
  });

  it('restores the snapshot under its own id', async () => {
    const store = testCase.open();
    const snapshot = await store.remove(testCase.entryId);

    await store.restore(snapshot);

    expect(testCase.upsertMock).toHaveBeenCalledWith(
      expect.objectContaining(testCase.expectedUpsert),
    );
    expect(store.ids()).toEqual([testCase.entryId]);
  });
});

describe('signature assignments', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    backend.backendDeleteSignature.mockResolvedValue(undefined);
    backend.backendUpsertSignature.mockResolvedValue(signature);
    backend.backendUpsertSignatureAssignment.mockResolvedValue(assignment);
  });

  it('mirrors the identities the delete detached, and reattaches them on undo', async () => {
    const store = useSignaturesStore();
    store.items = [signature];
    store.assignments = [assignment];

    const snapshot = await store.remove(signature.id);
    expect(store.assignments[0]!.signature_id).toBeNull();

    await store.restore(snapshot!);

    expect(backend.backendUpsertSignatureAssignment).toHaveBeenCalledWith({
      accountId: 'acc-1',
      identityEmail: 'me@example.com',
      signatureId: 'sig-1',
      firstContactMode: 'full_then_short',
    });
    expect(store.assignments[0]!.signature_id).toBe('sig-1');
  });
});
