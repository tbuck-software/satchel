// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Component } from 'vue';

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

const accountsStub = vi.hoisted(() => ({
  accounts: [{ id: 'acc-1', email: 'me@example.com', displayName: 'Me' }],
  activeAccountId: 'acc-1',
  load: vi.fn(async () => {}),
}));

vi.mock('@/services/backend', () => backend);
vi.mock('@/stores/accounts', () => ({ useAccountsStore: () => accountsStub }));

import AliasEditor from './AliasEditor.vue';
import SignaturesEditor from './SignaturesEditor.vue';
import SmartFoldersEditor from './SmartFoldersEditor.vue';
import TemplatesEditor from './TemplatesEditor.vue';
import { useToast } from '@/composables/useToast';

const signature = {
  id: 'sig-1',
  account_id: 'acc-1',
  name: 'Work signature',
  body_text: 'Regards',
  body_html: null,
  short_body_text: '',
  short_body_html: null,
  assets: [],
  is_default: 1,
  created_at: '2026-08-01T10:00:00Z',
  updated_at: '2026-08-01T10:00:00Z',
};

const alias = {
  id: 'alias-1',
  account_id: 'acc-1',
  email: 'sales@example.com',
  display_name: 'Sales',
  is_default: 0,
  created_at: '2026-08-01T10:00:00Z',
  updated_at: '2026-08-01T10:00:00Z',
};

const template = {
  id: 'tpl-1',
  account_id: 'acc-1',
  name: 'Offer',
  subject: null,
  body_text: 'Hello',
  body_html: null,
  shortcut: null,
  cc_json: null,
  bcc_json: null,
  attachments_json: null,
  created_at: '2026-08-01T10:00:00Z',
  updated_at: '2026-08-01T10:00:00Z',
};

const smartFolder = {
  id: 'sf-1',
  name: 'Invoices',
  icon: null,
  query: 'has:attachment',
  sort_order: 0,
  created_at: '2026-08-01T10:00:00Z',
  updated_at: '2026-08-01T10:00:00Z',
};

/** The four settings editors that delete a user-created entry. */
const cases = [
  {
    label: 'SignaturesEditor',
    component: SignaturesEditor as Component,
    entryId: signature.id,
    entryName: signature.name,
    seed: () => backend.backendListSignatures.mockResolvedValue([signature]),
    deleteMock: backend.backendDeleteSignature,
    upsertMock: backend.backendUpsertSignature,
  },
  {
    label: 'AliasEditor',
    component: AliasEditor as Component,
    entryId: alias.id,
    entryName: alias.email,
    seed: () => backend.backendListAliases.mockResolvedValue([alias]),
    deleteMock: backend.backendDeleteAlias,
    upsertMock: backend.backendUpsertAlias,
  },
  {
    label: 'TemplatesEditor',
    component: TemplatesEditor as Component,
    entryId: template.id,
    entryName: template.name,
    seed: () => backend.backendListTemplates.mockResolvedValue([template]),
    deleteMock: backend.backendDeleteTemplate,
    upsertMock: backend.backendUpsertTemplate,
  },
  {
    label: 'SmartFoldersEditor',
    component: SmartFoldersEditor as Component,
    entryId: smartFolder.id,
    entryName: smartFolder.name,
    seed: () => backend.backendListSmartFolders.mockResolvedValue([smartFolder]),
    deleteMock: backend.backendDeleteSmartFolder,
    upsertMock: backend.backendUpsertSmartFolder,
  },
];

const stubs = {
  RichTextEditor: true,
  IsolatedEmailDocument: true,
  AttachmentDropZone: true,
};

const toast = useToast();

/** Reads a toast the assertion expects to be there. */
function toastAt(index: number) {
  const entry = toast.state.toasts.at(index);
  if (!entry) throw new Error(`no toast at ${index}`);
  return entry;
}

describe.each(cases)('$label deletion', (testCase) => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    toast.clear();
    backend.backendSupportsNativeCommands.mockReturnValue(true);
    backend.backendListSignatures.mockResolvedValue([]);
    backend.backendListSignatureAssignments.mockResolvedValue([]);
    backend.backendListSignatureSuggestions.mockResolvedValue([]);
    backend.backendDiscoverSignatureSuggestions.mockResolvedValue([]);
    backend.backendListAliases.mockResolvedValue([]);
    backend.backendListTemplates.mockResolvedValue([]);
    backend.backendListSmartFolders.mockResolvedValue([]);
    backend.backendUpsertSignature.mockResolvedValue(signature);
    backend.backendUpsertAlias.mockResolvedValue(alias);
    backend.backendUpsertTemplate.mockResolvedValue(template);
    backend.backendUpsertSmartFolder.mockResolvedValue(smartFolder);
    testCase.deleteMock.mockResolvedValue(undefined);
    testCase.seed();
  });

  async function mountWithEntry() {
    const wrapper = mount(testCase.component, { global: { stubs } });
    await vi.waitFor(() => {
      expect(wrapper.findAll('.list .delete')).toHaveLength(1);
    });
    return wrapper;
  }

  it('removes the row and offers undo', async () => {
    const wrapper = await mountWithEntry();

    await wrapper.get('.list .delete').trigger('click');
    await vi.waitFor(() => {
      expect(wrapper.findAll('.list .delete')).toHaveLength(0);
      expect(toast.state.toasts).toHaveLength(1);
    });

    expect(testCase.deleteMock).toHaveBeenCalledWith(testCase.entryId);
    expect(toast.state.toasts).toHaveLength(1);
    expect(toastAt(0).message).toContain(testCase.entryName);
    expect(toastAt(0).action?.label).toBe('Undo');
  });

  it('puts the entry back through the store upsert when undo is used', async () => {
    const wrapper = await mountWithEntry();
    await wrapper.get('.list .delete').trigger('click');
    await vi.waitFor(() => expect(toast.state.toasts[0]?.action).toBeDefined());

    await toastAt(0).action?.handler();
    await vi.waitFor(() => {
      expect(wrapper.findAll('.list .delete')).toHaveLength(1);
    });

    expect(testCase.upsertMock).toHaveBeenCalledWith(
      expect.objectContaining({ id: testCase.entryId }),
    );
  });

  it('keeps the row and reports the cause when the backend refuses', async () => {
    testCase.deleteMock.mockRejectedValue(new Error('database is locked'));
    const wrapper = await mountWithEntry();

    await wrapper.get('.list .delete').trigger('click');
    await vi.waitFor(() => {
      expect(toast.state.toasts).toHaveLength(1);
    });

    expect(wrapper.findAll('.list .delete')).toHaveLength(1);
    expect(toastAt(0).tone).toBe('error');
    expect(toastAt(0).message).toContain('database is locked');
  });
});
