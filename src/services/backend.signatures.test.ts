import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendSaveDraft,
  backendSendMessage,
  backendUpsertSignature,
} from './backend';

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));

describe('signature backend contracts', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({});
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {} });
  });

  it('persists long and short HTML variants together with inline CID assets', async () => {
    await backendUpsertSignature({
      accountId: 'acc-work',
      name: 'Arbeit',
      bodyText: 'Freundliche Grüße\nAlex Morgan',
      bodyHtml: '<p>Freundliche Grüße<br>Alex Morgan</p><img src="cid:brand-logo">',
      shortBodyText: 'Viele Grüße\nAlex',
      shortBodyHtml: '<p>Viele Grüße<br>Alex</p>',
      assets: [
        {
          filename: 'logo.png',
          mimeType: 'image/png',
          contentId: 'brand-logo',
          contentBase64: 'iVBORw0KGgo=',
        },
      ],
      isDefault: true,
    } as Parameters<typeof backendUpsertSignature>[0] & {
      shortBodyText: string;
      shortBodyHtml: string;
      assets: Array<{
        filename: string;
        mimeType: string;
        contentId: string;
        contentBase64: string;
      }>;
    });

    expect(invokeMock).toHaveBeenCalledWith('upsert_signature', {
      input: {
        id: null,
        account_id: 'acc-work',
        name: 'Arbeit',
        body_text: 'Freundliche Grüße\nAlex Morgan',
        body_html: '<p>Freundliche Grüße<br>Alex Morgan</p><img src="cid:brand-logo">',
        short_body_text: 'Viele Grüße\nAlex',
        short_body_html: '<p>Viele Grüße<br>Alex</p>',
        assets: [
          {
            filename: 'logo.png',
            mime_type: 'image/png',
            content_id: 'brand-logo',
            content_base64: 'iVBORw0KGgo=',
          },
        ],
        is_default: true,
      },
    });
  });

  it('forwards automatic signature selection and exact sender identity when sending', async () => {
    await backendSendMessage({
      accountId: 'acc-work',
      to: ['client@example.com'],
      subject: 'Projektstatus',
      body: 'Hallo zusammen',
      threadId: 'thread-project',
      fromAddress: 'consulting@example.com',
      signatureId: 'sig-work',
      signatureVariant: 'auto',
    } as Parameters<typeof backendSendMessage>[0] & {
      signatureId: string;
      signatureVariant: 'auto';
    });

    expect(invokeMock).toHaveBeenCalledWith(
      'send_message',
      expect.objectContaining({
        accountId: 'acc-work',
        input: expect.objectContaining({
          thread_id: 'thread-project',
          from_address: 'consulting@example.com',
          signature_id: 'sig-work',
          signature_variant: 'auto',
        }),
      }),
    );
  });

  it('stores the signature choice on drafts so reopening does not silently change it', async () => {
    await backendSaveDraft({
      accountId: 'acc-work',
      threadId: 'thread-project',
      body: 'Entwurf',
      signatureId: 'sig-work',
      signatureVariant: 'short',
    } as Parameters<typeof backendSaveDraft>[0] & {
      signatureId: string;
      signatureVariant: 'short';
    });

    expect(invokeMock).toHaveBeenCalledWith(
      'save_draft',
      expect.objectContaining({
        input: expect.objectContaining({
          signature_id: 'sig-work',
          signature_variant: 'short',
        }),
      }),
    );
  });
});
